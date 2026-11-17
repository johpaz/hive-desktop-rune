//! Imágenes de A2UI: descarga, decodifica y entrega a Rune como `Pixels`.
//!
//! La URL la decide el agente, así que se trata como no confiable: solo http/https, sin seguir
//! redirecciones, con tope de tamaño y de tiempo, y sin tocar direcciones locales o privadas
//! (salvo el propio gateway, de donde salen las imágenes del chat, y solo a él se le manda el token).

use std::{
    collections::HashMap,
    io::Read,
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use rune::Pixels;
use url::{Host, Url};

pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_DIM: u32 = 4096;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone)]
pub struct Slot {
    pub pixels: Pixels,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub base: String,
    pub token: Option<String>,
}

#[derive(Default)]
struct Inner {
    origin: Option<Origin>,
    slots: HashMap<String, Slot>,
}

/// Caché de imágenes por URL. Clonarla comparte el estado; las descargas corren en hilos.
#[derive(Clone, Default)]
pub struct Images {
    inner: Arc<Mutex<Inner>>,
    dirty: Arc<AtomicBool>,
}

impl Images {
    /// El gateway al que pertenecen las rutas relativas y el único que recibe el token.
    pub fn set_origin(&self, base: &str, token: Option<&str>) {
        self.inner.lock().unwrap().origin = Some(Origin {
            base: base.to_string(),
            token: token.map(str::to_string),
        });
    }

    /// Estado y píxeles de `url`; la primera vez que se pide, arranca la descarga.
    pub fn get(&self, url: &str) -> Slot {
        let mut g = self.inner.lock().unwrap();
        if let Some(s) = g.slots.get(url) {
            return s.clone();
        }
        let slot = Slot {
            pixels: Pixels::new(),
            status: Status::Loading,
        };
        g.slots.insert(url.to_string(), slot.clone());
        let origin = g.origin.clone();
        drop(g);
        let (this, u) = (self.clone(), url.to_string());
        thread::spawn(move || {
            let result = load(&u, origin.as_ref());
            this.finish(&u, result);
        });
        slot
    }

    fn finish(&self, url: &str, result: Result<(u32, u32, Vec<u8>), String>) {
        let mut g = self.inner.lock().unwrap();
        if let Some(slot) = g.slots.get_mut(url) {
            match result {
                Ok((w, h, rgba)) => {
                    slot.pixels.set(w, h, rgba);
                    slot.status = Status::Ready;
                }
                Err(e) => slot.status = Status::Failed(e),
            }
        }
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// ¿Terminó alguna descarga desde la última vez? La vista debe reconstruirse (el tamaño cambia).
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }
}

fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
        }
        IpAddr::V6(v) => {
            v.is_loopback()
                || v.is_unspecified()
                || (v.segments()[0] & 0xfe00) == 0xfc00
                || (v.segments()[0] & 0xffc0) == 0xfe80
                || v.to_ipv4_mapped()
                    .is_some_and(|m| blocked_ip(IpAddr::V4(m)))
        }
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Valida `raw` y devuelve la URL absoluta y si se le puede mandar el token (solo al gateway).
pub fn resolve(raw: &str, origin: Option<&Origin>) -> Result<(String, bool), String> {
    let base = origin.and_then(|o| Url::parse(&o.base).ok());
    let url = if raw.starts_with('/') && !raw.starts_with("//") {
        base.as_ref()
            .ok_or("ruta relativa sin gateway")?
            .join(raw)
            .map_err(|e| e.to_string())?
    } else {
        Url::parse(raw).map_err(|_| "URL inválida".to_string())?
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("esquema no permitido: {}", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL con credenciales".into());
    }
    let is_gateway = base.as_ref().is_some_and(|b| same_origin(b, &url));
    if !is_gateway {
        let local = match url.host() {
            Some(Host::Domain(d)) => {
                let d = d.to_ascii_lowercase();
                d == "localhost"
                    || d.ends_with(".localhost")
                    || d.ends_with(".local")
                    || d.ends_with(".internal")
            }
            Some(Host::Ipv4(ip)) => blocked_ip(IpAddr::V4(ip)),
            Some(Host::Ipv6(ip)) => blocked_ip(IpAddr::V6(ip)),
            None => true,
        };
        if local {
            return Err("dirección local o privada no permitida".into());
        }
    }
    Ok((url.to_string(), is_gateway))
}

/// Enlace que se puede abrir fuera de la app (vídeo): solo http/https públicos. Nunca el gateway ni rutas relativas,
/// porque el navegador no llevaría el token. Devuelve la URL y el host que se enseña a la persona antes del clic.
pub fn external_link(raw: &str) -> Result<(String, String), String> {
    let (full, _) = resolve(raw, None)?;
    let host = Url::parse(&full)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .ok_or("URL sin host")?;
    Ok((full, host))
}

/// Abre `url` con el navegador del sistema. Solo se llama tras un clic y con una URL que pasó `external_link`.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("rundll32", vec!["url.dll,FileProtocolHandler", url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("No se pudo abrir el navegador: {e}"))
}

