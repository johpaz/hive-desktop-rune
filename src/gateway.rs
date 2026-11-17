//! Ciclo de vida del sidecar `hive-gateway-rust`: puerto, arranque, /health, reinicio y cierre.
//! Port de `apps/hive-desktop/src-tauri/src/main.rs:27-327`, sin Tauri ni tokio.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

/// Puerto local habitual de Hive.
pub const DEFAULT_PORT: u16 = 18790;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Starting,
    Ready { port: u16, external: bool },
    Failed(String),
}

pub struct Gateway {
    child: Arc<Mutex<Option<Child>>>,
    shutting_down: Arc<AtomicBool>,
    external: bool,
    pub port: u16,
}

/// `$HIVE_HOME` o `~/.hive`, el mismo directorio que usa la CLI.
pub fn resolve_hive_home() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("HIVE_HOME") {
        return Ok(PathBuf::from(explicit));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".hive"))
        .ok_or_else(|| "No se pudo resolver el directorio del usuario".into())
}

/// Busca el sidecar: `HIVE_GATEWAY_BIN`, junto al ejecutable (instalado) o el del repo (desarrollo).
fn sidecar_path() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("HIVE_GATEWAY_BIN") {
        return Ok(PathBuf::from(explicit));
    }
    let name = if cfg!(windows) {
        "hive-gateway-rust.exe"
    } else {
        "hive-gateway-rust"
    };
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
        }
    }
    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hiveSDK-Rust/target");
    candidates.push(sdk.join("debug").join(name));
    candidates.push(sdk.join("release").join(name));
    candidates.into_iter().find(|path| path.is_file()).ok_or_else(||
        "No se encontró hive-gateway-rust. Compila: cargo build --manifest-path ../hiveSDK-Rust/Cargo.toml -p hive-gateway-rust (o usa HIVE_GATEWAY_BIN)".into())
}

fn spawn_sidecar(port: u16, hive_home: &PathBuf) -> Result<Child, String> {
    let mut cmd = Command::new(sidecar_path()?);
    cmd.args(["start", "--skip-check"])
        .env("HIVE_HOME", hive_home)
        .env("HIVE_HOST", "127.0.0.1")
        .env("HIVE_PORT", port.to_string())
        .env("HIVE_GATEWAY_CHILD", "1")
        .env("NO_BROWSER", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Que el gateway muera con la app en cualquier salida (cerrar durante el arranque, un fallo, SIGKILL):
    // sin esto quedaba huérfano ocupando el puerto. Linux liga la señal al HILO que lo lanzó, así que ese
    // hilo debe seguir vivo mientras viva la app (ver `keep_thread_alive`).
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: entre `fork` y `exec` solo se llama a `prctl` y `getppid`, que son async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|error| format!("No se pudo iniciar el gateway: {error}"))?;
    if let Some(out) = child.stdout.take() {
        pipe_to_log(out, false);
    }
    if let Some(err) = child.stderr.take() {
        pipe_to_log(err, true);
    }
    Ok(child)
}

fn pipe_to_log(stream: impl Read + Send + 'static, is_err: bool) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if is_err {
                eprintln!("[Hive Gateway] {line}");
            } else {
                println!("[Hive Gateway] {line}");
            }
        }
    });
}

/// Mantiene vivo el hilo actual hasta que acabe el proceso. Con `PR_SET_PDEATHSIG` el gateway recibe la
/// señal cuando muere el hilo que lo lanzó, no solo el proceso: un hilo que termina lo mataría.
pub fn keep_thread_alive() -> ! {
    loop {
        thread::park();
    }
}