/// Decodifica a RGBA8. Rechaza imágenes enormes antes de reservar memoria para los píxeles.
pub fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| "no es una imagen reconocible".to_string())?;
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        return Err(format!("imagen demasiado grande ({w}×{h})"));
    }
    let img =
        image::load_from_memory(bytes).map_err(|_| "no es una imagen reconocible".to_string())?;
    Ok((w, h, img.to_rgba8().into_raw()))
}

/// Descarga `url` con la política de seguridad (esquema, dirección, sin redirecciones, tope de
/// tamaño y de tiempo). Compartida por imágenes y audio.
pub fn fetch(url: &str, origin: Option<&Origin>, max_bytes: u64) -> Result<Vec<u8>, String> {
    let (full, send_token) = resolve(url, origin)?;
    // Sin redirecciones: un 302 podría apuntar a una dirección que la política no dejaría pasar.
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .redirects(0)
        .build();
    let mut req = agent.get(&full);
    if let (true, Some(t)) = (send_token, origin.and_then(|o| o.token.as_ref())) {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let resp = req
        .call()
        .map_err(|e| format!("no se pudo descargar: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("lectura fallida: {e}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err("archivo demasiado pesado".into());
    }
    Ok(bytes)
}

fn load(url: &str, origin: Option<&Origin>) -> Result<(u32, u32, Vec<u8>), String> {
    decode(&fetch(url, origin, MAX_BYTES)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn video_links_are_public_http_only() {
        let (url, host) = external_link("https://cdn.example.com/v/clip.mp4").unwrap();
        assert_eq!(
            (url.as_str(), host.as_str()),
            ("https://cdn.example.com/v/clip.mp4", "cdn.example.com")
        );
        for bad in [
            "/media/clip.mp4",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://127.0.0.1:18790/x.mp4",
            "http://192.168.1.4/x.mp4",
            "http://localhost/x",
            "https://user:pw@example.com/x",
            "//evil.com/x",
            "no es url",
        ] {
            assert!(external_link(bad).is_err(), "debía rechazar {bad}");
        }
    }
    use super::*;
    use std::io::Write;

    fn gw(base: &str) -> Origin {
        Origin {
            base: base.into(),
            token: Some("tok".into()),
        }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            w,
            h,
            image::Rgba([200, 30, 30, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
        out
    }

    #[test]
    fn public_http_and_https_are_allowed_without_the_token() {
        assert_eq!(
            resolve("https://example.com/a.png", None),
            Ok(("https://example.com/a.png".to_string(), false))
        );
        assert!(resolve(
            "http://example.com/a.png",
            Some(&gw("http://127.0.0.1:18790"))
        )
        .is_ok());
    }

    #[test]
    fn relative_paths_resolve_against_the_gateway_and_carry_the_token() {
        let o = gw("http://127.0.0.1:18790");
        assert_eq!(
            resolve("/api/artifacts/x/download", Some(&o)),
            Ok((
                "http://127.0.0.1:18790/api/artifacts/x/download".to_string(),
                true
            ))
        );
        assert!(
            resolve("/api/x", None).is_err(),
            "sin gateway no hay a qué resolverla"
        );
        assert!(
            resolve("//evil.test/x.png", Some(&o)).is_err(),
            "una URL sin esquema se rechaza: nunca hereda el token del gateway"
        );
    }

    #[test]
    fn local_and_private_addresses_are_blocked() {
        let o = gw("http://127.0.0.1:18790");
        for bad in [
            "http://localhost/x.png",
            "http://127.0.0.1:5500/x.png",
            "http://10.0.0.5/x.png",
            "http://192.168.1.1/x.png",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/x.png",
            "http://[::ffff:127.0.0.1]/x.png",
            "http://printer.local/x.png",
            "http://api.internal/x.png",
            "http://0.0.0.0/x.png",
        ] {
            assert!(resolve(bad, Some(&o)).is_err(), "{bad} debería bloquearse");
        }
    }

    #[test]
    fn only_the_gateways_own_origin_may_be_local() {
        let o = gw("http://127.0.0.1:18790");
        assert_eq!(
            resolve("http://127.0.0.1:18790/api/artifacts/1/download", Some(&o)).map(|(_, t)| t),
            Ok(true)
        );
        assert!(
            resolve("http://127.0.0.1:18791/x", Some(&o)).is_err(),
            "otro puerto local no es el gateway"
        );
    }

    #[test]
    fn odd_schemes_and_credentials_are_rejected() {
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/a.png",
            "javascript:alert(1)",
            "https://user:pw@example.com/a.png",
            "no es url",
        ] {
            assert!(resolve(bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn decode_reads_dimensions_and_rejects_junk_and_huge_images() {
        let (w, h, rgba) = decode(&png(3, 2)).unwrap();
        assert_eq!((w, h, rgba.len()), (3, 2, 3 * 2 * 4));
        assert!(decode(b"esto no es una imagen").is_err());
        assert!(decode(&png(MAX_DIM + 1, 1))
            .unwrap_err()
            .contains("demasiado grande"));
    }

    /// Servidor HTTP de una sola respuesta; devuelve el puerto y lo que recibió.
    fn serve(status: &str, body: Vec<u8>) -> (u16, thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = status.to_string();
        let h = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            write!(
                s,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            s.write_all(&body).unwrap();
            req
        });
        (port, h)
    }

    fn wait(images: &Images, url: &str) -> Slot {
        let end = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let s = images.get(url);
            if s.status != Status::Loading || std::time::Instant::now() > end {
                return s;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_gateways_images_download_with_the_token_and_mark_the_view_dirty() {
        let (port, server) = serve("200 OK", png(4, 3));
        let images = Images::default();
        images.set_origin(&format!("http://127.0.0.1:{port}"), Some("tok"));
        assert_eq!(
            images.get("/api/artifacts/a/download").status,
            Status::Loading
        );
        let slot = wait(&images, "/api/artifacts/a/download");
        assert_eq!(slot.status, Status::Ready);
        assert!(images.take_dirty() && !images.take_dirty(), "avisa una vez");
        let req = server.join().unwrap();
        assert!(req.starts_with("GET /api/artifacts/a/download "), "{req}");
        assert!(
            req.to_ascii_lowercase()
                .contains("authorization: bearer tok"),
            "{req}"
        );
    }

    #[test]
    fn redirects_are_not_followed() {
        let (port, server) = serve("302 Found", vec![]);
        let images = Images::default();
        images.set_origin(&format!("http://127.0.0.1:{port}"), None);
        let slot = wait(&images, "/r");
        assert!(
            matches!(slot.status, Status::Failed(_)),
            "{:?}",
            slot.status
        );
        server.join().unwrap();
    }

    #[test]
    fn a_blocked_url_fails_without_any_request() {
        let images = Images::default();
        let slot = wait(&images, "http://127.0.0.1:9/x.png");
        assert!(
            matches!(&slot.status, Status::Failed(m) if m.contains("no permitida")),
            "{:?}",
            slot.status
        );
    }

    #[test]
    fn an_oversized_body_is_refused() {
        let (port, server) = serve("200 OK", vec![0u8; (MAX_BYTES + 10) as usize]);
        let images = Images::default();
        images.set_origin(&format!("http://127.0.0.1:{port}"), None);
        let slot = wait(&images, "/big");
        assert!(
            matches!(&slot.status, Status::Failed(m) if m.contains("pesado")),
            "{:?}",
            slot.status
        );
        let _ = server.join();
    }

    #[test]
    fn the_same_url_is_downloaded_once() {
        let (port, server) = serve("200 OK", png(1, 1));
        let images = Images::default();
        images.set_origin(&format!("http://127.0.0.1:{port}"), None);
        wait(&images, "/once");
        wait(&images, "/once");
        server.join().unwrap(); // un segundo pedido habría quedado sin servidor y fallado
        assert_eq!(images.get("/once").status, Status::Ready);
    }
}