pub fn is_healthy(port: u16) -> bool {
    let Some(address) = ("127.0.0.1", port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
    else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(250)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ =
        stream.write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response_is_healthy(&response)
}

fn response_is_healthy(response: &str) -> bool {
    (response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"))
        && response.contains("\"status\":\"ok\"")
}

pub fn metadata(port: u16) -> Option<serde_json::Value> {
    ureq::get(&format!("http://127.0.0.1:{port}/health"))
        .timeout(Duration::from_millis(500))
        .call()
        .ok()?
        .into_json()
        .ok()
}

fn compatible(port: u16, home: &std::path::Path) -> bool {
    let Some(value) = metadata(port) else {
        return false;
    };
    let Ok(home) = std::fs::canonicalize(home) else {
        return false;
    };
    let Ok(database) = std::fs::canonicalize(
        std::env::var_os("HIVE_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("data/hivedb")),
    ) else {
        return false;
    };
    metadata_matches(&value, &home, &database)
}

fn metadata_matches(
    value: &serde_json::Value,
    home: &std::path::Path,
    database: &std::path::Path,
) -> bool {
    value["desktopContract"] == 4
        && value["status"] == "ok"
        && value["backend"] == "hive-gateway-rust"
        && value["protocol"] == 2
        && value["home"].as_str().map(PathBuf::from).as_deref() == Some(home)
        && value["dbPath"].as_str().map(PathBuf::from).as_deref() == Some(database)
        && value["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|c| c == "chat"))
}

fn wait_for(
    port: u16,
    home: &std::path::Path,
    child: &Arc<Mutex<Option<Child>>>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child
            .lock()
            .map_err(|_| "No se pudo consultar el gateway")?
            .as_mut()
            .and_then(|c| c.try_wait().ok().flatten())
        {
            return Err(format!("hive-gateway-rust terminó durante el arranque ({status}); revisa el diagnóstico del gateway"));
        }
        if compatible(port, home) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(format!("El gateway Rust no respondió en el puerto {port}"))
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("No se pudo reservar un puerto local: {e}"))
}

impl Gateway {
    /// Arranca (o se conecta a) el gateway y espera `/health`. Bloquea hasta 30 s.
    pub fn start() -> Result<Gateway, String> {
        let hive_home = resolve_hive_home()?;
        std::fs::create_dir_all(&hive_home)
            .map_err(|e| format!("No se pudo crear HIVE_HOME: {e}"))?;
        let shutting_down = Arc::new(AtomicBool::new(false));

        // Ya hay un Hive sano (p. ej. `hive start`): solo somos su ventana; nunca lo matamos.
        if compatible(DEFAULT_PORT, &hive_home) {
            return Ok(Gateway {
                child: Arc::new(Mutex::new(None)),
                shutting_down,
                external: true,
                port: DEFAULT_PORT,
            });
        }
        let port = if TcpListener::bind(("127.0.0.1", DEFAULT_PORT)).is_ok() {
            DEFAULT_PORT
        } else {
            free_port()?
        };
        let child = Arc::new(Mutex::new(Some(spawn_sidecar(port, &hive_home)?)));
        if let Err(error) = wait_for(port, &hive_home, &child, Duration::from_secs(30)) {
            if let Some(mut process) = child.lock().ok().and_then(|mut c| c.take()) {
                let _ = process.kill();
                let _ = process.wait();
            }
            return Err(error);
        }
        Self::watch(port, hive_home, child.clone(), shutting_down.clone());
        Ok(Gateway {
            child,
            shutting_down,
            external: false,
            port,
        })
    }

    /// Reinicia el gateway si deja de contestar a /health (el servidor real corre como nieto
    /// del envoltorio, así que la salida del proceso hijo no basta).
    fn watch(port: u16, home: PathBuf, child: Arc<Mutex<Option<Child>>>, down: Arc<AtomicBool>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(5));
            if down.load(Ordering::SeqCst) {
                break;
            }
            if compatible(port, &home) {
                continue;
            }
            thread::sleep(Duration::from_secs(3));
            if down.load(Ordering::SeqCst) || compatible(port, &home) {
                continue;
            }
            eprintln!("[Hive Agents] el gateway dejó de responder — reiniciándolo");
            if let Some(mut previous) = child.lock().ok().and_then(|mut c| c.take()) {
                let _ = previous.kill();
                let _ = previous.wait();
            }
            let next = {
                let Ok(mut slot) = child.lock() else { break };
                if down.load(Ordering::SeqCst) {
                    break;
                }
                spawn_sidecar(port, &home).map(|next| *slot = Some(next))
            };
            match next {
                Ok(()) => {
                    if wait_for(port, &home, &child, Duration::from_secs(30)).is_err() {
                        eprintln!("[Hive Agents] el gateway reiniciado no respondió a tiempo");
                    }
                }
                Err(e) => eprintln!("[Hive Agents] no se pudo reiniciar el gateway: {e}"),
            }
        });
    }

    pub fn is_external(&self) -> bool {
        self.external
    }

    pub fn stop(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        if self.external {
            return;
        }
        if let Some(mut child) = self.child.lock().ok().and_then(|mut c| c.take()) {
            if let Err(e) = child.kill() {
                eprintln!("[Hive Agents] no se pudo detener el gateway: {e}");
            }
            let _ = child.wait();
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_requires_rust_protocol_profile_database_and_chat_capability() {
        let home = std::path::Path::new("/test/profile");
        let database = home.join("data/hivedb");
        let health = serde_json::json!({"status":"ok","backend":"hive-gateway-rust","protocol":2,"desktopContract":4,
            "home":home,"dbPath":database,"capabilities":["chat"]});
        assert!(metadata_matches(&health, home, &database));
        for (key, value) in [
            ("desktopContract", serde_json::json!(2)),
            ("backend", serde_json::json!("typescript")),
            ("protocol", serde_json::json!(1)),
            ("home", serde_json::json!("/other")),
            ("dbPath", serde_json::json!("/other/db")),
            ("capabilities", serde_json::json!([])),
        ] {
            let mut wrong = health.clone();
            wrong[key] = value;
            assert!(!metadata_matches(&wrong, home, &database), "{key}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn exited_sidecar_reports_failure_without_waiting_for_startup_timeout() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let child = Arc::new(Mutex::new(Some(child)));
        let start = Instant::now();
        let error = wait_for(
            0,
            std::path::Path::new("/nonexistent"),
            &child,
            Duration::from_secs(30),
        )
        .unwrap_err();
        assert!(error.contains("terminó durante el arranque"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn health_requires_200_and_ok_status() {
        assert!(response_is_healthy(
            "HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\"}"
        ));
        assert!(!response_is_healthy(
            "HTTP/1.1 500 X\r\n\r\n{\"status\":\"ok\"}"
        ));
        assert!(!response_is_healthy(
            "HTTP/1.1 200 OK\r\n\r\n{\"status\":\"down\"}"
        ));
    }

    #[test]
    fn free_port_is_bindable() {
        assert!(free_port().unwrap() > 0);
    }

    /// Prueban el cierre con la app contra el gateway real (`HIVE_HOME` temporal, puerto 18790):
    /// `cargo test -- --ignored --test-threads=1 gateway_`
    /// El gateway del SDK acompaña a un perfil Hive que ya existe: no crea uno desde cero. Estas pruebas parten de
    /// una copia aislada (`HIVE_RUNE_PROFILE`, preparada con `hive-gateway-rust --prepare-test-home`).
    fn scratch_home(tag: &str) {
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let (src, dst) = (entry.path(), to.join(entry.file_name()));
                if entry.file_type().unwrap().is_dir() {
                    copy(&src, &dst);
                } else {
                    std::fs::copy(&src, &dst).unwrap();
                }
            }
        }
        let profile =
            std::env::var("HIVE_RUNE_PROFILE").expect("HIVE_RUNE_PROFILE: copia de un perfil Hive");
        let dir = std::env::temp_dir().join(format!("hive-rune-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy(std::path::Path::new(&profile), &dir);
        std::env::set_var("HIVE_HOME", dir);
    }

    fn wait_until(secs: u64, cond: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        cond()
    }

    #[test]
    #[ignore]
    fn gateway_lives_while_its_launching_thread_is_kept_alive() {
        scratch_home("alive");
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let gw = Gateway::start().expect("arranca");
            tx.send((gw.port, gw)).unwrap();
            keep_thread_alive();
        });
        let (port, gw) = rx
            .recv_timeout(Duration::from_secs(45))
            .expect("el gateway arrancó");
        thread::sleep(Duration::from_secs(4));
        assert!(is_healthy(port), "sigue sano 4 s después, con su hilo vivo");
        gw.stop();
        assert!(wait_until(5, || !is_healthy(port)), "stop() lo detiene");
    }

    #[test]
    #[ignore]
    fn gateway_dies_if_the_thread_that_launched_it_ends() {
        // Por eso `keep_thread_alive`: Linux liga PDEATHSIG al hilo, no al proceso.
        scratch_home("dies");
        let port = thread::spawn(|| {
            let gw = Gateway::start().expect("arranca");
            let port = gw.port;
            std::mem::forget(gw); // Comprueba PDEATHSIG, sin pasar por Drop::stop.
            port
        })
        .join()
        .unwrap();
        assert!(
            wait_until(8, || !is_healthy(port)),
            "el gateway cayó al terminar el hilo que lo lanzó"
        );
    }
}
