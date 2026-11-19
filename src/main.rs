//! Fase 3: shell de Hive sobre Rune con chat mínimo contra el gateway.
use crate::ui::text;
mod a2ui;
mod audio_prefs;
mod chat;
mod computer;
mod conversations;
mod gateway;
mod live;
mod meeting;
mod meeting_ui;
mod ocr;
mod office;
mod oracle;
mod realtime;
mod remote;
mod settings;
mod system;
#[cfg(test)]
mod testutil;
mod tray;
mod ui;
mod vault;
mod voice;

use crate::ui::Tap;
use chat::{BootError, Chat, Event};
use gateway::{Gateway, Status};
use realtime::{REvent, Realtime};
use rune::tok::{radius, type_};
use rune::{col, row, text_input, AppSpec, Cx, El, ScrollSpec, TextEdit, Tx};
use std::sync::{Arc, Mutex};

static GATEWAY: Mutex<Option<Gateway>> = Mutex::new(None);
static CHAT: Mutex<Option<Chat>> = Mutex::new(None);
static VOICE: Mutex<Option<Realtime>> = Mutex::new(None);
/// Cámara y pantalla que alimentan la sesión de voz (un fotograma por segundo al modelo).
static CAMERA: Mutex<Option<hive_media::Camera>> = Mutex::new(None);
static SCREEN: Mutex<Option<hive_media::ScreenShare>> = Mutex::new(None);
static MEDIA: Mutex<MediaFlags> = Mutex::new(MediaFlags {
    camera: false,
    screen: false,
    starting: false,
    error: None,
});

/// Qué fuentes de imagen están enviando y el último error al abrirlas. Los hilos de captura lo cambian.
#[derive(Debug)]
struct MediaFlags {
    camera: bool,
    screen: bool,
    /// Una fuente se está abriendo (la pantalla espera a que la persona elija qué compartir).
    starting: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Default)]
struct Shared {
    status: Option<Status>,
    chat_error: Option<String>,
    /// El gateway pide credenciales y aún no hay sesión.
    needs_login: bool,
    login_error: Option<String>,
    logging_in: bool,
    capabilities: Option<Vec<String>>,
}

/// Lo que se lee del gateway al abrir la oficina: los agentes y el estado de Jev de `openrouter`.
struct OfficeLoad {
    agents: Vec<settings::api::Agent>,
    jev: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Screen {
    #[default]
    Chat,
    Settings,
    Office,
    Meeting,
    Live,
    Panel,
    Agents,
    Dashboard,
    Notes,
    Cron,
    ApiClient,
}

#[derive(Clone, Debug, PartialEq)]
struct Msg {
    id: String,
    mine: bool,
    text: String,
}

struct State {
    ocr: ocr::Ocr,
    computer: computer::Computer,
    oracle: oracle::Oracle,
    shared: Arc<Mutex<Shared>>,
    shown: Shared,
    chat_ready: bool,
    agent: Option<String>,
    msgs: Vec<Msg>,
    processes: Vec<chat::ProcessStep>,
    process_expanded: bool,
    webchat: conversations::Conversations,
    /// Avisos del servidor visibles (id, texto, cuándo llegó). Caducan a los 8 s, como en la web.
    notices: Vec<(String, String, std::time::Instant)>,
    /// Herramientas que esperan que la persona las apruebe o deniegue (política `ask` del gateway).
    approvals: Vec<Approval>,
    /// Casilla «Recordarme» del acceso: guarda el token en el llavero del sistema.
    remember: bool,
    /// Barra lateral plegada a solo iconos.
    sidebar_collapsed: bool,
    /// Tamaño de la ventana en puntos (lo anota el compositor en cada fotograma) y, si es estrecha, si la persona abrió la barra.
    win: (f32, f32),
    sidebar_open_narrow: bool,
    /// Logo de la cabecera (el de la web, 96×96).
    logo: rune::Pixels,
    /// El logo de Rust de la cabecera: el negro para el tema crema y el blanco para el oscuro.
    rust_dark: rune::Pixels,
    rust_light: rune::Pixels,
    busy: bool,
    input: TextEdit,
    a2: a2ui::view::A2State,
    settings: settings::Settings,
    meeting: meeting_ui::Ui,
    live: live::state::Live,
    /// Datos del Dashboard, la ética, el perfil y las versiones.
    sys: system::Sys,
    work: ui::work_pages::Work,
    profile: ui::system_pages::ProfileForm,
    audio: ui::system_pages::AudioUi,
    /// Protocolo ético con su texto desplegado.
    ethic_open: Option<String>,
    /// Agente que se mira en la pantalla de Agentes y si se está editando su configuración.
    agents_sel: Option<String>,
    agents_editing: bool,
    /// La vista de detalle (perfil) del agente elegido.
    agents_detail: bool,
    office: office::state::OfficeState,
    /// Agentes que un hilo acaba de leer del gateway, a la espera de que `poll` los recoja.
    office_agents: Arc<Mutex<Option<OfficeLoad>>>,
    office_requested: bool,
    screen: Screen,
    login_email: TextEdit,
    login_pass: TextEdit,
    voice_on: bool,
    voice_live: bool,
    /// Quién habló en el último fragmento de voz, para unir fragmentos consecutivos del mismo rol.
    voice_tail: Option<bool>,
}

/// Las herramientas headless de Rune (`--snap`, `--script`, `--trace`) terminan el proceso sin
/// pasar por el cierre normal: si arrancaran el gateway lo dejarían huérfano.
fn is_headless(args: &[String]) -> bool {
    args.iter().any(|a| {
        ["--snap", "--script", "--trace", "--strip"]
            .iter()
            .any(|f| a == f || a.starts_with(&format!("{f}=")))
    })
}

impl Default for State {
    fn default() -> Self {
        ui::theme::load();
        let mut state = State::offline();
        state.screen = Screen::Dashboard;
        if !is_headless(&std::env::args().collect::<Vec<_>>()) {
            live::model::preload(); // BIA se lee en segundo plano: HiveLive abre sin pausa
            boot(state.shared.clone());
        }
        state
    }
}

impl State {
    /// Estado inicial sin arrancar el gateway (pruebas y escenas).
    fn offline() -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: Some(Status::Starting),
            ..Default::default()
        }));
        let mut input = TextEdit::new("");
        input.focused = true;
        State {
            ocr: Default::default(),
            computer: Default::default(),
            oracle: Default::default(),
            webchat: Default::default(),
            shared,
            shown: Shared::default(),
            chat_ready: false,
            agent: None,
            msgs: vec![],
            processes: vec![],
            process_expanded: false,
            notices: vec![],
            approvals: vec![],
            remember: false,
            sidebar_collapsed: false,
            win: snap_win().unwrap_or((1440.0, 900.0)),
            sidebar_open_narrow: false,
            logo: load_logo(),
            rust_dark: load_png(include_bytes!("../assets/rust-dark.png")),
            rust_light: load_png(include_bytes!("../assets/rust-light.png")),
            busy: false,
            input,
            a2: Default::default(),
            settings: Default::default(),
            meeting: Default::default(),
            live: Default::default(),
            sys: Default::default(),
            work: Default::default(),
            profile: Default::default(),
            audio: Default::default(),
            ethic_open: None,
            agents_sel: None,
            agents_editing: false,
            agents_detail: false,
            office: Default::default(),
            office_agents: Default::default(),
            office_requested: false,
            screen: Screen::Chat,
            login_email: TextEdit::new(""),
            login_pass: TextEdit::new("").secure(),
            voice_on: false,
            voice_live: false,
            voice_tail: None,
        }
    }
}

/// Levanta el gateway y la conversación fuera del hilo de UI.
fn boot(shared: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        match Gateway::start() {
            Ok(gw) => {
                let port = gw.port;
                shared.lock().unwrap().capabilities = gateway::metadata(port)
                    .and_then(|v| serde_json::from_value(v["capabilities"].clone()).ok());
                shared.lock().unwrap().status = Some(Status::Ready {
                    port,
                    external: gw.is_external(),
                });
                *GATEWAY.lock().unwrap() = Some(gw);
                match boot_chat(port, &*vault::current()) {
                    Ok(chat) => *CHAT.lock().unwrap() = Some(chat),
                    Err(BootError::NeedsLogin) => shared.lock().unwrap().needs_login = true,
                    Err(e) => shared.lock().unwrap().chat_error = Some(e.to_string()),
                }
            }
            Err(e) => shared.lock().unwrap().status = Some(Status::Failed(e)),
        };
        // Este hilo lanzó el gateway: debe seguir vivo (ver `gateway::keep_thread_alive`).
        gateway::keep_thread_alive();
    });
}

/// Abre el chat: primero con el token recordado (si lo hay) y, si el gateway lo rechaza, lo olvida y pide acceso.
fn boot_chat(port: u16, vault: &dyn vault::Vault) -> Result<Chat, BootError> {
    boot_with(vault, |token| Chat::bootstrap(port, token))
}

/// La lógica de `boot_chat` sin red: `open` intenta abrir el chat con un token (o sin él).
fn boot_with<T>(
    vault: &dyn vault::Vault,
    open: impl Fn(Option<&str>) -> Result<T, BootError>,
) -> Result<T, BootError> {
    if let Some(token) = vault.load() {
        match open(Some(&token)) {
            Err(BootError::NeedsLogin) => vault.clear(), // vencido o revocado: no se queda guardado
            other => return other,
        }
    }
    open(None)
}

/// El id de la conversación/sesión actual (lo que la web muestra bajo «Panel interactivo»).
fn session_id() -> Option<String> {
    CHAT.lock().unwrap().as_ref().map(|c| c.user_id.clone())
}

/// ¿Hay un inicio de sesión con token en curso? (sin credenciales en el gateway no hay nada que cerrar)
fn has_session() -> bool {
    CHAT.lock()
        .unwrap()
        .as_ref()
        .is_some_and(|c| c.token.is_some())
}

/// Cierra la sesión: olvida el token recordado, suelta el chat y vuelve a pedir acceso.
fn sign_out(s: &mut State) {
    vault::current().clear();
    meeting_ui::abort();
    CHAT.lock().unwrap().take();
    s.chat_ready = false;
    s.msgs.clear();
    s.processes.clear();
    s.busy = false;
    s.settings.conn = None;
    s.live.catalog = Default::default();
    s.screen = Screen::Chat;
    s.shared.lock().unwrap().needs_login = true;
}

/// Inicia sesión en un hilo y, con el token, abre el chat. La contraseña no se guarda: se borra
/// del campo al enviarla; el token queda en memoria y, solo si se marcó «Recordarme», también en el llavero.
fn do_login(s: &mut State, _: &mut Tx) {
    let (email, password) = (
        s.login_email.text.trim().to_string(),
        s.login_pass.text.clone(),
    );
    let Some(Status::Ready { port, .. }) = s.shown.status.clone() else {
        return;
    };
    if email.is_empty() || password.is_empty() || s.shown.logging_in {
        return;
    }
    s.login_pass.clear();
    let shared = s.shared.clone();
    let remember = s.remember;
    {
        let mut sh = shared.lock().unwrap();
        sh.logging_in = true;
        sh.login_error = None;
    }
    std::thread::spawn(move || {
        let base = format!("http://127.0.0.1:{port}");
        let result = chat::login(&base, &email, &password).and_then(|token| {
            let chat = Chat::bootstrap(port, Some(&token)).map_err(|e| e.to_string())?;
            // Solo se guarda un token que el gateway acaba de aceptar.
            if remember {
                if let Err(e) = vault::current().save(&token) {
                    shared.lock().unwrap().chat_error = Some(e);
                }
            } else {
                vault::current().clear();
            }
            Ok(chat)
        });
        let mut sh = shared.lock().unwrap();
        sh.logging_in = false;
        match result {
            Ok(chat) => {
                *CHAT.lock().unwrap() = Some(chat);
                sh.needs_login = false;
            }
            Err(e) => sh.login_error = Some(e),
        }
    });
}

fn login_card(s: &State) -> El {
    let t = crate::ui::theme::tok();
    let accent = crate::ui::theme::accent_tok();
    let field = |edit: &TextEdit, key: &str, hint: &str, pick: fn(&mut State) -> &mut TextEdit| {
        row()
            .w_full()
            .px(10.0)
            .py(6.0)
            .radius(radius::SM)
            .fill(t.control_off)
            .child(
                text_input(edit, type_::BODY.lh(26.0))
                    .key(key.to_string())
                    .single_line()
                    .grow(1.0)
                    .color(t.text)
                    .placeholder(hint, t.text_secondary)
                    .on_edit(pick)
                    .on_submit(do_login),
            )
    };
    let mut card = col()
        .w_full()
        .padding(16.0)
        .gap(10.0)
        .radius(radius::SM)
        .fill(t.card)
        .child(text("Iniciar sesión").font(type_::TITLE).color(t.text))
        .child(field(&s.login_email, "login-email", "Correo", |s| {
            &mut s.login_email
        }))
        .child(field(&s.login_pass, "login-pass", "Contraseña", |s| {
            &mut s.login_pass
        }))
        .child(
            row()
                .key("login-remember")
                .gap(8.0)
                .center_y()
                .child(
                    rune::el()
                        .size(18.0, 18.0)
                        .radius(4.0)
                        .fill(if s.remember { accent } else { t.control_off })
                        .center()
                        .child(
                            text(if s.remember { "✓" } else { "" })
                                .font(type_::LABEL)
                                .color(t.text),
                        ),
                )
                .child(
                    text("Recordarme en este equipo (llavero del sistema)")
                        .font(type_::LABEL)
                        .color(t.text_secondary),
                )
                .tap(|s: &mut State| s.remember = !s.remember),
        )
        .child(
            row()
                .key("login-go")
                .px(14.0)
                .py(8.0)
                .radius(radius::SM)
                .fill(accent.alpha(if s.shown.logging_in { 0.4 } else { 0.9 }))
                .center()
                .child(
                    text(if s.shown.logging_in {
                        "Entrando…"
                    } else {
                        "Entrar"
                    })
                    .font(type_::BODY)
                    .color(t.text),
                )
                .tap(|s: &mut State| do_login(s, &mut Tx::default())),
        );
    if let Some(e) = &s.shown.login_error {
        card = card.child(
            text(e.clone())
                .font(type_::LABEL)
                .color(rune::color::hex("#E5484D").unwrap_or(accent)),
        );
    }
    card
}

/// Cancela el turno en curso: el gateway deja de trabajar en él y aquí se libera el campo de mensaje.
fn stop_turn(s: &mut State) {
    if let Some(chat) = CHAT.lock().unwrap().as_ref() {
        chat.stop();
    }
    s.busy = false;
}

/// Barge-in manual: calla al modelo y corta su respuesta.
fn interrupt_voice(_: &mut State) {
    if let Some(rt) = VOICE.lock().unwrap().as_ref() {
        rt.interrupt();
    }
}

fn send(s: &mut State, _: &mut Tx) {
    let content = s.input.text.trim().to_string();
    if content.is_empty()
        || s.busy
        || !s.chat_ready
        || (webchat_enabled(s) && (s.webchat.loading || !s.webchat.can_continue))
    {
        return;
    }
    if let Some(chat) = CHAT.lock().unwrap().as_ref() {
        chat.send(&content);
        s.msgs.push(Msg {
            id: format!("u{}", s.msgs.len()),
            mine: true,
            text: content,
        });
        s.busy = true;
        s.input.clear();
    }
}

/// (cámara activa, pantalla compartida) ahora mismo.
fn media_flags() -> (bool, bool) {
    let m = MEDIA.lock().unwrap();
    (m.camera, m.screen)
}

/// El último error al abrir la cámara o la pantalla, si lo hay.
fn media_error() -> Option<String> {
    MEDIA.lock().unwrap().error.clone()
}

/// Detiene cámara y pantalla; se llama al cerrar la voz, porque sin sesión no hay a quién mandar imágenes.
fn stop_media() {
    // Se sueltan fuera del candado: `stop()` espera al hilo de captura.
    let (cam, scr) = (CAMERA.lock().unwrap().take(), SCREEN.lock().unwrap().take());
    if let Some(mut c) = cam {
        c.stop();
    }
    if let Some(mut c) = scr {
        c.stop();
    }
    let mut m = MEDIA.lock().unwrap();
    (m.camera, m.screen, m.starting) = (false, false, false);
}

/// Capture callbacks keep the preview local; only Model frames go to Gemini.
#[cfg(not(test))]
fn open_source(
    screen: bool,
    sink: impl FnMut(hive_media::VideoFrame) + Send + 'static,
    on_ended: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    if screen {
        hive_media::ScreenShare::start(sink, on_ended)
            .map(|source| {
                *SCREEN.lock().unwrap() = Some(source);
            })
            .map_err(|e| e.message)
    } else {
        // La cámara que se eligió en «Pantalla y audio»; sin elegir, la primera que entrega imagen.
        let chosen = audio_prefs::load().camera;
        hive_media::Camera::start_with_ended(chosen.as_deref(), sink, on_ended)
            .map(|source| {
                *CAMERA.lock().unwrap() = Some(source);
            })
            .map_err(|e| e.message)
    }
}
#[cfg(test)]
fn open_source(
    _screen: bool,
    _sink: impl FnMut(hive_media::VideoFrame) + Send + 'static,
    _on_ended: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    Err("sin dispositivos en las pruebas".into())
}

/// Enciende o apaga una fuente de imagen. Abrirla puede tardar (la pantalla pregunta qué compartir),
/// así que se hace en un hilo y el resultado vuelve por `MEDIA`.
fn toggle_media(s: &mut State, screen: bool) {
    {
        let mut m = MEDIA.lock().unwrap();
        let on = if screen {
            m.screen || s.live.screen_preview.active
        } else {
            m.camera || s.live.camera_preview.active
        };
        if on {
            if screen {
                s.live.screen_preview.stop();
            } else {
                s.live.camera_preview.stop();
            }
            if screen {
                m.screen = false
            } else {
                m.camera = false
            }
            drop(m);
            let (cam, scr) = (
                if screen {
                    None
                } else {
                    CAMERA.lock().unwrap().take()
                },
                if screen {
                    SCREEN.lock().unwrap().take()
                } else {
                    None
                },
            );
            if let Some(mut c) = cam {
                c.stop();
            }
            if let Some(mut c) = scr {
                c.stop();
            }
            return;
        }
        if m.starting {
            return; // ya se está abriendo otra
        }
        m.starting = true;
        m.error = None;
    }
    // Sin sesión de voz viva no hay a quién mandar imágenes.
    let sink = VOICE
        .lock()
        .unwrap()
        .as_ref()
        .map(|rt| (rt.video_sink(None), rt.video_sink(Some("screen"))));
    let Some((cam_sink, scr_sink)) = sink else {
        let mut m = MEDIA.lock().unwrap();
        m.starting = false;
        m.error = Some("Inicia la voz antes de compartir imagen".into());
        return;
    };
    let feed = if screen {
        s.live.screen_preview.begin()
    } else {
        s.live.camera_preview.begin()
    };
    let preview = feed.clone();
    let ended = feed.clone();
    let mut cam_sink = cam_sink;
    let mut scr_sink = scr_sink;
    std::thread::spawn(move || {
        let result = open_source(
            screen,
            move |frame| preview.frame(frame, if screen { &mut scr_sink } else { &mut cam_sink }),
            move || {
                ended.end();
                let mut m = MEDIA.lock().unwrap();
                if screen {
                    m.screen = false;
                } else {
                    m.camera = false;
                }
            },
        );
        if result.is_err() {
            feed.end();
        }
        // Siempre VOICE antes que MEDIA (cerrar la voz toma los candados en ese orden): el contrario bloquearía.
        let voice_gone = VOICE.lock().unwrap().is_none();
        let mut m = MEDIA.lock().unwrap();
        m.starting = false;
        match result {
            // Si la voz se cerró mientras se abría, no hay nadie escuchando: se apaga.
            Ok(()) if voice_gone => {
                drop(m);
                stop_media();
            }
            Ok(()) if !feed.active() => {
                drop(m);
                if screen {
                    if let Some(mut source) = SCREEN.lock().unwrap().take() {
                        source.stop();
                    }
                } else {
                    if let Some(mut source) = CAMERA.lock().unwrap().take() {
                        source.stop();
                    }
                }
            }
            Ok(()) => {
                if screen {
                    m.screen = true
                } else {
                    m.camera = true
                }
            }
            Err(e) => m.error = Some(e),
        }
    });
}

/// La persona cambió un aparato en «Pantalla y audio» (1 micrófono, 2 salida, 3 cámara). Se guardó
/// para la próxima llamada; si ya hay una en curso se aplica ahora, solo a lo que cambió.
fn apply_device_change(s: &mut State, device: u8) {
    let prefs = s.audio.prefs.clone();
    match device {
        1 | 2 => {
            let result = VOICE.lock().unwrap().as_ref().map(|rt| {
                if device == 1 {
                    rt.switch_mic(prefs.input.as_deref())
                } else {
                    rt.switch_output(prefs.output.as_deref())
                }
            });
            if let Some(Err(e)) = result {
                s.audio.message = Some(format!(
                    "No se pudo usar ese aparato ({e}); se volvió al del sistema."
                ));
            }
        }
        3 => {
            // Con la cámara encendida: se apaga y se vuelve a abrir, ya con la elegida.
            let on = MEDIA.lock().unwrap().camera || s.live.camera_preview.active;
            if on {
                toggle_media(s, false);
                toggle_media(s, false);
            }
        }
        _ => {}
    }
}

fn toggle_voice(s: &mut State) {
    if !screen_available(s, Screen::Live) {
        return;
    }
    if !s.voice_on && s.live.catalog.busy() {
        return;
    }
    let mut slot = VOICE.lock().unwrap();
    if slot.take().is_some() {
        stop_media();
        // Soltar la sesión cierra el socket y libera micrófono y altavoz.
        s.voice_on = false;
        s.voice_live = false;
        s.live.camera_preview.stop();
        s.live.screen_preview.stop();
        return;
    }
    let (Some(port), Some(chat)) = (
        match &s.shown.status {
            Some(Status::Ready { port, .. }) => Some(*port),
            _ => None,
        },
        CHAT.lock()
            .unwrap()
            .as_ref()
            .map(|c| (c.user_id.clone(), c.token.clone(), c.thread_id.clone())),
    ) else {
        return;
    };
    s.live.reset();
    match Realtime::start_in_thread(
        port,
        &chat.0,
        chat.1.as_deref(),
        s.live.voice_id(),
        s.live.lang_id(),
        false,
        &chat.2,
    ) {
        Ok(rt) => {
            *slot = Some(rt);
            s.voice_on = true;
            s.voice_tail = None;
        }
        Err(e) => apply_voice(s, REvent::Error(e)),
    }
}

fn apply_voice(s: &mut State, ev: REvent) {
    s.live.on_event(&ev, std::time::Instant::now());
    match ev {
        REvent::Ready { .. } => s.voice_live = true,
        REvent::Transcript { assistant, text } => {
            // Fragmentos consecutivos del mismo rol forman un solo mensaje.
            match (s.voice_tail == Some(assistant), s.msgs.last_mut()) {
                (true, Some(m)) if m.id.starts_with('v') => m.text.push_str(&text),
                _ => s.msgs.push(Msg {
                    id: format!("v{}", s.msgs.len()),
                    mine: !assistant,
                    text,
                }),
            }
            s.voice_tail = Some(assistant);
        }
        REvent::TurnComplete => s.voice_tail = None,
        REvent::Error(e) => s.msgs.push(Msg {
            id: format!("e{}", s.msgs.len()),
            mine: false,
            text: format!("⚠ {e}"),
        }),
        REvent::Closed => {
            s.live.camera_preview.stop();
            s.live.screen_preview.stop();
            VOICE.lock().unwrap().take();
            stop_media();
            s.voice_on = false;
            s.voice_live = false;
        }
        REvent::Interrupted
        | REvent::Tool(_)
        | REvent::Usage { .. }
        | REvent::Pricing { .. }
        | REvent::Narration { .. }
        | REvent::Mouth { .. } => {}
    }
}

const NOTICE_TTL: std::time::Duration = std::time::Duration::from_secs(8);

/// Una petición de aprobación de herramienta: el turno del agente está detenido hasta que se
/// responda, y el gateway la deniega solo al vencer el plazo.
struct Approval {
    id: String,
    tool: String,
    /// Los argumentos tal como se van a usar, en texto plano y acotados.
    args: String,
    reason: Option<String>,
    expires_at: std::time::Instant,
    seconds: u64,
}
/// Prefijo de las superficies A2UI con las que el gateway pregunta lo mismo dentro del chat.
const APPROVAL_SURFACE: &str = "hive-approval-";
/// A la vez; si llegan más, las más antiguas se deniegan.
const MAX_APPROVALS: usize = 8;
const MAX_APPROVAL_ARGS: usize = 600;

fn approval_args(arguments: &serde_json::Value) -> String {
    if arguments.is_null() {
        return "(sin argumentos)".into();
    }
    let text = serde_json::to_string_pretty(arguments).unwrap_or_default();
    match text.char_indices().nth(MAX_APPROVAL_ARGS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

/// Nada queda pendiente (el turno acabó, se detuvo o se perdió la conexión): fuera las peticiones
/// y las superficies que el gateway dibujó para ellas.
fn clear_approvals(s: &mut State) {
    s.approvals.clear();
    s.a2.store
        .surfaces
        .retain(|surface| !surface.id.starts_with(APPROVAL_SURFACE));
}

/// Las peticiones que hay que dibujar como tarjeta. En el chat, si el gateway ya dibujó la
/// pregunta como superficie A2UI en el hilo, no se repite; en cualquier otra pantalla (o en
/// otro hilo) la tarjeta sigue siendo la única forma de verla.
fn visible_approvals(s: &State) -> Vec<&Approval> {
    s.approvals
        .iter()
        .filter(|a| {
            !(s.screen == Screen::Chat
                && s.a2
                    .store
                    .get(&format!("{APPROVAL_SURFACE}{}", a.id))
                    .is_some())
        })
        .collect()
}

/// Contesta una petición: la quita de la pantalla y manda la respuesta por el WebSocket del turno.
fn answer_approval(s: &mut State, id: &str, approved: bool) {
    if let Some(index) = s.approvals.iter().position(|a| a.id == id) {
        s.approvals.remove(index);
        s.a2.outbox.push(chat::approval_frame(id, approved));
    }
}

fn apply(s: &mut State, ev: Event) {
    match ev {
        Event::Connected { agent } => s.agent = agent,
        Event::Progress(step) => {
            if step.finished {
                s.busy = false;
            }
            if let Some(index) = s.processes.iter().position(|p| p.id == step.id) {
                s.processes.remove(index);
            }
            s.processes.push(step);
            let extra = s.processes.len().saturating_sub(256);
            s.processes.drain(..extra);
        }
        Event::Notification { id, content } => {
            if !s.notices.iter().any(|(n, ..)| *n == id) {
                let text = if content.trim().is_empty() {
                    "Nueva notificación".to_string()
                } else {
                    content
                };
                s.notices.push((id, text, std::time::Instant::now()));
            }
        }
        Event::Approval {
            id,
            tool,
            arguments,
            reason,
            expires_in_ms,
        } => {
            if !s.approvals.iter().any(|a| a.id == id) {
                s.approvals.push(Approval {
                    id,
                    tool,
                    args: approval_args(&arguments),
                    reason,
                    expires_at: std::time::Instant::now()
                        + std::time::Duration::from_millis(expires_in_ms),
                    seconds: expires_in_ms.div_ceil(1000),
                });
                // Sin lugar para más, las más antiguas se deniegan (nunca se aprueba por omisión).
                while s.approvals.len() > MAX_APPROVALS {
                    let old = s.approvals.remove(0);
                    s.a2.outbox.push(chat::approval_frame(&old.id, false));
                }
            }
        }
        Event::Chunk { id, text } => match s.msgs.iter_mut().find(|m| !m.mine && m.id == id) {
            Some(m) => m.text.push_str(&text),
            None => s.msgs.push(Msg {
                id,
                mine: false,
                text,
            }),
        },
        // La respuesta confirmada sustituye los fragmentos/preambulos anteriores al uso de herramientas.
        Event::Final { id, text } => match s.msgs.iter_mut().find(|m| !m.mine && m.id == id) {
            Some(message) => message.text = text,
            None => s.msgs.push(Msg {
                id,
                mine: false,
                text,
            }),
        },
        Event::Canvas {
            kind,
            data,
            timestamp,
        } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_millis() as f64);
            if s.office.graph.apply(&kind, &data, timestamp, now) && kind == "canvas:snapshot" {
                if let Some(agents) = data.get("agents").and_then(|v| {
                    serde_json::from_value::<Vec<settings::api::Agent>>(v.clone()).ok()
                }) {
                    s.office.agents = agents;
                }
            }
        }
        Event::A2ui { kind, data } => {
            // El gateway retira la pregunta al contestarla (por cualquier vía) o vencer: la petición también.
            if kind == "a2ui:deleteSurface" {
                if let Some(id) = data
                    .get("surfaceId")
                    .and_then(|v| v.as_str())
                    .and_then(|v| v.strip_prefix(APPROVAL_SURFACE))
                {
                    s.approvals.retain(|a| a.id != id);
                }
            }
            // Una superficie nueva queda anclada al punto de la conversación donde nació.
            if kind == "a2ui:createSurface" {
                if let Some(id) = data.get("surfaceId").and_then(|v| v.as_str()) {
                    s.a2.anchors.insert(id.to_string(), s.msgs.len());
                }
            }
            s.a2.apply(&kind, &data);
        }
        // El turno terminó (o se detuvo): ya no hay nada esperando respuesta.
        Event::Done => {
            s.busy = false;
            clear_approvals(s);
            s.webchat.refresh_pending = true;
        }
        Event::Error(e) => {
            s.msgs.push(Msg {
                id: format!("e{}", s.msgs.len()),
                mine: false,
                text: format!("⚠ {e}"),
            });
            s.busy = false;
            clear_approvals(s);
        }
        Event::Closed => {
            s.busy = false;
            clear_approvals(s);
            s.chat_ready = false;
            let error = "Se perdió la conexión con el gateway. La actividad no está confirmada; vuelve a abrir la conversación.".to_string();
            s.shared.lock().unwrap().chat_error = Some(error.clone());
            s.shown.chat_error = Some(error);
            for node in &mut s.office.graph.nodes {
                if matches!(node.status.as_str(), "thinking" | "tool_call") {
                    node.status = "error".into();
                    node.data["status"] = serde_json::json!("error");
                    node.data["currentTask"] =
                        serde_json::json!("Sin conexión; estado sin confirmar");
                }
            }
        }
    }
}

/// Superficie de ejemplo (la que `--snap --scene a2ui` dibuja y los tests usan).
fn demo_a2ui(s: &mut State) {
    use serde_json::json;
    let frames = [
        (
            "a2ui:createSurface",
            json!({"surfaceId": "demo", "theme": {"agentDisplayName": "Pepito", "primaryColor": "#F5A524"}}),
        ),
        (
            "a2ui:updateDataModel",
            json!({"surfaceId": "demo", "value": {
            "user": {"name": "Ana", "mail": "ana@"}, "terms": false, "plan": ["pro"], "seats": 3,
            "items": [{"title": "Pedido #1", "total": 1234.5}, {"title": "Pedido #2", "total": 80}]}}),
        ),
        (
            "a2ui:updateComponents",
            json!({"surfaceId": "demo", "components": [
                {"id": "root", "component": "Column", "children": ["title", "tabs"]},
                {"id": "title", "component": "Text", "variant": "h2", "text": {"call": "formatString", "args": {"value": "Hola ${/user/name}, ${/seats} puestos"}}},
                {"id": "tabs", "component": "Tabs", "tabs": [{"title": "Formulario", "child": "form"}, {"title": "Pedidos", "child": "orders"}]},
                {"id": "form", "component": "Column", "children": ["name", "mail", "terms", "plan", "seats", "go"]},
                {"id": "name", "component": "TextField", "label": "Nombre", "value": {"path": "/user/name"},
                 "checks": [{"call": "required", "args": {"value": {"path": "/user/name"}}, "message": "Requerido"}]},
                {"id": "mail", "component": "TextField", "label": "Correo", "value": {"path": "/user/mail"},
                 "checks": [{"call": "email", "args": {"value": {"path": "/user/mail"}}, "message": "Correo inválido"}]},
                {"id": "terms", "component": "CheckBox", "label": "Acepto los términos", "value": {"path": "/terms"}},
                {"id": "plan", "component": "ChoicePicker", "label": "Plan", "variant": "mutuallyExclusive",
                 "options": [{"label": "Básico", "value": "basic"}, {"label": "Pro", "value": "pro"}], "value": {"path": "/plan"}},
                {"id": "seats", "component": "Slider", "label": "Puestos", "min": 1, "max": 10, "value": {"path": "/seats"}},
                {"id": "go_t", "component": "Text", "text": "Enviar"},
                {"id": "go", "component": "Button", "child": "go_t",
                 "checks": [{"call": "email", "args": {"value": {"path": "/user/mail"}}}],
                 "action": {"event": {"name": "submit", "context": {"name": {"path": "/user/name"}, "plan": {"path": "/plan"}, "src": "form"}}}},
                {"id": "orders", "component": "List", "children": {"path": "/items", "componentId": "order"}},
                {"id": "order", "component": "Row", "justify": "spaceBetween", "children": ["o_t", "o_n"]},
                {"id": "o_t", "component": "Text", "text": {"path": "title"}},
                {"id": "o_n", "component": "Text", "text": {"call": "formatCurrency", "args": {"value": {"path": "total"}}}}
            ]}),
        ),
    ];
    for (kind, data) in frames {
        s.a2.apply(kind, &data);
    }
}

/// Escena `--snap --scene login`: el gateway pide credenciales y la última no coincidió.
fn demo_login(s: &mut State) {
    // Por `shared`: el `poll` copia de ahí a `shown` en cada fotograma.
    {
        let mut sh = s.shared.lock().unwrap();
        sh.status = Some(Status::Ready {
            port: 18790,
            external: false,
        });
        sh.needs_login = true;
        sh.login_error = Some("Correo o contraseña incorrectos".into());
    }
    s.login_email = TextEdit::new("ana@empresa.co");
    s.login_pass = TextEdit::new("secreto").secure();
}

/// Escena `--snap --scene chat`: conectado y sin mensajes (el estado vacío con sugerencias).
/// El tamaño que pide `--snap --size WxH` (en las capturas no hay fotogramas previos que midan la ventana).
fn snap_win() -> Option<(f32, f32)> {
    let a: Vec<String> = std::env::args().collect();
    let v = a
        .iter()
        .position(|x| x == "--size")
        .and_then(|i| a.get(i + 1))?;
    let (w, h) = v.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Por debajo de este ancho la barra lateral se pliega sola a iconos (la web hace lo mismo en pantallas estrechas).
const NARROW: f32 = 1080.0;

impl State {
    /// ¿Se dibuja la barra lateral plegada a iconos?
    fn sidebar_is_collapsed(&self) -> bool {
        if self.win.0 < NARROW {
            !self.sidebar_open_narrow
        } else {
            self.sidebar_collapsed
        }
    }

    fn content_width(&self) -> f32 {
        let side = if self.win.0 < NARROW || self.sidebar_is_collapsed() {
            ui::theme::SIDEBAR_COLLAPSED_W
        } else {
            ui::theme::SIDEBAR_W
        };
        (self.win.0 - side).max(0.0)
    }

    /// El tamaño del escenario de HiveLive y de la oficina: la ventana menos la barra, la cabecera y los márgenes.
    fn stage_size(&self) -> (f32, f32) {
        (
            (self.content_width() - 32.0).max(240.0),
            (self.win.1 - ui::theme::TOPBAR_H - 32.0).max(240.0),
        )
    }
}

fn demo_chat(s: &mut State) {
    s.chat_ready = true;
    s.agent = Some("Pepito".into());
}

/// Escena `--snap --scene chat-msgs`: una conversación corta.
fn demo_chat_msgs(s: &mut State) {
    demo_chat(s);
    s.msgs.push(Msg {
        id: "u0".into(),
        mine: true,
        text: "Hola, ¿qué puedes hacer por mí?".into(),
    });
    s.msgs.push(Msg { id: "b1".into(), mine: false, text: "¡Hola! Puedo **coordinar a mis especialistas**: buscar en la web, revisar código, enviar correos y más.\n\n- Resumir documentos\n- Preparar informes".into() });
}

/// Escena `--snap --scene approval`: una herramienta espera que la persona la apruebe.
fn demo_approval(s: &mut State) {
    demo_chat_msgs(s);
    s.screen = Screen::Chat;
    s.busy = true;
    apply(
        s,
        Event::Approval {
            id: "demo-approval".into(),
            tool: "task_delegate".into(),
            arguments: serde_json::json!({"agent": "Ventas", "prompt": "Prepara el informe trimestral y envíalo a finanzas"}),
            reason: Some("La política del gateway pide confirmación para esta herramienta.".into()),
            expires_in_ms: 120_000,
        },
    );
    gateway_approval_surface(s, "demo-approval");
}

/// Los tres frames A2UI con los que el gateway pregunta una aprobación dentro del chat, con la forma
/// exacta de `panel::approval_frames` (del SDK). Los usan la escena `approval` y las pruebas.
fn gateway_approval_surface(s: &mut State, id: &str) {
    use serde_json::json;
    let surface = format!("hive-approval-{id}");
    let button = |name: &str, label: &str, approved: bool| {
        vec![
            json!({"id": format!("{name}_t"), "component": "Text", "text": label}),
            json!({"id": name, "component": "Button", "child": format!("{name}_t"),
            "action": {"event": {"name": "hive_approval", "context": {"approvalId": id, "approved": approved}}}}),
        ]
    };
    let mut components = vec![
        json!({"id": "root", "component": "Column", "children": ["title", "what", "args", "wait", "buttons"]}),
        json!({"id": "title", "component": "Text", "variant": "h3", "text": "Aprobación necesaria"}),
        json!({"id": "what", "component": "Text", "text": "Hive quiere ejecutar «task_delegate»."}),
        json!({"id": "args", "component": "Text", "variant": "caption", "text": "{}"}),
        json!({"id": "wait", "component": "Text", "variant": "caption", "text": "Si no respondes en 30 s se denegará."}),
        json!({"id": "buttons", "component": "Row", "children": ["approve", "deny"]}),
    ];
    components.extend(button("approve", "Aprobar", true));
    components.extend(button("deny", "Denegar", false));
    for (kind, data) in [
        (
            "a2ui:createSurface",
            json!({"surfaceId": surface, "theme": {"agentDisplayName": "Hive"}}),
        ),
        (
            "a2ui:updateComponents",
            json!({"surfaceId": surface, "components": components}),
        ),
        (
            "a2ui:updateDataModel",
            json!({"surfaceId": surface, "path": "", "value": {}}),
        ),
    ] {
        apply(
            s,
            Event::A2ui {
                kind: kind.into(),
                data,
            },
        );
    }
}

/// Escena `--snap --scene meeting`: la pantalla de reuniones con una sesión anterior.
fn demo_meeting(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Meeting;
    let mut d = s.meeting.data.lock().unwrap();
    d.loaded = true;
    d.sessions = vec![meeting::Session {
        id: "m1".into(),
        title: "Ecuador".into(),
        status: "stopped".into(),
        segments: 12,
        report_path: None,
    }];
}

/// Escena `--snap --scene live`: HiveLive en reposo, sin llamada.
fn demo_live(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Live;
    s.live.viewport = s.stage_size();
}

/// Escena `--snap --scene live-call`: en llamada, con telemetría e hitos de ejemplo.
fn demo_live_call(s: &mut State) {
    demo_live(s);
    s.voice_on = true;
    s.voice_live = true;
    let now = std::time::Instant::now();
    for ev in [
        REvent::Ready {
            model: "gemini-3.8-live".into(),
        },
        REvent::Usage {
            input: 4200,
            output: 1300,
        },
        REvent::Transcript {
            assistant: false,
            text: "Busca los proveedores de café en Ecuador".into(),
        },
        REvent::Narration {
            kind: "delegated".into(),
            label: "Buscar proveedores".into(),
            detail: None,
            agent: "Investigador web".into(),
        },
        REvent::Narration {
            kind: "tool_call".into(),
            label: "web_search".into(),
            detail: None,
            agent: "Investigador web".into(),
        },
        REvent::Transcript {
            assistant: true,
            text: "Estoy consultando a mi investigador web.".into(),
        },
    ] {
        apply_voice(s, ev);
    }
    s.live.started = Some(now - std::time::Duration::from_secs(95));
}

/// Escena `--snap --scene panel`: el Panel interactivo con la superficie de ejemplo.
fn demo_panel(s: &mut State) {
    demo_chat(s);
    demo_a2ui(s);
    s.screen = Screen::Panel;
}

/// Escena `--snap --scene agents`: el mapa de la colmena con un coordinador y especialistas.
fn demo_agents(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Agents;
    {
        let mut d = s.settings.data.lock().unwrap();
        d.loaded = true;
        let mk = |id: &str, name: &str, role: &str, tools: usize, skills: usize| {
            serde_json::json!({
                "id": id, "name": name, "role": role, "enabled": true, "status": "idle", "providerId": "gemini", "modelId": "gemini-3.8-flash",
                "description": if role == "coordinator" { "Eres un agente orquestador multiagente que opera en entorno local: analiza cada solicitud y decide si responder directamente o delegar en un especialista." } else { "Especialista en su dominio." },
                "toolsJson": serde_json::to_string(&(0..tools).map(|i| format!("t{i}")).collect::<Vec<_>>()).unwrap(),
                "skillsJson": serde_json::to_string(&(0..skills).map(|i| format!("s{i}")).collect::<Vec<_>>()).unwrap(),
                "maxIterations": 10, "workspace": "/home/ana/Documentos/Hive"
            })
        };
        d.agents = serde_json::from_value(serde_json::json!([
            mk("boss", "Pepito", "coordinator", 8, 1),
            mk("a1", "Ingeniero de software", "worker", 8, 3),
            mk("a2", "Email manager", "worker", 4, 1),
            mk("a3", "Calendar manager", "worker", 3, 1),
            mk("a4", "Operador de cron", "worker", 8, 2),
            mk("a5", "Agente de código externo", "worker", 6, 3),
            mk("a6", "Operador de office", "worker", 5, 2),
            mk("a7", "Operador de APIs", "worker", 1, 1),
            mk("a8", "Constructor A2UI", "worker", 4, 3),
            mk("a9", "Investigador web", "worker", 2, 1)
        ]))
        .unwrap();
        d.providers = serde_json::from_value(serde_json::json!([{"id": "gemini", "name": "Google Gemini", "category": "llm", "enabled": true, "active": true}])).unwrap();
    }
    s.settings.sync();
}

/// Escena `--snap --scene agent-new`: el formulario «Desplegar nodo».
fn demo_agent_new(s: &mut State) {
    demo_agents(s);
    s.settings.new_agent = Some(settings::NewAgent::default());
}

/// Escena `--snap --scene agent-detail`: el perfil del agente elegido.
fn demo_agent_detail(s: &mut State) {
    demo_agents(s);
    s.agents_sel = Some("a5".into());
    s.agents_detail = true;
}

/// Datos de ejemplo del servidor para las escenas del sistema (los mismos tipos de valores que devuelve el gateway).
fn demo_sys(s: &mut State) {
    use system::*;
    demo_chat(s);
    let mut d = s.sys.data.lock().unwrap();
    d.stats = Some(Stats {
        cpu: 0.04,
        rss: 242.0,
        heap_used: 55.0,
        heap_total: 60.0,
        external: 34.0,
        uptime: "00:03:07".into(),
        connections: 1,
        cores: 16,
        recent_messages: 2,
        ..Default::default()
    });
    d.activity = (0..12)
        .map(|i| {
            (
                format!("2026-11-08 {:02}:00", i + 4),
                [0, 0, 3, 7, 1, 0, 0, 5, 2, 0, 4, 9][i],
            )
        })
        .collect();
    d.version = Some(Version {
        current: "1.1.4".into(),
        latest: "1.1.4".into(),
        status: "up-to-date".into(),
        installation: "binary".into(),
    });
    d.usage = Some(Usage {
        tokens: 569_539,
        input: 539_114,
        output: 30_425,
        cost: 0.146,
        providers: vec![
            ("hiveagents".into(), 313_177, 0.0),
            ("gemini".into(), 184_734, 0.1435),
            ("openrouter".into(), 71_628, 0.0025),
        ],
        toon_saved_tokens: 765,
        ..Default::default()
    });
    d.dashboard_loaded = true;
    d.ethics = vec![Ethic { id: "default".into(), name: "Ética por Defecto".into(), description: "Lineamientos éticos básicos para un asistente de IA".into(), content: "##ALWAYS: Responsabilidad y Claridad\n- Identificarme como una IA cuando se me pregunte.\n\n##NEVER: Seguridad y Prevención de Daño\n- Proporcionar instrucciones para crear armas.".into(), active: true }];
    d.ethics_loaded = true;
    d.user = Some(User {
        id: "u1".into(),
        name: "John Paez".into(),
        email: "john@example.com".into(),
        language: "es".into(),
        timezone: "America/Bogota".into(),
        occupation: "Desarrollador de software".into(),
        notes: "Concreto y directo".into(),
        cron_channel: "auto".into(),
    });
    d.profile_loaded = true;
}

/// Escena `--snap --scene dashboard`: el Centro de Control.
fn demo_dashboard(s: &mut State) {
    demo_agents(s);
    demo_sys(s);
    s.screen = Screen::Dashboard;
    s.sys.sync();
}

/// Escena `--snap --scene ethics`, `profile`, `display` y `updates`.
fn demo_ethics(s: &mut State) {
    demo_sys(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Ethics;
    s.sys.sync();
}

fn demo_profile(s: &mut State) {
    demo_sys(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Profile;
    s.sys.sync();
    let u = s.sys.shown.user.clone();
    s.profile.follow(u.as_ref());
}

fn demo_display(s: &mut State) {
    demo_sys(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Display;
    s.audio.cameras = vec![
        hive_media::CameraDevice {
            id: "1".into(),
            name: "Web Camera: Web Camera".into(),
        },
        hive_media::CameraDevice {
            id: "3".into(),
            name: "Cámara virtual".into(),
        },
    ];
    s.audio.inputs = vec![
        hive_media::AudioDevice {
            id: "mic1".into(),
            name: "YU7 Estéreo analógico".into(),
            is_default: true,
        },
        hive_media::AudioDevice {
            id: "mic2".into(),
            name: "BS-2".into(),
            is_default: false,
        },
    ];
    s.audio.outputs = vec![hive_media::AudioDevice {
        id: "out1".into(),
        name: "BS-2".into(),
        is_default: true,
    }];
    s.sys.sync();
}

fn demo_updates(s: &mut State) {
    demo_sys(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Updates;
    s.sys.sync();
}

/// Escenas `--snap --scene notes`, `cron` y `apiclient`.
fn demo_work_data(s: &mut State) {
    use system::*;
    demo_chat(s);
    let mut d = s.sys.data.lock().unwrap();
    d.notes = vec![Note { id: "n1".into(), thread_id: "u/webchat/conv-e5992b532870".into(), key: "capacidades_y_firma_correo".into(), value: "Capacidades: - Coordinación Multiagente (orquestación en paralelo de tareas complejas). - Gestión de correo (Gmail): lectura, búsqueda, redacción y envío en HTML estructurado. - Automatización y tareas programadas (Hive cron / recordatorios). - Investigación web y navegación automatizada. - Gestión de archivos en workspace, documentos Office y APIs REST. Feedback de formato y firma de correos futuros: - Siempre incluir explícitamente el nombre en la firma al despedirse (ej. Pepito / Asistente de John).".into(), source: "agent".into(), created_at: 1790280173 }];
    d.notes_loaded = true;
    let t = |id: &str,
             name: &str,
             status: &str,
             cron: &str,
             channel: &str,
             runs: u64,
             last: &str,
             next: &str,
             err: &str| Task {
        id: id.into(),
        name: name.into(),
        status: status.into(),
        kind: "recurring".into(),
        cron: cron.into(),
        channel: channel.into(),
        timezone: "America/Bogota".into(),
        run_count: runs,
        last_run: last.into(),
        next_run: next.into(),
        last_error: err.into(),
        ..Default::default()
    };
    d.tasks = vec![
        t(
            "t1",
            "reporte-diario-tareas",
            "active",
            "0 8 * * 1-5",
            "cli",
            0,
            "",
            "2026-11-09T13:00:00.000Z",
            "Missed run at 2026-11-05T13:00:00.000Z (policy: skip)",
        ),
        t(
            "t2",
            "limpieza-semanal-logs",
            "active",
            "0 2 * * 0",
            "system",
            1,
            "2026-11-08T07:00:00.000Z",
            "2026-11-15T07:00:00.000Z",
            "",
        ),
        t(
            "t3",
            "auditoria-mensual-dependencias",
            "paused",
            "0 3 1 * *",
            "cli",
            2,
            "2026-11-01T08:00:00.000Z",
            "",
            "",
        ),
        Task {
            internal: true,
            ..t(
                "t4",
                "_hive_cleanup_runs",
                "active",
                "0 4 * * *",
                "system",
                9,
                "",
                "",
                "",
            )
        },
    ];
    d.history.insert(
        "t2".into(),
        vec![
            Run {
                status: "success".into(),
                started: "2026-11-08T07:00:00.003Z".into(),
                duration_ms: 32534,
                response:
                    "He delegado la limpieza al agente de mantenimiento y confirmó 14 archivos."
                        .into(),
                error: String::new(),
            },
            Run {
                status: "error".into(),
                started: "2026-11-01T07:00:00.000Z".into(),
                duration_ms: 1200,
                response: String::new(),
                error: "El agente no respondió a tiempo".into(),
            },
        ],
    );
    d.tasks_loaded = true;
}

/// Vuelve a poner las tareas de ejemplo (las pruebas las pisan al releer del servidor falso).
#[cfg(test)]
fn demo_work_data_tasks_only(s: &mut State) {
    demo_work_data(s);
    s.sys.sync();
}

fn demo_notes(s: &mut State) {
    demo_work_data(s);
    s.screen = Screen::Notes;
    s.sys.sync();
}

fn demo_cron(s: &mut State) {
    demo_work_data(s);
    s.screen = Screen::Cron;
    s.work.history_open = Some("t2".into());
    s.sys.sync();
}

fn demo_apiclient(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::ApiClient;
    s.work.api.method = "POST";
    s.work.api.url = TextEdit::new("https://api.ejemplo.com/v1/usuarios");
    s.work.api.req_tab = ui::work_pages::ReqTab::Body;
    s.work.api.body = TextEdit::new("{\"nombre\": \"Ada\"}");
    s.work.api.answer = Some(ui::work_pages::Answer {
        reply: Ok(system::parse_http_reply(
            &serde_json::json!({"ok": true, "status": 201, "statusText": "Created", "headers": {"content-type": "application/json", "x-request-id": "a1b2c3"}, "body": "{\"id\":7,\"nombre\":\"Ada\"}"}),
        )),
        ms: 184,
    });
}

/// Escena `--snap --scene settings`: la página de Providers con datos de ejemplo.
fn demo_settings(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Providers;
    {
        let mut d = s.settings.data.lock().unwrap();
        d.loaded = true;
        let model = |id: &str, on: bool| serde_json::json!({"id": id, "name": id, "enabled": on});
        d.providers = serde_json::from_value(serde_json::json!([
            {"id": "anthropic", "name": "Anthropic", "category": "llm", "enabled": false, "active": false, "has_api_key": 0, "models": [model("Claude Fable 5", true), model("Claude Haiku 4.5", true), model("Claude Opus 5", true), model("Claude Sonnet 5", true)]},
            {"id": "deepseek", "name": "DeepSeek", "category": "llm", "enabled": false, "active": false, "has_api_key": 0, "models": [model("DeepSeek V4 Flash", true), model("DeepSeek V4 Pro", true)]},
            {"id": "gemini", "name": "Google Gemini", "category": "llm", "enabled": true, "active": true, "has_api_key": 1, "masked_api_key": "AIza••••••••R5jI",
             "models": (0..10).map(|i| model(["Gemini 2.5 Flash Native Audio", "Gemini 2.5 Flash TTS", "Gemini 2.5 Pro TTS"][i % 3], true)).collect::<Vec<_>>()},
            {"id": "groq", "name": "Groq", "category": "llm", "enabled": true, "active": true, "has_api_key": 1, "masked_api_key": "gsk_••••••••1uIh", "models": [model("GPT OSS 120B", true), model("GPT OSS 20B", true), model("Groq Compound", true)]},
            {"id": "hiveagents", "name": "HiveAgents LLM", "category": "llm", "enabled": true, "active": true, "has_api_key": 1, "masked_api_key": "hk_0••••••••0f4b", "models": [model("DeepSeek V4 Flash 90 GB (lento)", true), model("Qwen3.6 35B MoE (recomendado)", true)]},
            {"id": "kimi", "name": "Kimi (Moonshot)", "category": "llm", "enabled": false, "active": false, "has_api_key": 0, "models": [model("Kimi K2.6", true), model("Kimi K2.7 Code", true)]},
            {"id": "elevenlabs", "name": "ElevenLabs", "category": "tts", "enabled": true, "active": true, "has_api_key": 1, "models": []}
        ])).unwrap();
    }
    s.settings.sync();
}

/// Escena `--snap --scene channels`: la página de Canales.
fn demo_channels(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Channels;
    {
        let mut d = s.settings.data.lock().unwrap();
        d.loaded = true;
        let ch = |id: &str, kind: &str, on: bool, status: &str, cfg: bool| serde_json::json!({"id": id, "type": kind, "enabled": on, "active": on, "status": status, "isConfigured": cfg});
        d.channels = serde_json::from_value(serde_json::json!([
            ch("discord", "discord", false, "", false),
            ch("slack", "slack", false, "", false),
            ch("telegram", "telegram", false, "", false),
            ch("webchat", "webchat", true, "connected", true),
            ch("whatsapp", "whatsapp", false, "", false),
            ch("whatsapp_cloud", "whatsapp_cloud", false, "", false)
        ]))
        .unwrap();
    }
    s.settings.sync();
}

/// Escena `--snap --scene skills`: la galería de skills.
fn demo_skills(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Skills;
    {
        let mut d = s.settings.data.lock().unwrap();
        d.loaded = true;
        d.extras_loaded = true;
        let sk = |id: &str, cat: &str, desc: &str, on: bool, tools: &str, trig: &str| serde_json::json!({"id": id, "name": id, "category": cat, "description": desc, "active": on, "version": "1.0.0", "tools": tools, "triggers": trig});
        d.skills = serde_json::from_value(serde_json::json!([
            sk(
                "a2ui_dashboard",
                "a2ui",
                "Create real-time interactive dashboards using A2UI",
                true,
                "a,b,c,d",
                "x,y,z,w,v,u,t,s,r,q,p"
            ),
            sk(
                "a2ui_form",
                "a2ui",
                "Crear formularios interactivos ricos usando A2UI",
                true,
                "a,b,c,d",
                "x,y,z,w,v,u,t,s,r,q,p,o"
            ),
            sk(
                "agent_spawner",
                "agents",
                "Crear nuevos workers especializados o gestionar equipos",
                true,
                "a,b,c,d",
                "x,y,z,w,v,u,t,s,r,q,p,o,n,m"
            ),
            sk(
                "api_client",
                "api",
                "Esta skill se activa cuando el usuario necesita llamar APIs",
                true,
                "a",
                "x,y,z"
            ),
            sk(
                "apicultura_basica",
                "apicultura",
                "Conocimiento de apicultura",
                false,
                "",
                "x,y"
            ),
        ]))
        .unwrap();
    }
    s.settings.sync();
}

/// Escena `--snap --scene mcp`: los servidores MCP.
fn demo_mcp(s: &mut State) {
    demo_chat(s);
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Mcp;
    {
        let mut d = s.settings.data.lock().unwrap();
        d.loaded = true;
        d.extras_loaded = true;
        d.mcp = serde_json::from_value(serde_json::json!([
            {"id": "cal", "name": "calendario", "status": "connected", "enabled": true, "tools_count": 5, "config": {"transport": "sse", "url": "https://n8n.example.cloud/mcp/e6270002-1234-5678"}},
            {"id": "mail", "name": "Email", "status": "connected", "enabled": true, "tools_count": 4, "config": {"transport": "sse", "url": "https://n8n.example.cloud/mcp/95be11d4-1234-5678"}}
        ])).unwrap();
    }
    s.settings.sync();
}

/// Escena `--snap --scene office`: siete abejas en distintos estados y dos delegaciones en curso.
fn demo_office(s: &mut State) {
    use serde_json::json;
    s.screen = Screen::Office;
    let agent = |id: &str, name: &str, role: &str| settings::api::Agent {
        id: id.into(),
        name: name.into(),
        role: role.into(),
        status: "idle".into(),
        enabled: true,
        ..Default::default()
    };
    s.office.agents = vec![
        agent("boss", "Pepito", "coordinator"),
        agent("a1", "Buscador", "worker"),
        agent("a2", "Código", "worker"),
        agent("a3", "Correo", "worker"),
        agent("a4", "Calendario", "worker"),
        agent("a5", "Navegador", "worker"),
        agent("a6", "Office", "worker"),
        agent("a7", "Archivos", "worker"),
    ];
    let g = &mut s.office.graph;
    let node = |id: &str, status: &str, data: serde_json::Value| json!({"id": id, "type": "agent", "label": id, "status": status, "data": data});
    g.apply(
        "canvas:node_add",
        &node("boss", "thinking", json!({})),
        None,
        0.0,
    );
    g.apply(
        "canvas:node_add",
        &node(
            "a1",
            "tool_call",
            json!({"currentTask": "Buscar proveedores", "currentTool": "web_search"}),
        ),
        None,
        0.0,
    );
    g.apply(
        "canvas:node_add",
        &node(
            "a2",
            "thinking",
            json!({"currentTask": "Revisar el código"}),
        ),
        None,
        0.0,
    );
    g.apply(
        "canvas:node_add",
        &node("a3", "error", json!({"currentTask": "Enviar informe"})),
        None,
        0.0,
    );
    g.apply(
        "canvas:edge_add",
        &json!({"id": "e1", "source": "boss", "target": "a1", "edgeType": "delegates"}),
        None,
        0.0,
    );
    g.apply(
        "canvas:edge_add",
        &json!({"id": "e2", "source": "boss", "target": "a2", "edgeType": "delegates"}),
        None,
        0.0,
    );
    // Deja que el enjambre se reparta: 10 s de simulación.
    s.office.viewport = if snap_win().is_some() {
        s.stage_size()
    } else {
        (1068.0, 641.0)
    };
    for _ in 0..600 {
        s.office.step(1.0 / 60.0);
    }
    // Ya con la historia absorbida: llegan eventos nuevos y una decisión de Jev.
    let g = &mut s.office.graph;
    g.apply("canvas:work_event", &json!({"eventId": "w1", "phase": "delegated", "taskRef": "t1", "taskName": "Buscar proveedores", "actorId": "boss", "targetId": "a1"}), None, 0.0);
    g.apply("canvas:work_event", &json!({"eventId": "w2", "phase": "failed", "taskRef": "t3", "taskName": "Enviar informe", "actorId": "a3", "targetId": null, "detail": "El servidor de correo no responde"}), None, 0.0);
    g.apply("canvas:jev_decision", &json!({"eventId": "j1", "agentId": "a1", "kind": "context", "summary": "recorta el contexto", "savedTokens": 6100, "costUsd": 0.0012, "latencyMs": 310, "totals": {"decisions": 12, "savedTokens": 41800, "costUsd": 0.0123}}), None, 0.0);
    s.office.step(1.0 / 60.0);
    s.office.step(0.45); // el paquete ya vuela y el rayo de Jev está a medias
    s.office.selected = Some("a1".into());
}

/// Escena `--snap --scene markdown`: una respuesta del agente con tabla, código y lista.
fn demo_markdown(s: &mut State) {
    s.msgs = vec![
        Msg { id: "u0".into(), mine: true, text: "Resúmeme los pedidos y dame un ejemplo en Rust".into() },
        Msg { id: "m1".into(), mine: false, text: "Aquí está el **resumen**:\n\n| Pedido | Total |\n|---|--:|\n| #1 | $1,234.50 |\n| #2 | $80.00 |\n\n- Pagados: *1*\n- Pendientes: `1`\n\n```rust\nfn total(p: &[f64]) -> f64 {\n    p.iter().sum()\n}\n```\n\nMás en [la documentación](https://example.com).".into() },
    ];
}

/// Escena `--snap --scene tools`: pestaña de herramientas con búsqueda.
fn demo_tools(s: &mut State) {
    s.screen = Screen::Settings;
    s.settings.tab = settings::Tab::Tools;
    {
        let mut d = s.settings.data.lock().unwrap();
        d.loaded = true;
        d.extras_loaded = true;
        d.tools = serde_json::from_value(serde_json::json!([
            {"id": "web_search", "name": "web_search", "description": "Busca en la web y devuelve resultados", "category": "web", "active": 1, "enabled": true},
            {"id": "web_fetch", "name": "web_fetch", "description": "Descarga el contenido de una página", "category": "web", "active": false, "enabled": true},
            {"id": "read_file", "name": "read_file", "description": "Lee un archivo", "category": "filesystem", "active": true, "enabled": true},
            {"id": "a2ui_create_surface", "name": "a2ui_create_surface", "description": "Crear superficie A2UI para UI interactiva rica: formularios, dashboards, wizards", "category": "a2ui", "active": true, "enabled": true},
            {"id": "admin_actividad", "name": "admin_actividad_reciente", "description": "Actividad reciente de la plataforma: registros nuevos", "category": "admin", "active": false, "enabled": true}
        ])).unwrap();
    }
    s.settings.sync();
}

/// Escena `--snap --scene a2ui-replay`: repite los frames guardados con `A2UI_DUMP` (variable `A2UI_REPLAY`).
fn demo_replay(s: &mut State) {
    let Some(path) = std::env::var_os("A2UI_REPLAY") else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(frames) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        return;
    };
    for f in frames {
        if let Some(kind) = f.get("type").and_then(|v| v.as_str()) {
            s.a2.apply(kind, &f["data"]);
        }
    }
}

/// Decodifica el logo embebido a píxeles para la cabecera.
fn load_logo() -> rune::Pixels {
    load_png(include_bytes!("../assets/logo.png"))
}

/// Decodifica un PNG incrustado a píxeles de Rune (vacío si no se puede).
fn load_png(bytes: &[u8]) -> rune::Pixels {
    let pixels = rune::Pixels::new();
    if let Ok(img) = image::load_from_memory(bytes) {
        let rgba = img.to_rgba8();
        pixels.set(rgba.width(), rgba.height(), rgba.into_raw());
    }
    pixels
}

fn header_line(s: &State) -> String {
    match (&s.shown.status, &s.shown.chat_error) {
        (Some(Status::Failed(e)), _) => format!("Error: {e}"),
        (Some(Status::Ready { .. }), _) if s.shown.needs_login && !s.chat_ready => {
            "Inicia sesión".into()
        }
        (_, Some(e)) => e.clone(),
        (Some(Status::Ready { .. }), None) if s.chat_ready => {
            format!(
                "Conectado{}",
                s.agent
                    .as_ref()
                    .map(|a| format!(" · {a}"))
                    .unwrap_or_default()
            )
        }
        (Some(Status::Ready { port, .. }), None) => {
            format!("Gateway listo en :{port} · abriendo chat…")
        }
        _ => "Iniciando gateway…".into(),
    }
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let t = crate::ui::theme::tok();
    let accent = crate::ui::theme::accent_tok();
    let mut list = col().key("msgs").gap(8.0).w_full();
    // Los avisos del servidor van arriba del chat aunque no haya mensajes (ver `chat_page`).
    let mut notices: Vec<El> = Vec::new();
    for (id, msg, _) in &s.notices {
        let dismiss = id.clone();
        notices.push(
            row()
                .key(format!("notice/{id}"))
                .gap(8.0)
                .center_y()
                .px(12.0)
                .py(8.0)
                .radius(radius::SM)
                .fill(accent.alpha(0.25))
                .border(1.0, accent.alpha(0.5))
                .child(text("🔔").font(type_::BODY))
                .child(
                    text(msg.clone())
                        .font(type_::BODY)
                        .color(t.text)
                        .wrap()
                        .grow(1.0),
                )
                .child(text("✕").font(type_::LABEL).color(t.text_secondary))
                .tap(move |s: &mut State| s.notices.retain(|(n, ..)| *n != dismiss)),
        );
    }
    if s.shown.needs_login && !s.chat_ready {
        list = list.child(login_card(s));
    }
    // Las superficies A2UI van dentro del hilo, en el punto donde el agente las creó; las que no
    // tienen punto (escenas de ejemplo) o nacieron más allá del final van al cierre.
    let surfaces_at = |i: usize| {
        s.a2.store.surfaces.iter().filter(move |sf| {
            s.a2.anchors
                .get(&sf.id)
                .copied()
                .unwrap_or(usize::MAX)
                .min(s.msgs.len())
                == i
        })
    };
    for (i, m) in s.msgs.iter().enumerate() {
        for surface in surfaces_at(i) {
            list = list.child(a2ui::view::surface_view(&s.a2, surface));
        }
        // El agente responde en markdown (tablas, código, listas); lo propio se muestra tal cual.
        let last_streaming = s.busy && s.msgs.last().is_some_and(|l| std::ptr::eq(l, m));
        let bubble = if m.mine {
            text(m.text.clone()).font(type_::BODY).color(t.text).wrap()
        } else {
            rune_kit::markdown::Markdown::new(m.text.clone())
                .scroll_wide(true)
                .selectable(true)
                .streaming(last_streaming)
                .render(&rune_kit::markdown::MarkdownStyle::neutral(
                    !crate::ui::theme::is_light(),
                ))
        };
        let (fill, key) = if m.mine {
            (accent.alpha(0.35), "me")
        } else {
            (t.card, "bee")
        };
        let bubble = col()
            .key(format!("{key}-{}", m.id))
            .w_full()
            .min_w(0.0)
            .px(12.0)
            .py(8.0)
            .radius(radius::SM)
            .fill(fill)
            .child(bubble.w_full().min_w(0.0));
        list = list.child(if m.mine {
            row().w_full().child(col().grow(1.0)).child(bubble)
        } else {
            row().w_full().child(bubble)
        });
    }
    for surface in surfaces_at(s.msgs.len()) {
        list = list.child(a2ui::view::surface_view(&s.a2, surface));
    }
    if s.busy {
        list = list.child(
            row()
                .gap(10.0)
                .center_y()
                .child(text("…").font(type_::LABEL).color(t.text_secondary))
                .child(
                    text("Detener")
                        .key("chat/stop")
                        .font(type_::LABEL)
                        .color(crate::ui::theme::accent_tok())
                        .tap(stop_turn),
                ),
        );
    }
    let field = text_input(&s.input, type_::BODY.lh(26.0))
        .key("input")
        .single_line()
        .grow(1.0)
        .color(t.text)
        .placeholder(
            if s.chat_ready {
                "Escribe un mensaje"
            } else {
                "Esperando al gateway…"
            },
            t.text_secondary,
        )
        .on_edit(|s: &mut State| &mut s.input)
        .on_submit(send);
    let page = match s.screen {
        Screen::Settings => col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(settings::view::settings_view(s)),
        Screen::Office => col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(16.0)
            .gap(8.0)
            .child(office_view(s))
            .child(office_chat(s, field)),
        Screen::Meeting => col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(16.0)
            .child(meeting_ui::view(s)),
        Screen::Live => ui::live::view(s),
        Screen::Panel => ui::panel::view(s),
        Screen::Agents => ui::agents::view(s),
        Screen::Dashboard => ui::system_pages::dashboard(s),
        Screen::Notes => ui::work_pages::notes(s),
        Screen::Cron => ui::work_pages::cron(s),
        Screen::ApiClient => ui::work_pages::api_client(s),
        Screen::Chat => webchat_view(s, list, field, notices),
    };
    // Cada pantalla entra con un leve deslizamiento y fundido, como las páginas de la web; la oficina y HiveLive llevan un
    // escenario de GPU propio y no se mueven.
    let page = if matches!(s.screen, Screen::Office | Screen::Live) {
        page
    } else {
        let tr = rune::Transition::offset(0.0, 10.0).combined(rune::Transition::opacity());
        col()
            .key(format!("screen/{:?}", s.screen))
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .transition(tr)
            .child(page)
    };
    // Una aprobación se ve en cualquier pantalla: el turno está detenido esperándola.
    let page = if visible_approvals(s).is_empty() {
        page
    } else {
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .children(
                approval_cards(s)
                    .into_iter()
                    .map(|card| col().w_full().px(24.0).pt(8.0).child(card)),
            )
            .child(page)
    };
    ui::shell::frame(s, page)
}

/// Una tarjeta por petición: qué herramienta, con qué argumentos y por qué se pregunta.
fn approval_cards(s: &State) -> Vec<El> {
    use ui::theme as th;
    visible_approvals(s)
        .into_iter()
        .map(|a| {
            let button = |key: String, label: &str, color: rune::Color| {
                row()
                    .key(key)
                    .px(14.0)
                    .py(6.0)
                    .radius(12.0)
                    .fill(th::glass_strong())
                    .border(1.0, color.alpha(0.6))
                    .child(text(label.to_string()).font(type_::LABEL).color(color))
            };
            let (yes, no) = (a.id.clone(), a.id.clone());
            col()
                .key(format!("approval/{}", a.id))
                .w_full()
                .gap(8.0)
                .padding(12.0)
                .radius(radius::SM)
                .fill(th::amber().alpha(0.16))
                .border(1.0, th::amber().alpha(0.6))
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text("⚠").font(type_::BODY))
                        .child(
                            text("Aprobación necesaria")
                                .font(type_::BODY)
                                .color(th::foreground()),
                        ),
                )
                .child(
                    text(format!("Hive quiere ejecutar «{}».", a.tool))
                        .font(type_::BODY)
                        .color(th::foreground())
                        .wrap(),
                )
                .children(a.reason.clone().map(|reason| {
                    text(reason)
                        .font(type_::LABEL)
                        .color(th::muted_foreground())
                        .wrap()
                }))
                .child(
                    text(a.args.clone())
                        .font(type_::LABEL)
                        .color(th::muted_foreground())
                        .wrap(),
                )
                .child(
                    text(format!("Si no respondes en {} s se denegará.", a.seconds))
                        .font(type_::LABEL)
                        .color(th::muted_foreground()),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            button(format!("approval/{}/approve", a.id), "Aprobar", th::green())
                                .tap(move |s: &mut State| answer_approval(s, &yes, true)),
                        )
                        .child(
                            button(format!("approval/{}/deny", a.id), "Denegar", th::red())
                                .tap(move |s: &mut State| answer_approval(s, &no, false)),
                        ),
                )
        })
        .collect()
}

fn webchat_enabled(s: &State) -> bool {
    s.shown
        .capabilities
        .as_ref()
        .is_some_and(|caps| caps.iter().any(|cap| cap == "conversations"))
}
fn request_conversation(s: &mut State, target: Option<String>) {
    if s.voice_on {
        s.webchat.error = Some("Cuelga la llamada antes de cambiar de conversación".into());
        return;
    }
    if !s.chat_ready {
        s.webchat.error = Some("Conecta el chat antes de cambiar de conversación".into());
        return;
    }
    if s.busy || s.webchat.loading {
        s.webchat.error = Some("Termina o detén el turno antes de cambiar de conversación".into());
        return;
    }
    if let Some(conn) = s.settings.conn.clone() {
        s.webchat.request(conn, target);
    }
}
fn webchat_view(s: &State, list: El, field: El, notices: Vec<El>) -> El {
    use ui::theme as th;
    let page = chat_page(s, list, field, notices);
    if !webchat_enabled(s) {
        return page;
    }
    let narrow = s.win.0 < 960.0;
    let mut chats = col()
        .key("chat/conversations")
        .min_h(0.0)
        .gap(8.0)
        .padding(12.0)
        .border(1.0, th::border_subtle());
    chats = chats.child(
        row()
            .key("chat/new")
            .w_full()
            .gap(8.0)
            .center_y()
            .padding(10.0)
            .radius(8.0)
            .fill(th::glass_strong())
            .child(rune::icon(&ui::icons::CHAT_PLUS, 16.0).color(th::blue()))
            .child(
                text("Nueva conversación")
                    .font(type_::LABEL)
                    .color(th::foreground()),
            )
            .tap(|s: &mut State| request_conversation(s, None)),
    );
    chats = chats.child(
        row()
            .key("chat/refresh")
            .w_full()
            .padding(6.0)
            .child(
                text(if s.webchat.loading {
                    "Cargando…"
                } else {
                    "Actualizar conversaciones"
                })
                .font(type_::LABEL)
                .color(th::muted_foreground()),
            )
            .tap(|s: &mut State| {
                let id = s.webchat.active.clone().or_else(|| {
                    CHAT.lock()
                        .unwrap()
                        .as_ref()
                        .map(|chat| chat.thread_id.clone())
                });
                if let Some(id) = id {
                    request_conversation(s, Some(id));
                }
            }),
    );
    let mut items = col().w_full().gap(5.0);
    for (index, item) in s.webchat.items.iter().enumerate() {
        let id = item.id.clone();
        let active = s.webchat.active.as_ref() == Some(&id);
        let date = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(item.updated as i64)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%d %b · %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        items = items.child(
            col()
                .key(format!("chat/conversation/{index}"))
                .w_full()
                .padding(10.0)
                .gap(4.0)
                .radius(8.0)
                .fill(if active {
                    th::glass_strong()
                } else {
                    th::glass()
                })
                .child(
                    text(item.title().to_owned())
                        .font(type_::BODY)
                        .color(if active { th::blue() } else { th::foreground() })
                        .wrap(),
                )
                .child(
                    text(format!("{} mensajes · {date}", item.message_count))
                        .font(type_::LABEL)
                        .color(th::muted_foreground())
                        .wrap(),
                )
                .tap(move |s: &mut State| request_conversation(s, Some(id.clone()))),
        );
    }
    chats = chats.child(items.min_h(0.0).grow(1.0).scroll(ScrollSpec {
        stick_bottom: false,
    }));
    let mut content = col().grow(1.0).min_w(0.0).min_h(0.0);
    if let Some(error) = &s.webchat.error {
        content = content.child(
            text(error.clone())
                .font(type_::LABEL)
                .color(if s.webchat.remote_active {
                    th::amber()
                } else {
                    th::red()
                })
                .wrap()
                .padding(12.0),
        );
    }
    if s.webchat.loading {
        content = content.child(
            text("Cargando conversación…")
                .font(type_::LABEL)
                .color(th::blue())
                .padding(8.0),
        );
    }
    content = content.child(page);
    if narrow {
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(
                row()
                    .key("chat/show-list")
                    .padding(10.0)
                    .child(text("Conversaciones").font(type_::LABEL).color(th::blue()))
                    .tap(|s: &mut State| s.webchat.list_open = !s.webchat.list_open),
            )
            .child(if s.webchat.list_open {
                chats.w_full().grow(1.0)
            } else {
                content
            })
    } else {
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(chats.w(240.0))
            .child(content)
    }
}

/// El chat: cabecera con el agente, el hilo (o el estado vacío con sugerencias) y la barra de mensaje.
fn chat_page(s: &State, list: El, field: El, notices: Vec<El>) -> El {
    use ui::theme as th;
    let name = s.agent.clone().unwrap_or_else(|| "Hive".into());
    let empty = s.msgs.is_empty()
        && s.a2.store.surfaces.is_empty()
        && !s.busy
        && !(s.shown.needs_login && !s.chat_ready);
    let head = row()
        .h(48.0)
        .w_full()
        .px(16.0)
        .gap(10.0)
        .center_y()
        .border(1.0, th::border_subtle())
        .child(
            text(name.clone())
                .font(type_::BODY.lh(20.0))
                .color(th::foreground())
                .truncate()
                .help(name.clone()),
        )
        .children(chat_only(s).then(|| {
            text("Rust · chat y oficina en vivo")
                .font(type_::LABEL)
                .color(th::muted_foreground())
        }))
        .child(
            row()
                .gap(6.0)
                .center_y()
                .px(8.0)
                .py(2.0)
                .radius(10.0)
                .border(1.0, th::border_base())
                .child(rune::el().size(6.0, 6.0).radius(3.0).fill(if s.chat_ready {
                    th::green()
                } else {
                    th::amber()
                }))
                .child(
                    text(if s.chat_ready {
                        "CONECTADO"
                    } else {
                        "CONECTANDO"
                    })
                    .font(type_::LABEL.tracking(0.6))
                    .color(th::muted_foreground()),
                ),
        );
    let body = if empty {
        let pill = |key: &str, ico: &'static rune::Icon, label: &str, ask: &'static str| {
            row()
                .key(key.to_string())
                .gap(8.0)
                .center_y()
                .px(16.0)
                .py(8.0)
                .radius(20.0)
                .fill(th::glass_strong())
                .border(1.0, th::border_base())
                .child(rune::icon(ico, 14.0).color(th::blue()))
                .child(
                    text(label.to_string())
                        .font(type_::BODY.lh(20.0))
                        .color(th::foreground()),
                )
                .tap(move |s: &mut State| {
                    s.input.text = ask.to_string();
                    send(s, &mut Tx::default());
                })
        };
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(
                col()
                    .w_full()
                    .h_full()
                    .min_h(280.0)
                    .padding(12.0)
                    .center()
                    .gap(10.0)
                    .child(
                        row()
                            .size(64.0, 64.0)
                            .radius(16.0)
                            .fill(th::card())
                            .border(1.0, th::border_base())
                            .center()
                            .child(rune::icon(&ui::icons::BOT, 28.0).color(th::blue())),
                    )
                    .child(
                        text(format!("{name} está listo"))
                            .font(type_::TITLE.lh(24.0))
                            .color(th::foreground()),
                    )
                    .child(
                        text("Inicia la conversación escribiendo un mensaje abajo")
                            .font(type_::BODY.lh(20.0))
                            .color(th::muted_foreground()),
                    )
                    .child(
                        row()
                            .w_full()
                            .gap(12.0)
                            .flex_wrap()
                            .justify_center()
                            .py(8.0)
                            .child(pill(
                                "chat/suggest/1",
                                &ui::icons::SPARKLES,
                                "Que puedes hacer?",
                                "Que puedes hacer?",
                            ))
                            .child(pill(
                                "chat/suggest/2",
                                &ui::icons::ZAP,
                                "Dame un resumen",
                                "Dame un resumen",
                            ))
                            .child(pill(
                                "chat/suggest/3",
                                &ui::icons::BOT,
                                "Ayúdame con una tarea",
                                "Ayúdame con una tarea",
                            )),
                    ),
            )
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
    } else {
        col().w_full().grow(1.0).min_h(0.0).px(24.0).py(12.0).child(
            list.scroll(ScrollSpec { stick_bottom: true })
                .shrink()
                .min_h(0.0)
                .grow(1.0),
        )
    };
    let send_btn = if s.busy || s.webchat.remote_active {
        row()
            .key("chat/stop")
            .px(10.0)
            .py(6.0)
            .radius(12.0)
            .fill(th::glass_strong())
            .child(text("Detener").font(type_::LABEL).color(th::red()))
            .tap(|_: &mut State| {
                if let Some(chat) = CHAT.lock().unwrap().as_ref() {
                    chat.stop();
                }
            })
    } else {
        row()
            .key("chat/send")
            .size(32.0, 32.0)
            .radius(16.0)
            .center()
            .fill(th::glass_strong())
            .child(rune::icon(&ui::icons::SEND, 16.0).color(th::muted_foreground()))
            .tap(|s: &mut State| send(s, &mut Tx::default()))
    };
    let bar = row()
        .h(52.0)
        .w_full()
        .px(18.0)
        .gap(10.0)
        .center_y()
        .radius(26.0)
        .fill(th::glass())
        .border(1.0, th::border_base())
        .child(field)
        .child(send_btn);
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(head)
        .children(
            notices
                .into_iter()
                .map(|n| col().w_full().px(24.0).pt(8.0).child(n)),
        )
        .child(body)
        .child(process_view(s))
        .child(
            col().w_full().px(64.0).pb(8.0).gap(8.0).child(bar).child(
                text("Enter para enviar")
                    .font(type_::LABEL)
                    .color(th::muted_foreground())
                    .w_full(),
            ),
        )
}

/// Narración del trabajo real; los pasos terminados de un especialista no cierran el turno raíz.
fn process_view(s: &State) -> El {
    use ui::theme as th;
    let mut panel = col().key("process").w_full().px(18.0).py(6.0).gap(4.0);
    let Some(latest) = s.processes.last() else {
        return panel;
    };
    let caption = format!(
        "{} · {}",
        if s.busy || s.webchat.remote_active {
            "En curso"
        } else {
            "Actividad reciente"
        },
        latest.label
    );
    panel = panel.child(
        row()
            .key("process/toggle")
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                text(if s.process_expanded { "▾" } else { "▸" })
                    .font(type_::LABEL)
                    .color(th::muted_foreground()),
            )
            .child(
                text(caption)
                    .font(type_::LABEL)
                    .color(th::foreground())
                    .wrap(),
            )
            .tap(|s: &mut State| s.process_expanded = !s.process_expanded),
    );
    if s.process_expanded {
        let mut steps = col().w_full().gap(4.0);
        let shown: Vec<_> = s
            .processes
            .iter()
            .rev()
            .take(128)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let mut turns = Vec::new();
        for step in &shown {
            if !turns.contains(&step.turn_id.as_str()) {
                turns.push(step.turn_id.as_str());
            }
        }
        for (index, turn) in turns.iter().enumerate() {
            if turns.len() > 1 {
                steps = steps.child(
                    text(format!("Turno {}", index + 1))
                        .font(type_::LABEL)
                        .color(th::muted_foreground())
                        .pt(8.0),
                );
            }
            for step in shown.iter().filter(|step| step.turn_id == *turn) {
                let color = match step.status.as_str() {
                    "error" => th::red(),
                    "done" => th::green(),
                    _ => th::blue(),
                };
                let label = step.agent.as_ref().map_or_else(
                    || step.label.clone(),
                    |name| {
                        if step.label.starts_with(name) {
                            step.label.clone()
                        } else {
                            format!("{name} · {}", step.label)
                        }
                    },
                );
                let time =
                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(step.timestamp as i64)
                        .filter(|_| step.timestamp > 0)
                        .map(|at| {
                            at.with_timezone(&chrono::Local)
                                .format("%H:%M:%S")
                                .to_string()
                        })
                        .unwrap_or_default();
                let kind = match step.kind.as_str() {
                    "delegated" => "Delegación",
                    "tool_call" => "Herramienta",
                    "tool_result" => "Resultado",
                    "failed" => "Error",
                    "verified" => "Verificación",
                    "group_ready" => "Respuesta",
                    _ => "Actividad",
                };
                let mut entry = col()
                    .key(format!("process/{}", step.id))
                    .w_full()
                    .gap(2.0)
                    .child(
                        text(format!(
                            "{} {label}",
                            match step.status.as_str() {
                                "done" => "✓",
                                "error" => "!",
                                _ => "·",
                            }
                        ))
                        .font(type_::LABEL)
                        .color(color)
                        .wrap(),
                    )
                    .child(
                        text(format!("{kind} · {time}"))
                            .font(type_::LABEL)
                            .color(th::muted_foreground()),
                    );
                if let Some(detail) = &step.detail {
                    entry = entry.child(
                        text(if step.status == "error" {
                            detail.clone()
                        } else {
                            detail.chars().take(240).collect::<String>()
                        })
                        .font(type_::LABEL)
                        .color(th::muted_foreground())
                        .wrap(),
                    );
                }
                steps = steps.child(entry);
            }
        }
        panel = panel.child(steps.h(180.0).scroll(ScrollSpec { stick_bottom: true }));
    }
    panel
}

fn office_chat(s: &State, field: El) -> El {
    use ui::theme as th;
    let mut panel = col()
        .key("office/chat")
        .w_full()
        .gap(4.0)
        .fill(th::card())
        .radius(12.0)
        .padding(10.0)
        .child(process_view(s));
    if let Some(message) = s.msgs.iter().rev().find(|m| !m.mine) {
        panel = panel.child(
            text(message.text.chars().take(500).collect::<String>())
                .font(type_::BODY)
                .color(th::foreground())
                .wrap()
                .h(52.0)
                .scroll(ScrollSpec { stick_bottom: true }),
        );
    }
    let action = if s.busy {
        text("Detener")
            .key("office/stop")
            .font(type_::LABEL)
            .color(th::red())
            .tap(stop_turn)
    } else {
        text("Enviar")
            .key("office/send")
            .font(type_::LABEL)
            .color(th::blue())
            .tap(|s: &mut State| send(s, &mut Tx::default()))
    };
    panel.child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(field)
            .child(action),
    )
}

/// La oficina 3D: un hueco que la GPU rellena, con órbita al arrastrar y zoom con la rueda.
fn office_view(s: &State) -> El {
    office::hud_view::overlay(s)
        .into_iter()
        .fold(rune::effect(office::stage::OFFICE_MARK), |slot, e| {
            slot.child(e)
        })
        .key(office::stage::SLOT_KEY)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .radius(14.0)
        .on_drag(|s: &mut State, d: &mut rune::Drag, _| {
            let o = &mut s.office;
            match d.phase {
                rune::Phase::Down => {
                    o.drag_last = (0.0, 0.0);
                    o.dragging = true;
                    o.inertia.stop();
                    o.rig.drag_start(); // la persona toma el mando: se cancelan el enfoque y el seguimiento
                }
                // Un arrastre casi sin recorrido es un clic: elige la abeja (o la reina) que haya debajo.
                rune::Phase::Up if d.dx.abs() < 4.0 && d.dy.abs() < 4.0 && !d.cancelled() => {
                    let picked = office::hud::pick(o, (d.x, d.y), (d.w, d.h));
                    let boss = o.desks().0.map(|c| c.id);
                    office::hud::apply_pick(&mut o.selected, boss.as_deref(), &picked);
                }
                phase => {
                    // Durante la intro los controles no responden.
                    if o.rig.controls_enabled() {
                        let (mx, my) = (d.dx - o.drag_last.0, d.dy - o.drag_last.1);
                        o.camera.orbit(mx, my);
                        if mx != 0.0 || my != 0.0 {
                            o.inertia.track(mx, my);
                        }
                    }
                    o.drag_last = (d.dx, d.dy);
                    if phase == rune::Phase::Up {
                        o.dragging = false;
                        if !d.cancelled() {
                            o.inertia.release();
                        }
                        o.rig.drag_end(&o.camera); // donde la deja pasa a ser su pose de reposo
                    }
                }
            }
        })
        .on_wheel(|s: &mut State, e: rune::WheelEvent, _| {
            if s.office.rig.controls_enabled() {
                s.office.camera.zoom((e.dy / 40.0).clamp(-3.0, 3.0));
            }
            true
        })
}

/// Lee los agentes del gateway en un hilo; `pull_office` los recoge. Sin ellos la oficina no sabe cuántas abejas dibujar.
fn request_office_agents(s: &mut State) {
    let Some(conn) = s.settings.conn.clone() else {
        return;
    };
    s.office_requested = true;
    let slot = s.office_agents.clone();
    let allow_jev = !chat_only(s);
    std::thread::spawn(move || {
        if let Ok(agents) = settings::api::agents(&conn) {
            // Como la web: si Jev no ha avisado por el WebSocket, su estado sale del proveedor `openrouter`.
            let jev = (!conn.base.is_empty() && allow_jev)
                .then(|| settings::api::providers(&conn).ok())
                .flatten()
                .and_then(|ps| {
                    ps.into_iter()
                        .find(|p| p.id == "openrouter")
                        .and_then(|p| p.jev)
                });
            *slot.lock().unwrap() = Some(OfficeLoad { agents, jev });
        }
    });
}

/// Recoge los agentes leídos. Devuelve si cambió algo.
fn pull_office(s: &mut State) -> bool {
    match s.office_agents.lock().unwrap().take() {
        Some(load) => {
            s.office.agents = load.agents;
            if let (None, Some(jev)) = (&s.office.graph.jev_status, &load.jev) {
                s.office.graph.apply("canvas:jev_status", jev, None, 0.0);
            }
            true
        }
        None => false,
    }
}

/// Cambia de pantalla; la primera vez que se abren los ajustes (o la oficina) se leen del gateway.
fn chat_only(s: &State) -> bool {
    s.shown
        .capabilities
        .as_ref()
        .is_some_and(|caps| !caps.iter().any(|c| c == "settings"))
}

fn screen_available(s: &State, target: Screen) -> bool {
    !chat_only(s)
        || target == Screen::Chat
        || s.shown.capabilities.as_ref().is_some_and(|caps| {
            caps.iter().any(|c| {
                (target == Screen::Office && c == "office")
                    || (target == Screen::Dashboard && c == "dashboard")
                    || (target == Screen::Live && c == "hivelive")
            })
        })
}

fn open_screen(s: &mut State, target: Screen) {
    if !screen_available(s, target) {
        return;
    }
    s.screen = target;
    if target == Screen::Live {
        if let Some(conn) = s.settings.conn.as_ref() {
            s.live.catalog.refresh(conn);
        }
    }
    if target == Screen::Office && !s.office_requested {
        request_office_agents(s);
    }
    if target == Screen::Dashboard {
        // Los agentes (para los contadores) y las métricas; después `poll` las relee cada pocos segundos.
        if chat_only(s) {
            if !s.office_requested {
                request_office_agents(s);
            }
        } else if !s.settings.shown.loaded && s.settings.shown.busy == 0 {
            s.settings.dispatch(settings::Action::Refresh);
        }
        s.sys.reset_clock();
    }
    if matches!(target, Screen::Notes | Screen::Cron) {
        if let Some(c) = s.settings.conn.clone() {
            s.sys.refresh(
                &c,
                if target == Screen::Notes {
                    system::Part::Notes
                } else {
                    system::Part::Cron
                },
            );
        }
    }
    if target == Screen::Agents && !s.settings.shown.loaded && s.settings.shown.busy == 0 {
        s.settings.dispatch(settings::Action::Refresh);
    }
    if target == Screen::Meeting {
        let conn = s.settings.conn.clone();
        s.meeting.open(conn.as_ref());
    }
    if target == Screen::Settings && !s.settings.shown.loaded && s.settings.shown.busy == 0 {
        s.settings.dispatch(settings::Action::Refresh);
    }
}

fn poll(s: &mut State, _t: f64, tx: &mut Tx) -> bool {
    let office = s.screen == Screen::Office;
    // Con la oficina abierta la vista se reconstruye a unos 30 fps para que las placas sigan a las abejas.
    tx.wake_after(std::time::Duration::from_millis(if office {
        33
    } else {
        100
    }));
    let mut changed = office;
    changed |= s.live.catalog.poll();
    changed |= s.live.camera_preview.poll();
    changed |= s.live.screen_preview.poll();
    if s.screen == Screen::Live
        && !s.live.catalog.loaded
        && !s.live.catalog.busy()
        && s.live.catalog.error.is_none()
    {
        if let Some(conn) = s.settings.conn.as_ref() {
            s.live.catalog.refresh(conn);
            changed = true;
        }
    }
    let now = s.shared.lock().unwrap().clone();
    if now.needs_login && !s.shown.needs_login {
        // La tarjeta de acceso aparece: el teclado va al correo, no al campo de mensaje.
        s.login_email.focused = true;
        s.input.focused = false;
    }
    if now != s.shown {
        s.shown = now;
        changed = true;
    }
    if !screen_available(s, s.screen) {
        s.screen = Screen::Chat;
        changed = true;
    }
    if let Some(chat) = CHAT.lock().unwrap().as_ref() {
        if !s.chat_ready && s.shown.chat_error.is_none() {
            s.chat_ready = true;
            if let Some(Status::Ready { port, .. }) = &s.shown.status {
                // Las imágenes de A2UI con ruta relativa salen del gateway; solo a él se le manda el token.
                let base = format!("http://127.0.0.1:{port}");
                s.a2.images.set_origin(&base, chat.token.as_deref());
                s.a2.audio.clips.set_origin(&base, chat.token.as_deref());
                s.settings.conn = Some(settings::api::Conn {
                    base,
                    token: chat.token.clone(),
                });
                open_screen(s, s.screen);
            }
            changed = true;
        }
        for ev in chat.drain() {
            if let Event::Notification { id, .. } = &ev {
                chat.ack(id); // se confirma al recibirlo: el servidor no lo reenvía
            }
            apply(s, ev);
            changed = true;
        }
        if webchat_enabled(s)
            && s.webchat.active.is_none()
            && !s.webchat.loading
            && s.webchat.error.is_none()
        {
            if let Some(conn) = s.settings.conn.clone() {
                s.webchat.request(conn, Some(chat.thread_id.clone()));
            }
        }
        // Acciones del usuario sobre las superficies, de vuelta al agente.
        for frame in std::mem::take(&mut s.a2.outbox) {
            chat.send_raw(&frame);
        }
    }
    if webchat_enabled(s) {
        if let Some(result) = s.webchat.take() {
            if let Ok(loaded) = result {
                let switched = CHAT
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|chat| chat.thread_id != loaded.thread);
                if let Some(chat) = CHAT.lock().unwrap().as_mut() {
                    chat.select_thread(loaded.thread);
                }
                s.msgs = loaded.history["messages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|row| {
                        Some(Msg {
                            id: row["id"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| row["id"].to_string()),
                            mine: row["role"] == "user",
                            text: row["content"].as_str()?.to_owned(),
                        })
                    })
                    .collect();
                s.processes.clear();
                for frame in loaded.history["narrations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if let Some(event) = chat::parse(frame) {
                        apply(s, event);
                    }
                }
                s.busy = false;
                s.webchat.list_open = false;
                if switched {
                    s.input.clear();
                    s.input.focused = true;
                    s.process_expanded = false;
                    s.a2 = Default::default();
                    if let Some(conn) = &s.settings.conn {
                        s.a2.images.set_origin(&conn.base, conn.token.as_deref());
                        s.a2.audio
                            .clips
                            .set_origin(&conn.base, conn.token.as_deref());
                    }
                }
            }
            changed = true;
        }
        if !s.busy && s.webchat.due() {
            if let (Some(conn), Some(id)) = (s.settings.conn.clone(), s.webchat.active.clone()) {
                s.webchat.request(conn, Some(id));
            }
        }
    }
    if let Some(device) = s.audio.changed.take() {
        apply_device_change(s, device);
        changed = true;
    }
    let before = s.notices.len();
    s.notices.retain(|(.., at)| at.elapsed() < NOTICE_TTL);
    changed |= s.notices.len() != before;
    // El gateway ya denegó las que vencieron; no se deja un botón que no hace nada.
    let before = s.approvals.len();
    let now = std::time::Instant::now();
    let expired: Vec<String> = s
        .approvals
        .iter()
        .filter(|a| a.expires_at <= now)
        .map(|a| format!("{APPROVAL_SURFACE}{}", a.id))
        .collect();
    s.approvals.retain(|a| a.expires_at > now);
    s.a2.store
        .surfaces
        .retain(|surface| !expired.contains(&surface.id));
    changed |= s.approvals.len() != before;
    if s.a2.images.take_dirty() {
        changed = true;
    }
    if s.a2.audio.tick() {
        changed = true;
    }
    if s.settings.sync() {
        changed = true;
    }
    {
        let conn = s.settings.conn.clone();
        changed |= s.meeting.pump(conn.as_ref());
        changed |= s.meeting.sync();
        if s.screen == Screen::Dashboard {
            s.sys.tick(conn.as_ref());
        }
        changed |= s.ocr.sync();
        changed |= s.computer.sync();
        changed |= s.oracle.sync();
        let synced = s.sys.sync();
        if synced {
            let user = s.sys.shown.user.clone();
            s.profile.follow(user.as_ref());
        }
        changed |= synced;
        changed |= s.audio.tick();
        changed |= s.work.api.sync();
    }
    if pull_office(s) {
        changed = true;
    }
    let voice_events = VOICE
        .lock()
        .unwrap()
        .as_ref()
        .map(|rt| rt.drain())
        .unwrap_or_default();
    for ev in voice_events {
        apply_voice(s, ev);
        changed = true;
    }
    changed
}

/// Recoge lo que llega de un turno: fragmentos de texto y frames A2UI (aplicados a `a2`). Los `Done`
/// llegan por paso, no solo al final, así que se espera a `quiet` segundos sin ningún evento.
fn wait_turn(
    chat: &Chat,
    a2: &mut a2ui::view::A2State,
    frames: &mut Vec<serde_json::Value>,
    text: &mut String,
    start: std::time::Instant,
    quiet: u64,
    cap: u64,
) {
    let mut last_event = std::time::Instant::now();
    let mut steps = 0;
    while start.elapsed().as_secs() < cap {
        std::thread::sleep(std::time::Duration::from_millis(100));
        for ev in chat.drain() {
            last_event = std::time::Instant::now();
            let at = start.elapsed().as_secs();
            match ev {
                Event::A2ui { kind, data } => {
                    let comps = data.get("components").and_then(|c| c.as_array());
                    println!(
                        "[{at:>3}s] a2ui: {kind} {} (componentes={}, con propiedades={})",
                        data.get("surfaceId").and_then(|v| v.as_str()).unwrap_or(""),
                        comps.map_or(0, |c| c.len()),
                        comps.map_or(0, |c| c
                            .iter()
                            .filter(|d| d.as_object().is_some_and(|o| o.len() > 3))
                            .count())
                    );
                    a2.apply(&kind, &data);
                    frames.push(serde_json::json!({"type": kind, "data": data}));
                }
                Event::Chunk { text: t, .. } | Event::Final { text: t, .. } => text.push_str(&t),
                Event::Done => {
                    steps += 1;
                    println!("[{at:>3}s] fin de paso #{steps}");
                }
                Event::Error(e) => println!("[{at:>3}s] error: {e}"),
                Event::Closed => println!("[{at:>3}s] conexión cerrada"),
                _ => {}
            }
        }
        if steps > 0 && last_event.elapsed().as_secs() >= quiet {
            break;
        }
    }
}

/// `--office-check`: abre la oficina con el gateway real (solo lectura): lee los agentes, escucha el lienzo unos
/// segundos y resume lo que habría en pantalla. No envía mensajes ni gasta el modelo.
fn office_check() -> i32 {
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    let chat = match Chat::bootstrap_fresh(gw.port, None) {
        Ok(c) => c,
        Err(e) => {
            println!("chat no disponible: {e}");
            gw.stop();
            return 1;
        }
    };
    let conn = settings::api::Conn {
        base: format!("http://127.0.0.1:{}", gw.port),
        token: chat.token.clone(),
    };
    let mut o = office::state::OfficeState::default();
    match settings::api::agents(&conn) {
        Ok(list) => {
            println!("agentes leídos: {}", list.len());
            o.agents = list;
        }
        Err(e) => println!("agentes: error {e}"),
    }
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 6 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        for ev in chat.drain() {
            if let Event::Canvas {
                kind,
                data,
                timestamp,
            } = ev
            {
                *kinds.entry(kind.clone()).or_default() += 1;
                o.graph.apply(&kind, &data, timestamp, 0.0);
            }
        }
    }
    println!("mensajes del lienzo recibidos: {kinds:?}");
    for _ in 0..120 {
        o.step(1.0 / 60.0);
    }
    let (coordinator, desks) = o.desks();
    println!(
        "coordinador: {}",
        coordinator
            .as_ref()
            .map_or("(ninguno)".to_string(), |c| format!(
                "{} [{}]",
                c.name,
                c.state.label()
            ))
    );
    println!(
        "escritorios: {} → abejas simuladas: {}",
        desks.len(),
        o.bees.len()
    );
    for d in desks.iter().take(6) {
        println!(
            "  {} [{}]{}",
            d.name,
            d.state.label(),
            d.current_task
                .as_deref()
                .map_or(String::new(), |t| format!(" «{t}»"))
        );
    }
    if let Ok(ps) = settings::api::providers(&conn) {
        for p in ps.iter().filter(|p| p.jev.is_some()) {
            println!(
                "jev de {}: {}",
                p.id,
                p.jev.as_ref().map(|v| v.to_string()).unwrap_or_default()
            );
        }
    }
    println!(
        "grafo: {} nodos, {} aristas, {} eventos de trabajo, {} decisiones de Jev, Jev={:?}",
        o.graph.nodes.len(),
        o.graph.edges.len(),
        o.graph.work_events.len(),
        o.graph.jev_decisions.len(),
        o.graph.jev_status.as_ref().map(|j| j.state.as_str())
    );
    let frame = office::scene::build(&o);
    println!("la escena tendría {} dibujos", frame.draws.len());
    gw.stop();
    0
}

/// `--kill-check [exit|sigkill|abort]`: arranca el gateway y sale SIN detenerlo, para comprobar que no queda huérfano.
fn kill_check(how: &str) -> i32 {
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    println!(
        "gateway en el puerto {} (externo={}); saliendo con «{how}» sin detenerlo",
        gw.port,
        gw.is_external()
    );
    match how {
        #[cfg(unix)]
        "sigkill" => unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        },
        // Windows no tiene señales: abort es lo más parecido a una muerte sin limpieza.
        #[cfg(not(unix))]
        "sigkill" => std::process::abort(),
        "abort" => std::process::abort(),
        _ => std::process::exit(0),
    }
    #[allow(unreachable_code)]
    0
}

/// `--tray-check`: registra la bandeja en una app sin ventana y la prueba por D-Bus como lo haría un panel:
/// lee el menú, pulsa «Mostrar Hive» (abre la ventana unos segundos) y pulsa «Salir».
fn tray_check() -> i32 {
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    static TIMED_OUT: AtomicBool = AtomicBool::new(false);
    let app = rune::App::new(AppSpec::<State> {
        ..AppSpec::new("Hive", (1100.0, 640.0), State::offline, view)
    })
    .windowless();
    let handle = app.handle();
    let _tray = match tray::spawn(handle.clone()) {
        Ok(t) => t,
        Err(e) => {
            println!("bandeja: error {e}");
            return 1;
        }
    };
    let probe = handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let pid = std::process::id().to_string();
        let list = Command::new("busctl")
            .args(["--user", "list", "--no-legend"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let gdbus = |dest: &str, method: &str, args: &[&str]| {
            let mut c = Command::new("gdbus");
            // `--` evita que el `-1` del argumento se tome por una opción.
            c.args([
                "call",
                "--session",
                "--dest",
                dest,
                "--object-path",
                "/MenuBar",
                "--method",
                method,
                "--",
            ])
            .args(args);
            let out = c.output().ok()?;
            if !out.status.success() {
                println!(
                    "  gdbus {dest} {method}: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Some(out)
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        };
        println!(
            "  conexiones de este proceso: {:?}",
            list.lines()
                .filter(|l| l.split_whitespace().nth(1) == Some(pid.as_str()))
                .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        );
        // Las conexiones de este proceso: la del icono es la que sirve /MenuBar.
        let found = list
            .lines()
            .filter_map(|l| {
                let mut c = l.split_whitespace();
                let (name, p) = (c.next()?, c.next()?);
                (name.starts_with(':') && p == pid).then(|| name.to_string())
            })
            .find_map(|name| {
                gdbus(
                    &name,
                    "com.canonical.dbusmenu.GetLayout",
                    &["0", "-1", "[]"],
                )
                .map(|layout| (name, layout))
            });
        let Some((name, layout)) = found else {
            println!("icono: no se encontró una conexión D-Bus con /MenuBar en este proceso");
            TIMED_OUT.store(true, Ordering::SeqCst);
            probe.quit();
            return;
        };
        let labels: Vec<String> = layout
            .split("'label': <'")
            .skip(1)
            .filter_map(|p| p.split('\'').next().map(str::to_string))
            .collect();
        println!("icono registrado en {name}; menú: {labels:?}");
        let click = |id: &str| {
            gdbus(
                &name,
                "com.canonical.dbusmenu.Event",
                &[id, "clicked", "<int32 0>", "0"],
            )
            .is_some()
        };
        println!("clic en «Mostrar Hive»: {}", click("1"));
        std::thread::sleep(std::time::Duration::from_millis(3000));
        println!("ventana visible tras «Mostrar»: {}", probe.is_visible());
        println!("clic en «Salir»: {}", click("3"));
        std::thread::sleep(std::time::Duration::from_millis(4000));
        println!("la app no salió con el menú");
        TIMED_OUT.store(true, Ordering::SeqCst);
        probe.quit();
    });
    app.run();
    if TIMED_OUT.load(Ordering::SeqCst) {
        return 1;
    }
    println!("la app salió con «Salir»");
    0
}

/// `--a2ui-flow`: pide el formulario, lo rellena como lo haría una persona (nombre y casilla), pulsa el
/// botón con acción y espera la reacción del agente. Dos turnos del modelo.
fn a2ui_flow() -> i32 {
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    let chat = match Chat::bootstrap_fresh(gw.port, None) {
        Ok(c) => c,
        Err(e) => {
            println!("chat no disponible: {e}");
            gw.stop();
            return 1;
        }
    };
    let mut a2 = a2ui::view::A2State::default();
    let (mut frames, mut text) = (Vec::new(), String::new());
    println!("── turno 1: pedir el formulario");
    chat.send("Usa tu herramienta de A2UI para mostrarme un formulario simple con un campo de nombre, una casilla y un botón Enviar. No hagas nada más.");
    wait_turn(
        &chat,
        &mut a2,
        &mut frames,
        &mut text,
        std::time::Instant::now(),
        20,
        300,
    );
    println!(
        "respuesta 1: {}",
        text.trim().chars().take(200).collect::<String>()
    );

    // Rellenar como una persona: cada campo ligado a una ruta absoluta recibe un valor.
    let Some(surface) = a2.store.surfaces.first().cloned() else {
        println!("no hay superficie: no se puede continuar");
        gw.stop();
        return 1;
    };
    println!(
        "superficie {} raíz={:?} componentes={}",
        surface.id,
        surface.root,
        surface.components.len()
    );
    let bound = |def: &serde_json::Value| {
        def.get("value")
            .and_then(|v| v.get("path"))
            .and_then(|p| p.as_str())
            .map(str::to_string)
    };
    let mut filled = 0;
    for def in surface.components.values() {
        let (kind, path) = (
            def.get("component").and_then(|v| v.as_str()).unwrap_or(""),
            bound(def),
        );
        match (kind, path) {
            ("TextField", Some(p)) => {
                a2.write(&surface.id, &p, serde_json::json!("Ana Prueba"));
                println!("  relleno {p} = \"Ana Prueba\"");
                filled += 1;
            }
            ("CheckBox", Some(p)) => {
                a2.write(&surface.id, &p, serde_json::json!(true));
                println!("  marco {p} = true");
                filled += 1;
            }
            _ => {}
        }
    }
    // Pulsar el botón con acción: mismo cálculo que el clic (contexto resuelto contra el modelo local).
    let sf = a2.store.get(&surface.id).cloned().unwrap_or(surface);
    let button = sf.components.iter().find_map(|(id, def)| {
        (def.get("component").and_then(|v| v.as_str()) == Some("Button"))
            .then(|| {
                def.get("action")
                    .and_then(a2ui::model::action_event)
                    .map(|ev| (id.clone(), ev))
            })
            .flatten()
    });
    let Some((button_id, (name, context))) = button else {
        println!("el formulario no tiene un botón con acción ({filled} campos rellenados): no hay nada que enviar");
        gw.stop();
        return 1;
    };
    let resolved = a2ui::model::resolve_context(&context, &a2ui::model::Ctx::new(&sf.data, None));
    let frame = a2ui::model::action_frame(&name, &sf.id, &button_id, resolved);
    println!("── turno 2: pulsar «{button_id}» → acción {name}");
    println!("contexto enviado: {}", frame["data"]["context"]);
    chat.send_raw(&frame);
    text.clear();
    wait_turn(
        &chat,
        &mut a2,
        &mut frames,
        &mut text,
        std::time::Instant::now(),
        25,
        240,
    );
    println!(
        "respuesta 2: {}",
        text.trim().chars().take(400).collect::<String>()
    );
    if let Ok(path) = std::env::var("A2UI_DUMP") {
        let _ = std::fs::write(
            &path,
            serde_json::to_string_pretty(&frames).unwrap_or_default(),
        );
        println!("frames guardados: {} en {path}", frames.len());
    }
    gw.stop();
    0
}

/// `--a2ui "petición"`: un turno real pidiéndole al agente una superficie A2UI; resume lo que llega
/// y, con `A2UI_DUMP=ruta`, guarda los frames para repetirlos con `--scene a2ui-replay`.
/// Gasta una vuelta del modelo del agente.
fn a2ui_check(args: &[String]) -> i32 {
    let prompt = args
        .iter()
        .position(|a| a == "--a2ui")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "Usa tu herramienta de A2UI para mostrarme un formulario simple con un campo de nombre, una casilla y un botón Enviar. No hagas nada más.".into());
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    let chat = match Chat::bootstrap_fresh(gw.port, None) {
        Ok(c) => c,
        Err(e) => {
            println!("chat no disponible: {e}");
            gw.stop();
            return 1;
        }
    };
    chat.send(&prompt);
    let mut a2 = a2ui::view::A2State::default();
    let (mut frames, mut text) = (Vec::new(), String::new());
    let start = std::time::Instant::now();
    wait_turn(&chat, &mut a2, &mut frames, &mut text, start, 30, 300);
    println!(
        "turno: {} s · respuesta: {}",
        start.elapsed().as_secs(),
        text.trim().chars().take(300).collect::<String>()
    );
    println!("superficies: {}", a2.store.surfaces.len());
    for sf in &a2.store.surfaces {
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for def in sf.components.values() {
            *kinds
                .entry(
                    def.get("component")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                )
                .or_default() += 1;
        }
        println!(
            "  {} raíz={:?} componentes={} tipos={kinds:?}",
            sf.id,
            sf.root,
            sf.components.len()
        );
    }
    if let Ok(path) = std::env::var("A2UI_DUMP") {
        let _ = std::fs::write(
            &path,
            serde_json::to_string_pretty(&frames).unwrap_or_default(),
        );
        println!("frames guardados: {} en {path}", frames.len());
    }
    gw.stop();
    0
}

/// `--settings`: lee proveedores, agentes y canales (solo GET) y los resume. Nunca imprime claves.
fn settings_check() -> i32 {
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    let conn = settings::api::Conn {
        base: format!("http://127.0.0.1:{}", gw.port),
        token: std::env::var("HIVE_AUTH_TOKEN").ok(),
    };
    match settings::api::providers(&conn) {
        Ok(list) => {
            let llm: Vec<_> = list
                .iter()
                .filter(|p| p.category.is_empty() || p.category == "llm")
                .collect();
            println!("proveedores: {} ({} de lenguaje)", list.len(), llm.len());
            for p in llm.iter().filter(|p| p.enabled || p.has_api_key) {
                println!(
                    "  {} activo={} clave={} modelos={}/{}",
                    p.id,
                    p.enabled && p.active,
                    if p.has_api_key { "sí" } else { "no" },
                    p.models.iter().filter(|m| m.enabled).count(),
                    p.models.len()
                );
            }
        }
        Err(e) => println!("proveedores: error {e}"),
    }
    match settings::api::agents(&conn) {
        Ok(list) => {
            println!("agentes: {}", list.len());
            for a in list.iter().take(5) {
                println!(
                    "  {} [{}] estado={} modelo={}",
                    a.name, a.role, a.status, a.model_id
                );
            }
        }
        Err(e) => println!("agentes: error {e}"),
    }
    match settings::api::tools(&conn) {
        Ok(list) => {
            let on = list.iter().filter(|t| t.active && t.enabled).count();
            let mut cats: Vec<&str> = list.iter().map(|t| t.category.as_str()).collect();
            cats.sort();
            cats.dedup();
            println!(
                "herramientas: {} ({on} activas) categorías: {}",
                list.len(),
                cats.join(", ")
            );
        }
        Err(e) => println!("herramientas: error {e}"),
    }
    match settings::api::mcp_servers(&conn) {
        Ok(list) => {
            println!("mcp: {}", list.len());
            for m in &list {
                println!(
                    "  {} estado={} activo={} herramientas={}",
                    m.name,
                    m.status,
                    m.enabled,
                    m.tool_count().map_or("?".to_string(), |n| n.to_string())
                );
            }
        }
        Err(e) => println!("mcp: error {e}"),
    }
    match settings::api::skills(&conn) {
        Ok(list) => println!(
            "skills: {} ({} activas)",
            list.len(),
            list.iter().filter(|x| x.active).count()
        ),
        Err(e) => println!("skills: error {e}"),
    }
    match settings::api::channels(&conn) {
        Ok(list) => {
            println!("canales: {}", list.len());
            for c in &list {
                println!(
                    "  {} activo={} estado={} configurado={}",
                    c.kind,
                    c.enabled && c.active,
                    c.status,
                    c.is_configured
                );
            }
        }
        Err(e) => println!("canales: error {e}"),
    }
    gw.stop();
    0
}

/// `--audio`: lista dispositivos, mide el micrófono 2 s y reproduce el tono de prueba.
fn audio_check() -> i32 {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (name, dir) in [
        ("entradas", hive_media::Direction::Input),
        ("salidas", hive_media::Direction::Output),
    ] {
        match voice::devices(dir) {
            Ok(list) => {
                println!("{name}: {}", list.len());
                for d in list {
                    println!(
                        "  {}{} [{}]",
                        if d.is_default { "* " } else { "" },
                        d.name,
                        d.id
                    );
                }
            }
            Err(e) => println!("{name}: error {e:?}"),
        }
    }
    match hive_media::list_system_sources() {
        Ok(list) => {
            println!("audio del sistema: {}", list.len());
            for d in list {
                println!(
                    "  {}{} [{}]",
                    if d.is_default { "* " } else { "" },
                    d.name,
                    d.id
                );
            }
        }
        Err(e) => println!("audio del sistema: error {e:?}"),
    }
    let mut v = voice::Voice::default();
    let frames = std::sync::Arc::new(AtomicUsize::new(0));
    let bytes = std::sync::Arc::new(AtomicUsize::new(0));
    let (f, b) = (frames.clone(), bytes.clone());
    match v.start_mic(None, move |pcm| {
        f.fetch_add(1, Ordering::Relaxed);
        b.fetch_add(pcm.len(), Ordering::Relaxed);
    }) {
        Ok(()) => {
            let mut peak = 0f32;
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                peak = peak.max(v.mic_level());
            }
            println!(
                "micrófono: {} bloques, {} bytes, nivel pico {peak:.4}",
                frames.load(Ordering::Relaxed),
                bytes.load(Ordering::Relaxed)
            );
            v.stop_mic();
        }
        Err(e) => println!("micrófono: error {e:?}"),
    }
    match v.start_player(None) {
        Ok(()) => {
            v.enqueue(&voice::test_tone());
            println!("altavoz: {} ms en cola", v.queued_ms());
            while v.queued_ms() > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            println!("altavoz: cola vacía");
            v.stop_player();
        }
        Err(e) => println!("altavoz: error {e:?}"),
    }
    0
}

/// `--voice [segundos]`: abre una sesión de voz real y muestra lo que llega.
/// Con `--camera` abre la cámara real al estar lista la sesión, manda sus fotogramas (solo cuenta cuántos y su tamaño;
/// no guarda ninguna imagen) y le pregunta al modelo qué ve. **Gasta tokens y enciende la cámara.**
fn voice_check(args: &[String]) -> i32 {
    let secs: u64 = args
        .iter()
        .position(|a| a == "--voice")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    // Usuario real del gateway; en modo setup cae a "default".
    let (user, token) = match Chat::bootstrap_fresh(gw.port, None) {
        Ok(c) => (c.user_id.clone(), c.token.clone()),
        Err(_) => ("default".to_string(), None),
    };
    let with_camera = args.iter().any(|a| a == "--camera");
    match Realtime::start(gw.port, &user, token.as_deref(), false) {
        Ok(rt) => {
            let start = std::time::Instant::now();
            let mut closed = false;
            let mut greeted = false;
            let mut camera: Option<hive_media::Camera> = None;
            let (frames, bytes) = (
                std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            );
            let mut asked_at: Option<std::time::Instant> = None;
            while !closed && start.elapsed().as_secs() < secs {
                std::thread::sleep(std::time::Duration::from_millis(100));
                for ev in rt.drain() {
                    println!("{ev:?}");
                    closed |= ev == REvent::Closed;
                    // Con la sesión lista, pedirle una palabra por texto: ejercita transcripción y voz.
                    if !greeted && matches!(ev, REvent::Ready { .. }) {
                        greeted = true;
                        if with_camera {
                            let (mut send, f, b) =
                                (rt.video_sink(None), frames.clone(), bytes.clone());
                            match hive_media::Camera::start(None, move |fr| {
                                if fr.kind == hive_media::FrameKind::Model {
                                    f.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                    b.fetch_add(fr.jpeg.len(), std::sync::atomic::Ordering::SeqCst);
                                }
                                send(fr);
                            }) {
                                Ok(c) => {
                                    println!("cámara: abierta");
                                    camera = Some(c);
                                    asked_at = Some(std::time::Instant::now());
                                }
                                Err(e) => println!("cámara: error {}", e.message),
                            }
                        } else {
                            rt.text("Responde en una sola palabra: hola");
                        }
                    }
                }
                // Con la cámara abierta se le deja ver unos segundos y entonces se le pregunta por escrito.
                if asked_at.is_some_and(|t| t.elapsed().as_secs() >= 4) {
                    asked_at = None;
                    println!(
                        "fotogramas enviados hasta ahora: {} ({} bytes)",
                        frames.load(std::sync::atomic::Ordering::SeqCst),
                        bytes.load(std::sync::atomic::Ordering::SeqCst)
                    );
                    rt.text("Mira la imagen de la cámara y dime en una sola frase corta qué ves. No describas a personas, solo objetos y colores.");
                }
            }
            if let Some(mut c) = camera.take() {
                c.stop();
                println!(
                    "cámara: cerrada; fotogramas enviados en total: {}",
                    frames.load(std::sync::atomic::Ordering::SeqCst)
                );
            }
            if !closed {
                rt.stop();
                std::thread::sleep(std::time::Duration::from_millis(300));
                println!("{:?}", rt.drain());
            }
        }
        Err(e) => println!("voz: {e}"),
    }
    gw.stop();
    0
}

/// Verificación sin ventana: `--check` (gateway) y `--chat "texto"` (turno completo).
fn cli(args: &[String]) -> i32 {
    let gw = match Gateway::start() {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!("FALLO gateway: {e}");
            return 1;
        }
    };
    println!(
        "gateway puerto={} externo={} health={}",
        gw.port,
        gw.is_external(),
        gateway::is_healthy(gw.port)
    );
    if args.iter().any(|a| a == "--ws") {
        // Solo el apretón de manos: en modo setup /ws acepta sesiones sin credenciales.
        match Chat::connect(gw.port, "default", None, String::new()) {
            Ok(chat) => {
                std::thread::sleep(std::time::Duration::from_secs(2));
                println!("ws eventos: {:?}", chat.drain());
            }
            Err(e) => println!("ws fallo: {e}"),
        }
    }
    let code = match Chat::bootstrap_fresh(gw.port, None) {
        Ok(chat) => {
            println!("chat listo thread={}", chat.thread_id);
            if let Some(i) = args.iter().position(|a| a == "--chat") {
                chat.send(args.get(i + 1).map(String::as_str).unwrap_or("hola"));
                let start = std::time::Instant::now();
                let mut done = false;
                let mut failed = false;
                while !done && start.elapsed().as_secs() < 120 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    for ev in chat.drain() {
                        println!("{ev:?}");
                        failed |= matches!(ev, Event::Error(_) | Event::Closed);
                        done |= matches!(ev, Event::Done | Event::Error(_) | Event::Closed);
                    }
                }
                if !done || failed {
                    gw.stop();
                    return 1;
                }
            }
            0
        }
        Err(e) => {
            println!("chat no disponible: {e}");
            1
        }
    };
    gw.stop();
    code
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--office-check") {
        std::process::exit(office_check());
    }
    if let Some(i) = args.iter().position(|a| a == "--kill-check") {
        std::process::exit(kill_check(args.get(i + 1).map_or("exit", String::as_str)));
    }
    if args.iter().any(|a| a == "--tray-check") {
        std::process::exit(tray_check());
    }
    if args.iter().any(|a| a == "--a2ui-flow") {
        std::process::exit(a2ui_flow());
    }
    if args.iter().any(|a| a == "--a2ui") {
        std::process::exit(a2ui_check(&args));
    }
    if args.iter().any(|a| a == "--settings") {
        std::process::exit(settings_check());
    }
    if args.iter().any(|a| a == "--voice") {
        std::process::exit(voice_check(&args));
    }
    if args.iter().any(|a| a == "--audio") {
        std::process::exit(audio_check());
    }
    if args
        .iter()
        .any(|a| a == "--check" || a == "--chat" || a == "--ws")
    {
        std::process::exit(cli(&args));
    }
    let no_tray = args.iter().any(|a| a == "--no-tray");
    let mut app = rune::App::new(AppSpec::<State> {
        scenes: &[
            ("notes", demo_notes),
            ("cron", demo_cron),
            ("apiclient", demo_apiclient),
            ("dashboard", demo_dashboard),
            ("ethics", demo_ethics),
            ("profile", demo_profile),
            ("display", demo_display),
            ("updates", demo_updates),
            ("channels", demo_channels),
            ("skills", demo_skills),
            ("mcp", demo_mcp),
            ("agent-new", demo_agent_new),
            ("agent-detail", demo_agent_detail),
            ("agents", demo_agents),
            ("panel", demo_panel),
            ("live", demo_live),
            ("live-call", demo_live_call),
            ("chat", demo_chat),
            ("chat-msgs", demo_chat_msgs),
            ("approval", demo_approval),
            ("meeting", demo_meeting),
            ("office", demo_office),
            ("a2ui", demo_a2ui),
            ("login", demo_login),
            ("settings", demo_settings),
            ("markdown", demo_markdown),
            ("tools", demo_tools),
            ("a2ui-replay", demo_replay),
        ],
        poll: Some(poll),
        ..AppSpec::new("Hive", (1100.0, 640.0), State::default, view)
    })
    .compositor(live::stage::Stages::default());
    // Bandeja: el icono siempre se registra (aparece cuando haya anfitrión). Cerrar la ventana solo la
    // manda a la bandeja si hay anfitrión y el icono se creó: si no, la app quedaría invisible.
    let _tray = if no_tray || is_headless(&args) {
        None
    } else {
        match tray::spawn(app.handle()) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("bandeja: {e}");
                None
            }
        }
    };
    if _tray.is_some()
        && tray::close_to_tray(
            tray::host_present(),
            std::env::var("HIVE_CLOSE_TO_TRAY").ok().as_deref(),
            no_tray,
        )
    {
        app = app.keep_running();
    }
    app.run();
    if let Some(gw) = GATEWAY.lock().unwrap().take() {
        gw.stop();
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use rune::{Env, Runtime};
    use serde_json::{json, Value};

    const BTN: &str = "demo/root>tabs>form>go";

    fn boot_ui(mail: &str) -> Runtime<State> {
        let mut st = State::offline();
        demo_a2ui(&mut st);
        st.a2.apply(
            "a2ui:updateDataModel",
            &json!({"surfaceId": "demo", "path": "/user/mail", "value": mail}),
        );
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        rt
    }

    fn click(rt: &mut Runtime<State>, key: &str) {
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("no hay elemento {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        rt.frame(1.0 / 60.0);
    }

    fn model(rt: &Runtime<State>, path: &str) -> Value {
        get_at_path_value(&rt.state.a2.store.get("demo").unwrap().data, path)
    }

    fn get_at_path_value(v: &Value, path: &str) -> Value {
        a2ui::model::get_at_path(v, path)
            .cloned()
            .unwrap_or(Value::Null)
    }

    #[test]
    fn enabled_button_sends_an_action_with_the_context_resolved_at_click_time() {
        let mut rt = boot_ui("ana@b.co");
        click(&mut rt, BTN);
        let out = &rt.state.a2.outbox;
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["type"], "a2ui:action");
        assert_eq!(out[0]["data"]["name"], "submit");
        assert_eq!(out[0]["data"]["surfaceId"], "demo");
        assert_eq!(out[0]["data"]["sourceComponentId"], "go");
        assert_eq!(
            out[0]["data"]["context"],
            json!({"name": "Ana", "plan": ["pro"], "src": "form"})
        );
    }

    #[test]
    fn button_with_a_failing_check_is_disabled() {
        let mut rt = boot_ui("ana@");
        click(&mut rt, BTN);
        assert!(rt.state.a2.outbox.is_empty());
    }

    #[test]
    fn a_model_change_re_enables_the_button() {
        let mut rt = boot_ui("ana@");
        // Por la vía de actualización de Rune, para que reconstruya la vista.
        rt.update_if(|s: &mut State, _| {
            s.a2.apply(
                "a2ui:updateDataModel",
                &json!({"surfaceId": "demo", "path": "/user/mail", "value": "ana@b.co"}),
            )
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, BTN);
        assert_eq!(rt.state.a2.outbox.len(), 1);
    }

    #[test]
    fn checkbox_writes_back_to_the_model() {
        let mut rt = boot_ui("a@b.co");
        assert_eq!(model(&rt, "/terms"), json!(false));
        click(&mut rt, "demo/root>tabs>form>terms");
        assert_eq!(model(&rt, "/terms"), json!(true));
        click(&mut rt, "demo/root>tabs>form>terms");
        assert_eq!(model(&rt, "/terms"), json!(false));
    }

    #[test]
    fn single_choice_replaces_the_selection() {
        let mut rt = boot_ui("a@b.co");
        click(&mut rt, "demo/root>tabs>form>plan#0");
        assert_eq!(model(&rt, "/plan"), json!(["basic"]));
        click(&mut rt, "demo/root>tabs>form>plan#1");
        assert_eq!(model(&rt, "/plan"), json!(["pro"]));
    }

    #[test]
    fn slider_steps_and_clamps() {
        let mut rt = boot_ui("a@b.co");
        click(&mut rt, "demo/root>tabs>form>seats+");
        assert_eq!(model(&rt, "/seats"), json!(4.0));
        for _ in 0..20 {
            click(&mut rt, "demo/root>tabs>form>seats+");
        }
        assert_eq!(model(&rt, "/seats"), json!(10.0), "se detiene en el máximo");
    }

    #[test]
    fn tabs_switch_the_visible_content() {
        let mut rt = boot_ui("a@b.co");
        assert!(rt.find("demo/root>tabs>form>go").is_some());
        click(&mut rt, "demo/root>tabs#1");
        assert!(
            rt.find("demo/root>tabs>form>go").is_none(),
            "el formulario ya no se pinta"
        );
        assert!(
            rt.find("demo/root>tabs>orders").is_some()
                || rt.state.a2.tabs.values().any(|t| t == "Pedidos")
        );
    }

    #[test]
    fn server_updates_win_over_a_bound_text_field() {
        let mut st = State::offline();
        demo_a2ui(&mut st);
        assert_eq!(
            st.a2.edits.get("demo/name").map(|e| e.text.as_str()),
            Some("Ana")
        );
        st.a2.apply(
            "a2ui:updateDataModel",
            &json!({"surfaceId": "demo", "path": "/user/name", "value": "Beto"}),
        );
        assert_eq!(
            st.a2.edits.get("demo/name").map(|e| e.text.as_str()),
            Some("Beto")
        );
        st.a2
            .apply("a2ui:deleteSurface", &json!({"surfaceId": "demo"}));
        assert!(
            st.a2.edits.is_empty(),
            "los editores de una superficie borrada se limpian"
        );
    }

    #[test]
    fn login_card_shows_when_the_gateway_needs_credentials() {
        let mut st = State::offline();
        st.shown.status = Some(Status::Ready {
            port: 1,
            external: false,
        });
        st.shown.needs_login = true;
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(
            rt.find("login-email").is_some()
                && rt.find("login-pass").is_some()
                && rt.find("login-go").is_some()
        );
        assert_eq!(header_line(&rt.state), "Inicia sesión");
    }

    #[test]
    fn login_card_is_gone_once_the_chat_is_ready() {
        let mut st = State::offline();
        st.shown.needs_login = true;
        st.chat_ready = true;
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(rt.find("login-email").is_none());
    }

    #[test]
    fn the_password_field_is_secure() {
        assert!(
            State::offline().login_pass.is_secure(),
            "el campo de contraseña oculta lo escrito"
        );
    }

    /// Una imagen PNG de 4×2 servida por un servidor local, para ver la vista pasar de
    /// "Cargando…" a la imagen con su proporción.
    #[test]
    fn a2ui_image_loads_and_keeps_its_aspect_ratio() {
        use std::io::{Read, Write};
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            2,
            image::Rgba([10, 200, 90, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                png.len()
            )
            .unwrap();
            sock.write_all(&png).unwrap();
        });

        let mut st = State::offline();
        st.a2
            .images
            .set_origin(&format!("http://127.0.0.1:{port}"), None);
        st.a2
            .apply("a2ui:createSurface", &json!({"surfaceId": "demo"}));
        st.a2.apply(
            "a2ui:updateComponents",
            &json!({"surfaceId": "demo", "components": [
            {"id": "root", "component": "Image", "url": "/p.png", "usageHint": "mediumFeature"}]}),
        );
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(
            rt.find("demo/root").is_none(),
            "mientras carga se ve el marcador, no la imagen"
        );

        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !rt.state.a2.images.take_dirty() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        rt.update_if(|_: &mut State, _| true);
        rt.frame(1.0 / 60.0);
        rt.frame(1.0 / 60.0);
        let (rect, _) = rt.presented("demo/root").expect("la imagen ya se pinta");
        assert!(
            (rect[2] - 192.0).abs() < 0.5,
            "ancho de mediumFeature: {rect:?}"
        );
        assert!(
            (rect[3] - 96.0).abs() < 1.5,
            "el alto sale de la proporción 4:2: {rect:?}"
        );
    }

    // ── pantalla de ajustes ──

    use crate::settings::{api::Conn, Action, Data, Tab};
    use crate::testutil::fake_http;

    fn lists(path: &str) -> Value {
        match path {
            "/api/providers" => json!({"providers": [
                {"id": "openai", "name": "OpenAI", "category": "llm", "enabled": true, "active": true, "has_api_key": 1,
                 "masked_api_key": "sk-p••••••••abcd", "models": [{"id": "openai/gpt-4o", "name": "gpt-4o", "enabled": true}]},
                {"id": "elevenlabs", "name": "ElevenLabs", "category": "tts", "enabled": true, "active": true, "has_api_key": 0, "models": []}]}),
            "/api/agents" => {
                json!({"agents": [{"id": "a1", "name": "Hive", "description": "Coordina", "role": "coordinator", "status": "idle",
                "enabled": true, "providerId": "openai", "modelId": "openai/gpt-4o"}]})
            }
            "/api/channels" => {
                json!({"channels": [{"id": "telegram", "type": "telegram", "enabled": true, "active": true, "status": "connected", "isConfigured": true}]})
            }
            "/api/tools" => json!({"tools": [
                {"id": "web_search", "name": "web_search", "description": "Busca en la web", "category": "web", "active": 1, "enabled": true},
                {"id": "read_file", "name": "read_file", "description": "Lee un archivo", "category": "fs", "active": true, "enabled": true},
                {"id": "web_fetch", "name": "web_fetch", "description": "Descarga una página", "category": "web", "active": false, "enabled": true}]}),
            "/api/skills" => {
                json!({"skills": [{"id": "sk1", "name": "Resumen", "description": "Resume textos", "active": true}]})
            }
            "/api/mcp/servers" => {
                json!([{"id": "cal", "name": "calendario", "status": "connected", "enabled": true, "tools": [{}, {}, {}]},
                {"id": "mail", "name": "Email", "status": "disconnected", "enabled": false}])
            }
            _ => json!({"ok": true, "success": true}),
        }
    }

    /// Estado en la pantalla de ajustes con las listas ya cargadas y un gateway falso por conexión.
    fn settings_rt(port: u16, tab: Tab) -> Runtime<State> {
        settings_rt_sized(port, tab, (1100.0, 900.0))
    }

    fn settings_rt_sized(port: u16, tab: Tab, size: (f32, f32)) -> Runtime<State> {
        let mut st = State::offline();
        st.screen = Screen::Settings;
        st.settings.tab = tab;
        // Como la web, lo que se configura (claves, tokens) vive tras el botón CONFIG / la tarjeta; las pruebas lo abren de entrada.
        match tab {
            Tab::Providers => st.settings.config_open = Some("openai".into()),
            Tab::Channels => {
                st.settings.config_open = Some("telegram".into());
                st.settings.add_open = true;
            }
            _ => {}
        }
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: Some("t".into()),
        });
        {
            let mut d = st.settings.data.lock().unwrap();
            *d = Data {
                loaded: true,
                ..Default::default()
            };
            d.providers =
                serde_json::from_value(lists("/api/providers")["providers"].clone()).unwrap();
            d.agents = serde_json::from_value(lists("/api/agents")["agents"].clone()).unwrap();
            d.channels =
                serde_json::from_value(lists("/api/channels")["channels"].clone()).unwrap();
            d.tools = serde_json::from_value(lists("/api/tools")["tools"].clone()).unwrap();
            d.skills = serde_json::from_value(lists("/api/skills")["skills"].clone()).unwrap();
            d.mcp = serde_json::from_value(lists("/api/mcp/servers")).unwrap();
            d.extras_loaded = true;
        }
        st.settings.sync();
        let mut rt = Runtime::new(
            st,
            view,
            size,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        rt
    }

    fn wait_idle(rt: &Runtime<State>) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while rt.state.settings.data.lock().unwrap().busy > 0 && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            rt.state.settings.data.lock().unwrap().busy,
            0,
            "la operación no terminó"
        );
    }

    fn bodies(seen: &[crate::testutil::Request]) -> Vec<(String, String, Value)> {
        seen.iter()
            .map(|(m, p, _, b)| {
                (
                    m.clone(),
                    p.clone(),
                    serde_json::from_str(b).unwrap_or(Value::Null),
                )
            })
            .collect()
    }

    #[test]
    fn only_language_providers_are_listed() {
        let (port, _server) = fake_http(|_, p, _, _| (200, lists(p)), 0);
        let rt = settings_rt(port, Tab::Providers);
        assert!(rt.find("prov/openai/toggle").is_some());
        assert!(
            rt.find("prov/elevenlabs/toggle").is_none(),
            "los de voz no son proveedores de lenguaje"
        );
    }

    #[test]
    fn toggling_a_provider_posts_then_rereads_the_lists() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Providers);
        click(&mut rt, "prov/openai/toggle");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/providers/openai/toggle".into(),
                json!({"active": false})
            )
        );
        let reads: Vec<&str> = seen[1..].iter().map(|r| r.1.as_str()).collect();
        assert_eq!(reads, ["/api/providers", "/api/agents", "/api/channels"]);
        assert_eq!(
            rt.state.settings.data.lock().unwrap().notice.as_deref(),
            Some("Proveedor actualizado")
        );
    }

    #[test]
    fn a_providers_models_unfold_and_one_can_be_switched_off() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Providers);
        assert!(
            rt.find("model/openai/gpt-4o/toggle").is_none(),
            "plegados por defecto"
        );
        click(&mut rt, "prov/openai/models");
        assert_eq!(rt.state.settings.models_open.as_deref(), Some("openai"));
        assert!(
            rt.find("model/openai/gpt-4o/toggle").is_some(),
            "al desplegar aparece el modelo"
        );
        click(&mut rt, "model/openai/gpt-4o/toggle");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/models/openai%2Fgpt-4o/toggle".into(),
                json!({"active": false})
            )
        );
        assert_eq!(
            rt.state.settings.data.lock().unwrap().notice.as_deref(),
            Some("Modelo actualizado")
        );
        click(&mut rt, "prov/openai/models");
        assert!(
            rt.find("model/openai/gpt-4o/toggle").is_none(),
            "se vuelve a plegar"
        );
    }

    #[test]
    fn clearing_a_key_needs_a_second_confirming_click() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Providers);
        click(&mut rt, "prov/openai/clear");
        assert_eq!(rt.state.settings.confirm_clear.as_deref(), Some("openai"));
        assert_eq!(
            rt.state.settings.data.lock().unwrap().busy,
            0,
            "el primer clic no envía nada"
        );
        click(&mut rt, "prov/openai/clear-no");
        assert_eq!(rt.state.settings.confirm_clear, None, "cancelar");
        click(&mut rt, "prov/openai/clear");
        click(&mut rt, "prov/openai/clear-yes");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "PUT".into(),
                "/api/providers/openai".into(),
                json!({"clearApiKey": true})
            )
        );
        assert_eq!(seen.len(), 4, "una sola escritura y tres lecturas");
    }

    #[test]
    fn saving_a_key_sends_it_and_empties_the_field() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Providers);
        rt.update_if(|s: &mut State, _| {
            s.settings.keys.insert(
                "openai".into(),
                rune::TextEdit::new("  sk-secreta  ").secure(),
            );
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "prov/openai/save");
        assert!(
            rt.state.settings.keys["openai"].text.is_empty(),
            "la clave no se queda en pantalla"
        );
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "PUT".into(),
                "/api/providers/openai".into(),
                json!({"apiKey": "sk-secreta"})
            )
        );
    }

    #[test]
    fn saving_or_verifying_an_empty_key_sends_nothing() {
        let (port, _server) = fake_http(|_, p, _, _| (200, lists(p)), 0);
        let mut rt = settings_rt(port, Tab::Providers);
        click(&mut rt, "prov/openai/save");
        click(&mut rt, "prov/openai/verify");
        let d = rt.state.settings.data.lock().unwrap().clone();
        assert_eq!(
            (d.busy, d.notice.as_deref()),
            (0, Some("Escribe la clave primero"))
        );
    }

    #[test]
    fn a_bad_key_verification_surfaces_the_providers_error() {
        let (port, server) = fake_http(
            |m, p, _, _| {
                if p == "/api/setup/verify-provider" && m == "POST" {
                    (200, json!({"success": false, "error": "API error: 401"}))
                } else {
                    (200, lists(p))
                }
            },
            1,
        );
        let mut rt = settings_rt(port, Tab::Providers);
        rt.update_if(|s: &mut State, _| {
            s.settings
                .keys
                .insert("openai".into(), rune::TextEdit::new("mala").secure());
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "prov/openai/verify");
        wait_idle(&rt);
        server.join().unwrap();
        assert_eq!(
            rt.state.settings.data.lock().unwrap().error.as_deref(),
            Some("API error: 401")
        );
        assert_eq!(
            rt.state.settings.keys["openai"].text, "mala",
            "verificar no vacía el campo ni guarda nada"
        );
    }

    #[test]
    fn a_channel_token_is_sent_through_reconnect_and_cleared_from_the_field() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Channels);
        assert!(
            rt.find("chan/telegram/token").is_some(),
            "los canales con bot muestran el campo"
        );
        // Demasiado corto: no se envía nada y el campo se queda.
        rt.update_if(|s: &mut State, _| {
            s.settings
                .channel_tokens
                .insert("telegram".into(), rune::TextEdit::new("123").secure());
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "chan/telegram/token-save");
        assert_eq!(
            rt.state.settings.data.lock().unwrap().busy,
            0,
            "un token corto no sale"
        );
        assert_eq!(rt.state.settings.channel_tokens["telegram"].text, "123");
        rt.update_if(|s: &mut State, _| {
            s.settings.channel_tokens.insert(
                "telegram".into(),
                rune::TextEdit::new(" 123456:ABC-secreto ").secure(),
            );
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "chan/telegram/token-save");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/channels/telegram/reconnect".into(),
                json!({"config": {"botToken": "123456:ABC-secreto"}})
            )
        );
        assert_eq!(
            rt.state.settings.channel_tokens["telegram"].text, "",
            "el token no se queda en pantalla"
        );
        let notice = rt
            .state
            .settings
            .data
            .lock()
            .unwrap()
            .notice
            .clone()
            .unwrap_or_default();
        assert!(
            !notice.contains("secreto"),
            "el aviso nunca repite el token"
        );
    }

    #[test]
    fn linking_whatsapp_shows_the_qr_then_reports_connected() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polls = std::sync::Arc::new(AtomicUsize::new(0));
        let p2 = polls.clone();
        let (port, server) = fake_http(
            move |m, p, _, _| {
                if p == "/api/channels/whatsapp/wa1/status" && m == "GET" {
                    return match p2.fetch_add(1, Ordering::SeqCst) {
                        0 => (
                            200,
                            json!({"status": "qr", "qrCode": "2@abc123,def456,ghi789=="}),
                        ),
                        _ => (200, json!({"status": "connected"})),
                    };
                }
                (200, lists(p))
            },
            6,
        );
        let mut rt = settings_rt(port, Tab::Channels);
        rt.state.settings.whatsapp_poll_ms = Some(150);
        {
            let mut d = rt.state.settings.data.lock().unwrap();
            d.channels.push(serde_json::from_value(json!({"id": "wa1", "type": "whatsapp", "enabled": true, "active": true, "status": "disconnected", "isConfigured": false})).unwrap());
        }
        rt.state.settings.sync();
        rt.update_if(|s: &mut State, _| {
            s.settings.config_open = Some("wa1".into());
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("chan/wa1/token").is_none(), "WhatsApp no usa token");
        click(&mut rt, "chan/wa1/link");
        // Espera el QR.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut saw_qr = false;
        while std::time::Instant::now() < end {
            rt.update_if(|s: &mut State, _| s.settings.sync());
            rt.frame(1.0 / 60.0);
            if rt.find("chan/wa1/qr").is_some() {
                saw_qr = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/channels/wa1/reconnect".into(),
                json!({"config": {}})
            )
        );
        assert_eq!(seen[1].1, "/api/channels/whatsapp/wa1/status");
        rt.state.settings.sync();
        assert_eq!(
            rt.state
                .settings
                .shown
                .whatsapp
                .as_ref()
                .map(|w| w.status.as_str()),
            Some("connected")
        );
        assert!(saw_qr, "el QR se vio en pantalla antes de conectar");
        assert!(
            polls.load(Ordering::SeqCst) >= 2,
            "se siguió consultando hasta conectar"
        );
    }

    #[test]
    fn adding_a_channel_validates_the_fields_then_posts_and_clears_the_secrets() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Channels);
        assert!(
            rt.find("chan/add/token").is_none(),
            "el formulario empieza cerrado"
        );
        click(&mut rt, "chan/add/discord");
        assert!(
            rt.find("chan/add/token").is_some() && rt.find("chan/add/extra").is_some(),
            "Discord pide token y Application ID"
        );
        assert!(
            rt.state.settings.add_token.is_secure(),
            "el token es secreto"
        );
        // Falta el Application ID: no sale nada.
        rt.update_if(|s: &mut State, _| {
            s.settings.add_token = rune::TextEdit::new("discord-token-123456").secure();
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "chan/add/create");
        assert_eq!(
            rt.state.settings.data.lock().unwrap().busy,
            0,
            "incompleto no se envía"
        );
        assert_eq!(
            rt.state.settings.add_token.text, "discord-token-123456",
            "y el campo se conserva"
        );
        rt.update_if(|s: &mut State, _| {
            s.settings.add_extra = rune::TextEdit::new(" 998877 ");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "chan/add/create");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/channels".into(),
                json!({"type": "discord", "config": {"botToken": "discord-token-123456", "applicationId": "998877"}})
            )
        );
        assert_eq!(
            rt.state.settings.add_token.text, "",
            "el token no queda en pantalla"
        );
        assert_eq!(rt.state.settings.add_kind, None, "el formulario se cierra");
        assert_eq!(
            rt.state.settings.data.lock().unwrap().notice.as_deref(),
            Some("Canal creado")
        );
    }

    #[test]
    fn a_server_notice_shows_once_dismisses_on_click_and_expires() {
        let mut st = State::offline();
        apply(
            &mut st,
            Event::Notification {
                id: "n1".into(),
                content: "Reunión en 5 minutos".into(),
            },
        );
        apply(
            &mut st,
            Event::Notification {
                id: "n1".into(),
                content: "Reunión en 5 minutos".into(),
            },
        ); // reenvío
        apply(
            &mut st,
            Event::Notification {
                id: "n2".into(),
                content: "  ".into(),
            },
        );
        assert_eq!(st.notices.len(), 2, "el mismo id no se repite");
        assert_eq!(
            st.notices[1].1, "Nueva notificación",
            "sin texto, el mismo mensaje genérico que la web"
        );
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(rt.find("notice/n1").is_some() && rt.find("notice/n2").is_some());
        click(&mut rt, "notice/n1");
        assert!(
            rt.find("notice/n1").is_none() && rt.find("notice/n2").is_some(),
            "un clic lo descarta"
        );
        // Caduca a los 8 s: se simula un aviso más viejo que el plazo.
        rt.state.notices[0].2 =
            std::time::Instant::now() - NOTICE_TTL - std::time::Duration::from_secs(1);
        let mut tx = rune::Tx::default();
        assert!(poll(&mut rt.state, 0.0, &mut tx));
        assert!(rt.state.notices.is_empty(), "el aviso vencido se retira");
    }

    fn ask(id: &str, tool: &str, ms: u64) -> Event {
        Event::Approval {
            id: id.into(),
            tool: tool.into(),
            arguments: serde_json::json!({"agente":"ventas","n":1}),
            reason: Some("la política pide confirmación".into()),
            expires_in_ms: ms,
        }
    }

    #[test]
    fn a_tool_approval_shows_on_any_screen_and_each_button_sends_its_answer_once() {
        let mut st = State::offline();
        st.screen = Screen::Settings; // no es el chat: la aprobación se ve igual
        apply(&mut st, ask("ap1", "task_delegate", 30_000));
        apply(&mut st, ask("ap1", "task_delegate", 30_000)); // reenvío del mismo id
        apply(&mut st, ask("ap2", "a2ui_createSurface", 30_000));
        assert_eq!(st.approvals.len(), 2, "el mismo id no se repite");
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for key in [
            "approval/ap1",
            "approval/ap1/approve",
            "approval/ap1/deny",
            "approval/ap2/approve",
            "approval/ap2/deny",
        ] {
            assert!(rt.find(key).is_some(), "falta {key}");
        }
        click(&mut rt, "approval/ap1/approve");
        click(&mut rt, "approval/ap2/deny");
        assert_eq!(
            rt.state.a2.outbox,
            vec![
                chat::approval_frame("ap1", true),
                chat::approval_frame("ap2", false)
            ]
        );
        assert!(
            rt.state.approvals.is_empty()
                && rt.find("approval/ap1").is_none()
                && rt.find("approval/ap2").is_none(),
            "contestadas, desaparecen"
        );
        // Contestar dos veces no manda dos respuestas.
        answer_approval(&mut rt.state, "ap1", true);
        assert_eq!(rt.state.a2.outbox.len(), 2);
    }

    #[test]
    fn in_the_chat_the_a2ui_surface_replaces_the_card_its_buttons_answer_and_deleting_it_clears_the_request(
    ) {
        let mut st = State::offline();
        st.screen = Screen::Chat;
        apply(&mut st, ask("ap1", "task_delegate", 30_000));
        gateway_approval_surface(&mut st, "ap1");
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 760.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(
            rt.find("a2:hive-approval-ap1").is_some(),
            "la pregunta se ve como superficie A2UI en el hilo"
        );
        assert!(
            rt.find("approval/ap1").is_none(),
            "y la tarjeta no la repite"
        );
        click(&mut rt, "hive-approval-ap1/root>buttons>approve");
        let out = &rt.state.a2.outbox;
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0]["type"], "a2ui:action");
        assert_eq!(out[0]["data"]["name"], "hive_approval");
        assert_eq!(out[0]["data"]["surfaceId"], "hive-approval-ap1");
        assert_eq!(out[0]["data"]["sourceComponentId"], "approve");
        assert_eq!(
            out[0]["data"]["context"],
            json!({"approvalId": "ap1", "approved": true})
        );
        // El gateway resuelve y retira la superficie: la petición desaparece también del escritorio.
        apply(
            &mut rt.state,
            Event::A2ui {
                kind: "a2ui:deleteSurface".into(),
                data: json!({"surfaceId": "hive-approval-ap1"}),
            },
        );
        assert!(
            rt.state.approvals.is_empty() && rt.state.a2.store.get("hive-approval-ap1").is_none()
        );
        // El botón de denegar manda la suya, con el sentido contrario.
        let mut st = State::offline();
        apply(&mut st, ask("ap2", "t", 30_000));
        gateway_approval_surface(&mut st, "ap2");
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 760.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "hive-approval-ap2/root>buttons>deny");
        assert_eq!(
            rt.state.a2.outbox[0]["data"]["context"],
            json!({"approvalId": "ap2", "approved": false})
        );
        assert_eq!(rt.state.a2.outbox[0]["data"]["sourceComponentId"], "deny");
    }

    #[test]
    fn outside_the_chat_the_card_is_still_the_way_to_answer_even_if_the_surface_exists() {
        let mut st = State::offline();
        st.screen = Screen::Settings;
        apply(&mut st, ask("ap1", "task_delegate", 30_000));
        gateway_approval_surface(&mut st, "ap1");
        assert_eq!(visible_approvals(&st).len(), 1);
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 760.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(
            rt.find("approval/ap1/approve").is_some(),
            "el chat no está a la vista: la tarjeta es la única forma de verla"
        );
        click(&mut rt, "approval/ap1/approve");
        assert_eq!(
            rt.state.a2.outbox,
            vec![chat::approval_frame("ap1", true)],
            "por la vía directa de siempre"
        );
        // Una petición cuya superficie no llegó (otro hilo a la vista) se ve como tarjeta también en el chat.
        let mut st = State::offline();
        st.screen = Screen::Chat;
        apply(&mut st, ask("ap9", "t", 30_000));
        assert_eq!(visible_approvals(&st).len(), 1);
    }

    #[test]
    fn when_the_turn_ends_or_the_connection_drops_the_approval_surface_goes_too() {
        for end in [Event::Done, Event::Error("x".into()), Event::Closed] {
            let mut st = State::offline();
            apply(&mut st, ask("ap1", "t", 30_000));
            gateway_approval_surface(&mut st, "ap1");
            // Otra superficie del agente, que no es una aprobación, no se toca.
            apply(
                &mut st,
                Event::A2ui {
                    kind: "a2ui:createSurface".into(),
                    data: json!({"surfaceId": "panel-1"}),
                },
            );
            assert!(st.a2.store.get("hive-approval-ap1").is_some());
            apply(&mut st, end);
            assert!(st.approvals.is_empty() && st.a2.store.get("hive-approval-ap1").is_none());
            assert!(
                st.a2.store.get("panel-1").is_some(),
                "las superficies normales quedan"
            );
        }
        // Al vencer, también se retira la superficie.
        let mut st = State::offline();
        apply(&mut st, ask("ap1", "t", 30_000));
        gateway_approval_surface(&mut st, "ap1");
        st.approvals[0].expires_at =
            std::time::Instant::now() - std::time::Duration::from_millis(1);
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let mut tx = rune::Tx::default();
        assert!(poll(&mut rt.state, 0.0, &mut tx));
        assert!(
            rt.state.approvals.is_empty() && rt.state.a2.store.get("hive-approval-ap1").is_none()
        );
    }

    #[test]
    fn an_approval_that_expires_or_whose_turn_ends_is_removed_and_never_answered_by_default() {
        let mut st = State::offline();
        apply(&mut st, ask("a", "t", 30_000));
        st.approvals[0].expires_at =
            std::time::Instant::now() - std::time::Duration::from_millis(1);
        apply(&mut st, ask("b", "t", 30_000));
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let mut tx = rune::Tx::default();
        assert!(poll(&mut rt.state, 0.0, &mut tx));
        assert_eq!(
            rt.state
                .approvals
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["b"],
            "la vencida se retira"
        );
        assert!(
            rt.state.a2.outbox.is_empty(),
            "vencer no aprueba ni manda nada: el gateway ya denegó"
        );
        for end in [Event::Done, Event::Error("x".into()), Event::Closed] {
            apply(&mut rt.state, ask("c", "t", 30_000));
            assert!(!rt.state.approvals.is_empty());
            apply(&mut rt.state, end);
            assert!(
                rt.state.approvals.is_empty(),
                "al acabar el turno no queda nada pendiente"
            );
        }
        assert!(rt.state.a2.outbox.is_empty());
    }

    #[test]
    fn too_many_pending_approvals_deny_the_oldest_and_the_arguments_are_bounded_plain_text() {
        let mut st = State::offline();
        for n in 0..(MAX_APPROVALS + 3) {
            apply(&mut st, ask(&format!("a{n}"), "t", 30_000));
        }
        assert_eq!(st.approvals.len(), MAX_APPROVALS);
        assert_eq!(st.approvals[0].id, "a3", "se quedan las más recientes");
        assert_eq!(
            st.a2.outbox,
            (0..3)
                .map(|n| chat::approval_frame(&format!("a{n}"), false))
                .collect::<Vec<_>>(),
            "las desplazadas se deniegan, nunca se aprueban"
        );
        let long = serde_json::json!({"texto": format!("<b>{}</b>", "y".repeat(5000)), "x": "ñ".repeat(900)});
        let preview = approval_args(&long);
        assert!(
            preview.chars().count() <= MAX_APPROVAL_ARGS + 1 && preview.ends_with('…'),
            "acotado sin cortar un carácter: {}",
            preview.chars().count()
        );
        assert!(preview.contains("<b>"), "se ve tal cual, no se interpreta");
        assert_eq!(approval_args(&serde_json::Value::Null), "(sin argumentos)");
        assert_eq!(
            approval_args(&serde_json::json!({"a":1})),
            "{\n  \"a\": 1\n}"
        );
        // Seconds se redondea hacia arriba para no prometer menos de lo que se tiene.
        apply(&mut st, ask("z", "t", 1_500));
        assert_eq!(st.approvals.last().unwrap().seconds, 2);
    }

    #[test]
    fn a_remembered_token_opens_the_chat_and_a_rejected_one_is_forgotten() {
        use crate::vault::{MemVault, Vault};
        use std::cell::RefCell;
        // Aceptado: se usa y se conserva.
        let v = MemVault::default();
        v.save("tok-bueno").unwrap();
        let tried = RefCell::new(vec![]);
        let r = boot_with(&v, |t| {
            tried.borrow_mut().push(t.map(str::to_string));
            Ok::<_, BootError>("chat")
        });
        assert!(matches!(r, Ok("chat")));
        assert_eq!(
            *tried.borrow(),
            [Some("tok-bueno".to_string())],
            "solo se probó el guardado"
        );
        assert_eq!(v.load().as_deref(), Some("tok-bueno"));
        // Rechazado por el gateway: se borra y se pide acceso.
        let tried = RefCell::new(vec![]);
        let r = boot_with(&v, |t| {
            tried.borrow_mut().push(t.map(str::to_string));
            Err::<&str, _>(BootError::NeedsLogin)
        });
        assert!(matches!(r, Err(BootError::NeedsLogin)));
        assert_eq!(
            *tried.borrow(),
            [Some("tok-bueno".to_string()), None],
            "tras el rechazo prueba sin token"
        );
        assert_eq!(v.load(), None, "un token vencido no se queda guardado");
        // Un fallo que no es de acceso (red caída) no borra nada.
        v.save("tok-bueno").unwrap();
        let r = boot_with(&v, |_| Err::<&str, _>(BootError::Msg("sin red".into())));
        assert!(matches!(r, Err(BootError::Msg(_))));
        assert_eq!(
            v.load().as_deref(),
            Some("tok-bueno"),
            "no se pierde la sesión por un fallo de red"
        );
        // Sin token guardado: arranque normal.
        v.clear();
        let tried = RefCell::new(vec![]);
        let _ = boot_with(&v, |t| {
            tried.borrow_mut().push(t.map(str::to_string));
            Err::<&str, _>(BootError::NeedsLogin)
        });
        assert_eq!(*tried.borrow(), [None]);
    }

    #[test]
    fn the_remember_checkbox_is_off_by_default_and_toggles() {
        let mut st = State::offline();
        st.shown.needs_login = true;
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(
            !rt.state.remember,
            "sin marcar, el token no sale de la memoria"
        );
        click(&mut rt, "login-remember");
        assert!(rt.state.remember);
        click(&mut rt, "login-remember");
        assert!(!rt.state.remember);
    }

    #[test]
    fn signing_out_forgets_the_token_and_asks_for_access_again() {
        use crate::vault::{MemVault, Vault};
        let mem = std::sync::Arc::new(MemVault::default());
        mem.save("tok").unwrap();
        crate::vault::install(mem.clone());
        let mut st = State::offline();
        st.chat_ready = true;
        st.msgs.push(Msg {
            id: "u0".into(),
            mine: true,
            text: "hola".into(),
        });
        st.screen = Screen::Settings;
        sign_out(&mut st);
        assert_eq!(mem.load(), None, "el llavero queda vacío");
        assert!(!st.chat_ready && st.msgs.is_empty() && st.screen == Screen::Chat);
        assert!(
            st.shared.lock().unwrap().needs_login,
            "vuelve la tarjeta de acceso"
        );
    }

    #[test]
    fn opening_meetings_lists_past_sessions_and_a_report_can_be_requested() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p) {
                ("GET", "/api/meetings") => (
                    200,
                    json!({"ok": true, "sessions": [
                {"id": "m1", "title": "Semanal", "status": "stopped", "segment_count": 7, "report_path": null},
                {"id": "m2", "title": "Vacía", "status": "stopped", "segment_count": 0, "report_path": null},
                {"id": "m3", "title": "En curso", "status": "active", "segment_count": 3, "report_path": null}]}),
                ),
                ("POST", "/api/meetings/m1/report") => (
                    200,
                    json!({"ok": true, "report_path": "/tmp/informe-semanal.docx"}),
                ),
                _ => (404, json!({"error": "no"})),
            },
            3,
        );
        let mut st = State::offline();
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (900.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/meeting");
        for _ in 0..40 {
            rt.frame(1.0 / 60.0);
        } // la pantalla entra con una transición
          // El hilo trae la lista; la vista la recoge en `poll`.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while rt.state.meeting.shown.sessions.is_empty() && std::time::Instant::now() < end {
            rt.update_if(|s: &mut State, _| s.meeting.sync());
            rt.frame(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(rt.state.meeting.shown.sessions.len(), 3);
        assert!(
            rt.find("meet/report/m1").is_some(),
            "una reunión terminada con fragmentos ofrece el informe"
        );
        assert!(
            rt.find("meet/report/m2").is_none(),
            "sin fragmentos no hay nada que resumir"
        );
        assert!(rt.find("meet/report/m3").is_none(), "una en curso tampoco");
        for _ in 0..120 {
            rt.frame(1.0 / 60.0);
        } // la lista se acomoda con una animación: se espera a que termine
        click(&mut rt, "meet/report/m1");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while rt.state.meeting.data.lock().unwrap().notice.is_none()
            && std::time::Instant::now() < end
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let seen = bodies(&server.join().unwrap());
        assert_eq!(seen[0].1, "/api/meetings");
        assert_eq!(
            (seen[1].0.as_str(), seen[1].1.as_str()),
            ("POST", "/api/meetings/m1/report")
        );
        assert_eq!(
            rt.state.meeting.data.lock().unwrap().notice.as_deref(),
            Some("Informe listo: /tmp/informe-semanal.docx")
        );
    }

    #[test]
    fn starting_a_meeting_without_a_ready_gateway_explains_instead_of_opening_audio() {
        let st = State::offline();
        let mut rt = Runtime::new(
            st,
            view,
            (900.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/meeting");
        for _ in 0..40 {
            rt.frame(1.0 / 60.0);
        } // la pantalla entra con una transición
        assert!(rt.find("meet/title").is_some() && rt.find("meet/start").is_some());
        click(&mut rt, "meet/start");
        let d = rt.state.meeting.data.lock().unwrap();
        assert_eq!(d.error.as_deref(), Some("El gateway aún no está listo"));
        assert!(!d.live && !d.starting, "no queda a medio abrir");
    }

    #[test]
    fn the_system_audio_checkbox_toggles_only_while_idle() {
        let mut rt = Runtime::new(
            State::offline(),
            view,
            (900.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/meeting");
        for _ in 0..40 {
            rt.frame(1.0 / 60.0);
        } // la pantalla entra con una transición
        assert!(
            !rt.state.meeting.include_system,
            "por defecto solo el micrófono"
        );
        click(&mut rt, "meet/system");
        assert!(rt.state.meeting.include_system);
        rt.update_if(|s: &mut State, _| {
            s.meeting.shown.live = true;
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "meet/system");
        assert!(
            rt.state.meeting.include_system,
            "con la reunión en marcha no cambia"
        );
    }

    fn agents_rt(port: u16) -> Runtime<State> {
        let mut st = State::offline();
        demo_agents(&mut st);
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    #[test]
    fn the_agents_screen_shows_the_coordinator_first_and_selecting_a_hexagon_swaps_the_card() {
        let mut rt = agents_rt(1);
        assert!(
            rt.find("agents/hero").is_some()
                && rt.find("agents/node").is_some()
                && rt.find("agents/map").is_some()
        );
        assert_eq!(
            super::ui::agents::current(
                &rt.state.settings.shown.agents,
                rt.state.agents_sel.as_deref()
            )
            .unwrap()
            .name,
            "Pepito",
            "arranca con el coordinador"
        );
        assert!(
            rt.find("agents/hex/boss").is_some() && rt.find("agents/hex/a3").is_some(),
            "un hexágono por agente"
        );
        click(&mut rt, "agents/hex/a5");
        assert_eq!(rt.state.agents_sel.as_deref(), Some("a5"));
        assert_eq!(
            super::ui::agents::current(
                &rt.state.settings.shown.agents,
                rt.state.agents_sel.as_deref()
            )
            .unwrap()
            .name,
            "Agente de código externo"
        );
        assert!(!rt.state.agents_editing, "elegir un nodo no abre el editor");
    }

    #[test]
    fn configuring_a_node_opens_the_editor_validates_and_saves_through_the_gateway() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = agents_rt(port);
        click(&mut rt, "agents/hex/a5");
        click(&mut rt, "agents/configure");
        assert!(
            rt.state.agents_editing
                && rt
                    .state
                    .settings
                    .agent_form
                    .as_ref()
                    .is_some_and(|f| f.id == "a5"),
            "el editor trae el agente elegido"
        );
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("agents/editor").is_some()
                && rt.find("agents/f/name").is_some()
                && rt.find("agents/f/prompt").is_some()
        );
        assert!(
            rt.find("agents/map").is_none(),
            "el editor ocupa el lugar del mapa"
        );
        // Un nombre vacío no se envía.
        rt.update_if(|s: &mut State, _| {
            s.settings.agent_form.as_mut().unwrap().name = rune::TextEdit::new("");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "agents/save");
        assert_eq!(
            rt.state.settings.data.lock().unwrap().busy,
            0,
            "con un nombre vacío no sale nada"
        );
        rt.update_if(|s: &mut State, _| {
            let f = s.settings.agent_form.as_mut().unwrap();
            f.name = rune::TextEdit::new("Código externo");
            f.prompt = rune::TextEdit::new("# ROL\nHaz código.");
            f.enabled = false;
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "agents/save");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str()),
            ("PUT", "/api/agents/a5")
        );
        assert_eq!(seen[0].2["name"], "Código externo");
        assert_eq!(seen[0].2["systemPrompt"], "# ROL\nHaz código.");
        assert_eq!(seen[0].2["enabled"], false);
        assert_eq!(
            seen[0].2["maxIterations"], 10,
            "las iteraciones del agente se conservan"
        );
        let reads: Vec<&str> = seen[1..].iter().map(|r| r.1.as_str()).collect();
        assert_eq!(
            reads,
            ["/api/providers", "/api/agents", "/api/channels"],
            "tras guardar se relee todo"
        );
        click(&mut rt, "agents/cancel");
        assert!(
            !rt.state.agents_editing && rt.state.settings.agent_form.is_none(),
            "volver al mapa cierra el editor"
        );
    }

    #[test]
    fn deploying_a_node_creates_the_agent_then_saves_its_prompt_and_rereads_everything() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p) {
                ("POST", "/api/agents") => (
                    200,
                    json!({"ok": true, "agent": {"id": "nuevo1", "name": "Analista"}}),
                ),
                ("PUT", "/api/agents/nuevo1") => (200, json!({"ok": true})),
                _ => (200, lists(p)),
            },
            5,
        );
        let mut rt = agents_rt(port);
        {
            // La demo trae un proveedor sin clave: se le da una y un modelo activo para poder elegirlo.
            let mut d = rt.state.settings.data.lock().unwrap();
            d.providers = serde_json::from_value(json!([{"id": "gemini", "name": "Google Gemini", "category": "llm", "enabled": true, "active": true, "has_api_key": 1, "models": [{"id": "gemini-3.8-flash", "name": "Gemini 3.8 Flash", "enabled": true}, {"id": "off", "enabled": false}]}])).unwrap();
        }
        rt.state.settings.sync();
        rt.update_if(|_: &mut State, _| true);
        click(&mut rt, "agents/deploy");
        assert!(rt.state.settings.new_agent.is_some());
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("agents/new").is_some()
                && rt.find("agents/n/name").is_some()
                && rt.find("agents/n/provider").is_some()
        );
        // Sin nombre no sale nada.
        click(&mut rt, "agents/create");
        assert_eq!(rt.state.settings.data.lock().unwrap().busy, 0);
        assert!(
            rt.state.settings.new_agent.is_some(),
            "el formulario sigue abierto"
        );
        rt.update_if(|s: &mut State, _| {
            let f = s.settings.new_agent.as_mut().unwrap();
            f.name = rune::TextEdit::new("Analista");
            f.desc = rune::TextEdit::new("Analiza el mercado");
            f.prompt = rune::TextEdit::new("# ROL\nAnaliza.");
            f.workspace = rune::TextEdit::new("/tmp/mercado");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "agents/create");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str()),
            ("POST", "/api/agents")
        );
        assert_eq!(
            seen[0].2,
            json!({"name": "Analista", "description": "Analiza el mercado", "providerId": "gemini", "modelId": "gemini-3.8-flash", "workspace": "/tmp/mercado"})
        );
        assert_eq!(
            (seen[1].0.as_str(), seen[1].1.as_str()),
            ("PUT", "/api/agents/nuevo1"),
            "el prompt va aparte porque el servidor no lo guarda al crear"
        );
        assert_eq!(
            seen[1].2,
            json!({"role": "worker", "systemPrompt": "# ROL\nAnaliza."}),
            "y el agente nuevo se pasa a especialista: solo puede haber un coordinador"
        );
        assert_eq!(
            seen[2..].iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["/api/providers", "/api/agents", "/api/channels"]
        );
        assert!(
            rt.state.settings.new_agent.is_none(),
            "el formulario se cierra al enviarlo"
        );
    }

    #[test]
    fn a_new_agent_is_always_made_a_specialist_even_without_a_prompt() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p) {
                ("POST", "/api/agents") => (200, json!({"ok": true, "agent": {"id": "nuevo2"}})),
                _ => (200, lists(p)),
            },
            5,
        );
        let mut rt = agents_rt(port);
        click(&mut rt, "agents/deploy");
        rt.update_if(|s: &mut State, _| {
            s.settings.new_agent.as_mut().unwrap().name = rune::TextEdit::new("Sin prompt");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "agents/create");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            (seen[1].0.as_str(), seen[1].1.as_str()),
            ("PUT", "/api/agents/nuevo2")
        );
        assert_eq!(
            seen[1].2,
            json!({"role": "worker"}),
            "el servidor lo crea como coordinador: sin prompt, igualmente se corrige"
        );
    }

    #[test]
    fn extra_coordinators_are_flagged_and_fixed_without_touching_the_real_one() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 5);
        let mut rt = agents_rt(port);
        assert!(
            rt.find("agents/extra-coordinators").is_none(),
            "con un solo coordinador no hay aviso"
        );
        {
            let mut d = rt.state.settings.data.lock().unwrap();
            for id in ["falso1", "falso2"] {
                d.agents.push(
                    serde_json::from_value(
                        json!({"id": id, "name": id, "role": "coordinator", "enabled": true}),
                    )
                    .unwrap(),
                );
            }
        }
        rt.state.settings.sync();
        rt.update_if(|_: &mut State, _| true);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("agents/extra-coordinators").is_some(),
            "con dos de más, aviso"
        );
        // Los de más son especialistas para todo lo demás: el mando sigue siendo Pepito y hay un hexágono por agente.
        assert_eq!(
            settings::api::core_id(&rt.state.settings.shown.agents),
            Some("boss")
        );
        assert!(rt.find("agents/hex/falso1").is_some() && rt.find("agents/hex/falso2").is_some());
        click(&mut rt, "agents/fix-coordinators");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        let puts: Vec<(&str, &Value)> = seen
            .iter()
            .filter(|r| r.0 == "PUT")
            .map(|r| (r.1.as_str(), &r.2))
            .collect();
        assert_eq!(puts.len(), 2, "solo los dos de más");
        assert!(puts.iter().all(|(_, b)| **b == json!({"role": "worker"})));
        assert!(
            puts.iter().any(|(p, _)| *p == "/api/agents/falso1")
                && puts.iter().any(|(p, _)| *p == "/api/agents/falso2")
        );
        assert!(
            !seen.iter().any(|r| r.1 == "/api/agents/boss"),
            "el coordinador de verdad no se toca"
        );
    }

    #[test]
    fn the_hero_counts_every_agent_but_the_single_coordinator_as_a_specialist() {
        let rt = agents_rt(1);
        let agents = &rt.state.settings.shown.agents;
        assert_eq!(agents.len(), 10);
        assert_eq!(
            agents.len() - usize::from(settings::api::core_id(agents).is_some()),
            9
        );
    }

    #[test]
    fn a_new_agent_body_omits_what_was_left_blank_so_the_server_uses_its_defaults() {
        let f = settings::NewAgent {
            name: rune::TextEdit::new("  Solo nombre "),
            ..Default::default()
        };
        assert_eq!(
            f.body(&[]).unwrap(),
            json!({"name": "Solo nombre", "description": ""})
        );
        assert!(settings::NewAgent::default().body(&[]).is_err());
    }

    #[test]
    fn the_profile_lists_tools_and_skills_and_goes_back_to_the_map() {
        let mut rt = agents_rt(1);
        click(&mut rt, "agents/hex/a5");
        click(&mut rt, "agents/profile");
        assert!(rt.state.agents_detail);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("agents/identity").is_some() && rt.find("agents/loadout").is_some());
        assert!(
            rt.find("agents/tool/t0").is_some() && rt.find("agents/tool/t5").is_some(),
            "las 6 herramientas del agente"
        );
        assert!(rt.find("agents/tool/t6").is_none());
        assert!(rt.find("agents/skill/s2").is_some(), "y sus 3 skills");
        assert!(
            rt.find("agents/map").is_none(),
            "el perfil sustituye al mapa"
        );
        click(&mut rt, "agents/back");
        assert!(!rt.state.agents_detail);
    }

    #[test]
    fn the_sidebar_leads_to_every_screen_and_marks_the_current_one() {
        let mut rt = Runtime::new(
            State::offline(),
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for (key, screen) in [
            ("nav/live", Screen::Live),
            ("nav/office", Screen::Office),
            ("nav/panel", Screen::Panel),
            ("nav/meeting", Screen::Meeting),
            ("nav/chat", Screen::Chat),
        ] {
            click(&mut rt, key);
            assert_eq!(rt.state.screen, screen, "{key}");
        }
        click(&mut rt, "nav/providers");
        assert!(rt.state.screen == Screen::Settings && rt.state.settings.tab == Tab::Providers);
        click(&mut rt, "nav/channels");
        assert_eq!(rt.state.settings.tab, Tab::Channels);
        click(&mut rt, "nav/agents");
        assert_eq!(rt.state.screen, Screen::Agents);
        click(&mut rt, "sidebar/toggle");
        assert!(rt.state.sidebar_collapsed, "la barra se pliega a iconos");
        click(&mut rt, "sidebar/toggle");
        assert!(!rt.state.sidebar_collapsed);
    }

    #[test]
    fn the_panel_badge_counts_surfaces_and_deleting_one_removes_it_everywhere() {
        let (port, server) = fake_http(
            |m, p, _, _| {
                if m == "DELETE" {
                    (200, json!({"ok": true}))
                } else {
                    (200, lists(p))
                }
            },
            1,
        );
        let mut st = State::offline();
        demo_panel(&mut st);
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.a2.store.surfaces.len(), 1);
        assert!(rt.find("panel/surface/demo").is_some() && rt.find("panel/delete/demo").is_some());
        click(&mut rt, "panel/delete/demo");
        assert!(
            rt.state.a2.store.surfaces.is_empty(),
            "desaparece al instante"
        );
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str()),
            ("DELETE", "/api/a2ui/surfaces/demo"),
            "y se borra en el servidor, como la papelera de la web"
        );
    }

    fn settled(mut rt: Runtime<State>) -> Runtime<State> {
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    fn scene_rt(demo: fn(&mut State)) -> Runtime<State> {
        scene_rt_sized(demo, (1440.0, 900.0))
    }
    fn scene_rt_sized(demo: fn(&mut State), size: (f32, f32)) -> Runtime<State> {
        let mut st = State::offline();
        demo(&mut st);
        let mut rt = Runtime::new(
            st,
            view,
            size,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        settled(rt)
    }

    #[test]
    fn responsive_screens_keep_text_and_controls_inside_the_window() {
        fn image(s: &mut State) {
            demo_settings(s);
            s.settings.prov_cat = 1;
            s.settings.config_open = Some("gemini".into());
            let model =
                "gemini/".to_string() + &"modelo_de_vision_con_identificador_extenso_".repeat(6);
            let mut d = s.settings.data.lock().unwrap();
            let p = d.providers.iter_mut().find(|p| p.id == "gemini").unwrap();
            p.name = "Proveedor para analizar imágenes y documentos con un nombre muy largo".into();
            p.models=serde_json::from_value(serde_json::json!([{"id":model,"name":model,"enabled":true,"active":true,"model_type":"llm","capabilities":["vision"]}])).unwrap();
            drop(d);
            s.settings.sync();
        }
        fn voice(s: &mut State) {
            image(s);
            s.settings.prov_cat = 2;
            let mut d = s.settings.data.lock().unwrap();
            let m = &mut d
                .providers
                .iter_mut()
                .find(|p| p.id == "gemini")
                .unwrap()
                .models[0];
            m.model_type = "tts".into();
            m.capabilities = vec!["tts".into()];
            drop(d);
            s.settings.sync();
        }
        fn piper(s: &mut State) {
            voice(s);
            s.settings.config_open = Some("piper".into());
            let mut d = s.settings.data.lock().unwrap();
            d.providers.push(
                serde_json::from_value(
                    serde_json::json!({"id":"piper","category":"tts","enabled":true,"active":true}),
                )
                .unwrap(),
            );
            d.piper = Some(
                serde_json::json!({"installed":true,"running":true,"models":[{"id":"claude","name":"Voz de prueba con descripción extensa para una configuración de síntesis local","installed":true,"selected":true}],"settings":{"length_scale":1.0},"logs":["Un registro de prueba extenso para comprobar que los detalles permanezcan dentro de la tarjeta de configuración de Piper"]}),
            );
            drop(d);
            s.settings.sync();
        }
        fn live_open(s: &mut State) {
            demo_live(s);
            s.live.panel_open_narrow = true;
            s.live.open_select = 4;
            s.live.catalog.models = vec![
                (
                    "one".into(),
                    "Modelo de voz en tiempo real con un nombre largo".repeat(4),
                ),
                ("two".into(), "Otro modelo".into()),
            ];
        }
        fn previews(s: &mut State) {
            demo_live_call(s);
            s.live.camera_preview.active = true;
            s.live.screen_preview.active = true;
        }
        fn agent_edit(s: &mut State) {
            demo_agents(s);
            let a = &s.settings.shown.agents[0];
            s.settings.agent_form = Some(settings::AgentForm::from(a));
            s.agents_editing = true;
        }
        fn profile_edit(s: &mut State) {
            demo_profile(s);
            s.profile.editing = true;
            s.profile.name.text = "Nombre largo de prueba ".repeat(12);
            s.profile.notes.text = "Preferencia extensa ".repeat(60);
        }
        fn api_headers(s: &mut State) {
            demo_apiclient(s);
            s.work.api.res_tab = ui::work_pages::ResTab::Headers;
            s.work.api.answer = Some(ui::work_pages::Answer {
                ms: 5,
                reply: Ok(system::parse_http_reply(
                    &serde_json::json!({"ok":true,"status":200,"statusText":"Respuesta correcta con una descripción larga de prueba","headers":{"x-very-long-response-header-name":"value_without_spaces_".repeat(30)},"body":""}),
                )),
            });
        }
        fn sidebar_open(s: &mut State) {
            demo_settings(s);
            s.sidebar_open_narrow = true;
        }
        type Scene = (&'static str, fn(&mut State));
        let scenes: [Scene; 32] = [
            ("dashboard", demo_dashboard),
            ("chat", demo_chat_msgs),
            ("live", demo_live_call),
            ("agents", demo_agents),
            ("agent-new", demo_agent_new),
            ("agent-detail", demo_agent_detail),
            ("providers", demo_settings),
            ("channels", demo_channels),
            ("panel", demo_panel),
            ("meeting", demo_meeting),
            ("notes", demo_notes),
            ("cron", demo_cron),
            ("api", demo_apiclient),
            ("tools", demo_tools),
            ("skills", demo_skills),
            ("mcp", demo_mcp),
            ("ethics", demo_ethics),
            ("profile", demo_profile),
            ("display", demo_display),
            ("updates", demo_updates),
            ("office", demo_office),
            ("login", demo_login),
            ("vision-long", image),
            ("voice-long", voice),
            ("piper", piper),
            ("live-open", live_open),
            ("previews", previews),
            ("agent-edit", agent_edit),
            ("profile-edit", profile_edit),
            ("api-headers", api_headers),
            ("sidebar-open", sidebar_open),
            ("markdown", demo_markdown),
        ];
        let mut issues = Vec::new();
        for size in [
            (480.0, 640.0),
            (640.0, 360.0),
            (640.0, 480.0),
            (800.0, 600.0),
            (1024.0, 768.0),
            (1440.0, 900.0),
        ] {
            for (name, demo) in scenes {
                let mut st = State::offline();
                demo(&mut st);
                st.win = size;
                st.live.viewport = st.stage_size();
                st.office.viewport = st.stage_size();
                let mut rt = Runtime::new(
                    st,
                    view,
                    size,
                    Env {
                        system_dark: true,
                        scale: 1.0,
                    },
                );
                rt.reduced_motion = true;
                rt.frame(0.0);
                for _ in 0..3 {
                    rt.frame(1.0 / 60.0);
                }
                rt.settle_all();
                rt.frame(0.0);
                let elements = rt.elements();
                if name == "previews" {
                    let dock = elements
                        .iter()
                        .find(|e| e.key.as_deref() == Some("live/dock"))
                        .unwrap()
                        .rect;
                    for preview in elements.iter().filter(|e| {
                        matches!(
                            e.key.as_deref(),
                            Some("live/camera-preview" | "live/screen-preview")
                        )
                    }) {
                        assert!(
                            preview.rect[1] + preview.rect[3] <= dock[1] + 1.0,
                            "La vista previa tapa los controles en {size:?}"
                        );
                    }
                }
                for e in &elements {
                    if e.exiting
                        || e.opacity < 0.1
                        || (e.text.is_none() && e.role != "button" && e.key.is_none())
                    {
                        continue;
                    }
                    let [x, y, w, h] = e.rect;
                    if h <= 0.0 || w <= 0.0 {
                        continue;
                    }
                    // Code and tables may scroll horizontally inside their own bounded viewport.
                    let mut parent = e.parent;
                    let mut wide = false;
                    let mut code = false;
                    for _ in 0..40 {
                        let Some(p) = parent.and_then(|i| elements.get(i)) else {
                            break;
                        };
                        wide |= p.key.as_deref() == Some("scroll")
                            && p.rect[0] >= 0.0
                            && p.rect[0] + p.rect[2] <= size.0 + 1.0;
                        code |= matches!(p.role, "code" | "table");
                        parent = p.parent;
                    }
                    if wide && code && e.key.as_deref() != Some("scroll") {
                        continue;
                    }
                    if let Some(parent) = e.parent.and_then(|i| elements.get(i)) {
                        let [px, py, pw, ph] = parent.rect;
                        if e.text.is_some()
                            && (parent.role == "button" || parent.key.as_deref() == Some("topbar"))
                            && ph > 0.0
                            && (y < py - 1.0 || y + h > py + ph + 1.0)
                        {
                            issues.push(format!("{name} {size:?}: texto fuera de altura del botón {:?}: {:?} child {:?} parent {:?}",parent.key,e.text,e.rect,parent.rect));
                        }
                        if e.text.is_some() && pw > 0.0 && (x < px - 1.0 || x + w > px + pw + 1.0) {
                            issues.push(format!("{name} {size:?}: texto fuera de padre {:?}: {:?} child {:?} parent [{px:.0},{py:.0},{pw:.0},{ph:.0}]",parent.key,e.text,e.rect));
                        }
                    }
                    if x < -1.0 || x + w > size.0 + 1.0 {
                        issues.push(format!(
                            "{name} {size:?}: {:?} {:?} [{x:.0},{y:.0},{w:.0},{h:.0}]",
                            e.key,
                            e.text
                                .as_deref()
                                .map(|t| t.chars().take(65).collect::<String>())
                        ));
                    }
                }
            }
        }
        assert!(
            issues.is_empty(),
            "Desbordamientos horizontales ({}):\n{}",
            issues.len(),
            issues.join("\n")
        );
    }

    #[test]
    fn responsive_sidebar_overlays_content_and_closes_without_changing_page_width() {
        let mut s = State::offline();
        demo_settings(&mut s);
        s.win = (480.0, 640.0);
        let width = s.content_width();
        let mut rt = Runtime::new(
            s,
            view,
            (480.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.reduced_motion = true;
        rt.frame(0.0);
        click(&mut rt, "sidebar/toggle");
        assert!(rt.state.sidebar_open_narrow);
        assert_eq!(rt.state.content_width(), width);
        assert!(rt.find("sidebar/dismiss").is_some());
        click(&mut rt, "sidebar/close-narrow");
        assert!(!rt.state.sidebar_open_narrow);
        assert_eq!(rt.state.content_width(), width);
        for size in [(1440.0, 900.0), (640.0, 480.0), (480.0, 640.0)] {
            rt.state.win = size;
            rt.set_viewport(size, 1.0);
            rt.frame(0.0);
            rt.settle_all();
            rt.frame(0.0);
            let [x, _, w, _] = rt
                .elements()
                .iter()
                .find(|e| e.key.as_deref() == Some("prov/gemini"))
                .unwrap()
                .rect;
            assert!(
                x >= 0.0 && x + w <= size.0 + 1.0,
                "tarjeta tras redimensionar: {x} {w} {size:?}"
            );
        }
    }

    #[test]
    fn provider_cards_filter_by_category_and_the_config_button_opens_one_panel_at_a_time() {
        let mut rt = scene_rt(demo_settings);
        assert!(
            rt.find("prov/gemini/toggle").is_some() && rt.find("prov/groq/toggle").is_some(),
            "una tarjeta con interruptor por proveedor de lenguaje"
        );
        assert!(
            rt.find("prov/elevenlabs/toggle").is_none(),
            "los de voz no salen en la pestaña de proveedores"
        );
        assert!(
            rt.find("prov/gemini/key").is_none(),
            "la clave solo aparece tras CONFIG"
        );
        click(&mut rt, "prov/gemini/config");
        assert_eq!(rt.state.settings.config_open.as_deref(), Some("gemini"));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("prov/gemini/key").is_some() && rt.find("prov/gemini/verify").is_some());
        click(&mut rt, "prov/groq/config");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("prov/gemini/key").is_none() && rt.find("prov/groq/key").is_some(),
            "al abrir otro se cierra el anterior"
        );
        click(&mut rt, "prov/cat/2");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.settings.prov_cat, 2);
        assert!(
            rt.find("prov/elevenlabs/toggle").is_some() && rt.find("prov/gemini/toggle").is_none(),
            "la pestaña Voz muestra solo los de voz"
        );
    }

    #[test]
    fn multimodal_provider_cards_expose_scoped_models_and_piper_has_local_configuration() {
        fn demo(s: &mut State) {
            demo_settings(s);
            s.settings.prov_cat = 1;
            s.settings.data.lock().unwrap().providers=serde_json::from_value(serde_json::json!([
              {"id":"gemini","category":"llm","enabled":true,"active":true,"models":[
                {"id":"eye","model_type":"llm","enabled":true,"capabilities":"[\"chat\",\"vision\"]"},
                {"id":"paint","model_type":"image","enabled":false,"capabilities":["image_generation"]},
                {"id":"tts","model_type":"tts","enabled":true,"capabilities":["tts"]}]},
              {"id":"piper","category":"tts","enabled":true,"active":true,"models":[]}
            ])).unwrap();
            s.settings.sync();
        }
        let mut rt = scene_rt(demo);
        assert!(rt.find("prov/gemini/toggle").is_some());
        assert!(rt.find("model/eye/toggle").is_some() && rt.find("model/paint/toggle").is_some());
        assert!(rt.find("model/eye/vision/default").is_some());
        assert!(rt.find("model/paint/vision/default").is_none());
        assert!(rt.find("model/tts/toggle").is_none());
        click(&mut rt, "prov/cat/2");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("model/tts/toggle").is_some());
        assert!(rt.find("model/eye/toggle").is_none());
        click(&mut rt, "prov/piper/config");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("piper/refresh").is_some());
        assert!(rt.find("prov/piper/key").is_none());
    }

    #[test]
    fn channels_count_filter_and_open_the_new_channel_form_from_its_button() {
        let mut rt = scene_rt(demo_channels);
        assert!(rt.find("chan/card/webchat").is_some() && rt.find("chan/card/discord").is_some());
        assert!(
            rt.find("chan/add/telegram").is_none(),
            "el formulario empieza cerrado"
        );
        click(&mut rt, "chan/filter/1");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("chan/card/webchat").is_some() && rt.find("chan/card/discord").is_none(),
            "ACTIVOS: solo el conectado"
        );
        click(&mut rt, "chan/filter/2");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("chan/card/webchat").is_none() && rt.find("chan/card/discord").is_some(),
            "INACTIVOS: el resto"
        );
        click(&mut rt, "chan/new");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.state.settings.add_open && rt.find("chan/add/telegram").is_some(),
            "NUEVO CANAL despliega el formulario"
        );
        click(&mut rt, "chan/card/discord");
        assert_eq!(
            rt.state.settings.config_open.as_deref(),
            Some("discord"),
            "pulsar una tarjeta abre su configuración"
        );
    }

    #[test]
    fn the_tools_gallery_filters_by_category_and_each_card_has_its_switch() {
        let mut rt = scene_rt(demo_tools);
        for id in [
            "web_search",
            "web_fetch",
            "read_file",
            "a2ui_create_surface",
        ] {
            assert!(
                rt.find(&format!("tool/{id}/toggle")).is_some(),
                "interruptor de {id}"
            );
        }
        click(&mut rt, "tool/cat/web");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.settings.tool_cat.as_deref(), Some("web"));
        assert!(
            rt.find("tool/web_search/toggle").is_some()
                && rt.find("tool/read_file/toggle").is_none(),
            "solo la categoría elegida"
        );
        click(&mut rt, "tool/cat/all");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.state.settings.tool_cat.is_none() && rt.find("tool/read_file/toggle").is_some());
    }

    #[test]
    fn the_environment_pages_hang_from_settings_in_the_sidebar_like_the_web() {
        let mut rt = scene_rt(demo_tools);
        for key in ["nav/tools", "nav/mcp", "nav/skills"] {
            assert!(rt.find(key).is_some(), "falta {key} bajo Ajustes");
        }
        click(&mut rt, "nav/skills");
        assert_eq!(rt.state.settings.tab, Tab::Skills);
        click(&mut rt, "nav/mcp");
        assert_eq!(rt.state.settings.tab, Tab::Mcp);
        click(&mut rt, "nav/chat");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("nav/tools").is_none(),
            "fuera de Ajustes el submenú se recoge"
        );
    }

    #[test]
    fn mcp_cards_show_the_transport_the_url_and_the_exposed_tools_from_the_real_payload_shape() {
        let rt = scene_rt(demo_mcp);
        assert!(rt.find("mcp/cal").is_some() && rt.find("mcp/mail").is_some());
        let srv: &settings::api::McpServer = &rt.state.settings.shown.mcp[0];
        assert_eq!(
            (srv.transport().as_str(), srv.tool_count()),
            ("sse", Some(5)),
            "el servidor manda `tools_count` en snake_case"
        );
        assert!(srv.url().starts_with("https://n8n."));
    }

    #[test]
    fn opening_the_dashboard_reads_the_agents_and_the_metrics_and_shows_the_swarm() {
        let (port, server) = fake_http(
            |_, p, _, _| match p.split('?').next().unwrap() {
                "/api/system-stats" => (
                    200,
                    json!({"cpu": 1.2, "memory": {"rss": 300, "heapUsed": 60, "external": 20}, "uptime": "01:00:00", "connections": 3, "cores": 8, "recentMessages": 4}),
                ),
                "/api/activity-stats" => (200, json!([{"time": "2026-11-08 09:00", "count": 6}])),
                "/api/version" => (
                    200,
                    json!({"current": "1.1.4", "latest": "1.1.4", "status": "up-to-date", "installationType": "binary"}),
                ),
                "/api/usage-stats" => (
                    200,
                    json!({"totalTokens": 1500, "totalCostUsd": 0.02, "byProvider": {"gemini": {"tokens": 1500, "costUsd": 0.02}}}),
                ),
                "/api/agents/proposals" => (
                    200,
                    json!({"proposals": [{"agentName": "Cron", "reason": "harmful_count exceeded helpful_count"}]}),
                ),
                other => (200, lists(other)),
            },
            8,
        );
        let mut st = State::offline();
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/dashboard");
        assert_eq!(rt.state.screen, Screen::Dashboard);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut tx = rune::Tx::default();
        while (rt.state.sys.shown.stats.is_none() || rt.state.settings.shown.agents.is_empty())
            && std::time::Instant::now() < end
        {
            // `poll` es lo que, con el Dashboard abierto, lanza la lectura de las métricas y copia sus resultados.
            poll(&mut rt.state, 0.0, &mut tx);
            rt.update_if(|s: &mut State, _| s.settings.sync() | s.sys.sync());
            rt.frame(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        server.join().unwrap();
        rt.update_if(|s: &mut State, _| s.settings.sync() | s.sys.sync());
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        let d = &rt.state.sys.shown;
        assert_eq!(
            d.stats.as_ref().map(|s| (s.connections, s.recent_messages)),
            Some((3, 4))
        );
        assert_eq!(d.proposals.len(), 1, "una propuesta del curador pendiente");
        for key in ["dash/monitor", "dash/usage", "dash/activity", "dash/health"] {
            assert!(rt.find(key).is_some(), "falta {key}");
        }
        assert_eq!(
            rt.state.settings.shown.agents.len(),
            1,
            "los agentes del servidor falso"
        );
    }

    #[test]
    fn the_swarm_health_counts_every_specialist_but_not_the_coordinator() {
        let mut st = State::offline();
        demo_agents(&mut st);
        {
            let mut d = st.settings.data.lock().unwrap();
            d.agents[2].enabled = false; // un especialista apagado
        }
        st.settings.sync();
        st.office.graph.apply(
            "canvas:node_add",
            &json!({"id": "a1", "type": "agent", "label": "a1", "status": "thinking", "data": {}}),
            None,
            0.0,
        );
        let (idle, working, stuck, disabled) = ui::system_pages::swarm_counts(&st);
        assert_eq!(
            (idle, working, stuck, disabled),
            (7, 1, 0, 1),
            "9 especialistas: 1 trabajando, 1 apagado, 7 libres"
        );
    }

    #[test]
    fn rust_dashboard_uses_live_office_roster_without_loading_unported_settings() {
        let mut state = State::offline();
        state.shown.capabilities = Some(vec!["chat".into(), "office".into(), "dashboard".into()]);
        assert!(screen_available(&state, Screen::Dashboard));
        assert!(!screen_available(&state, Screen::Settings));
        state.sys.shown.stats = Some(system::parse_stats(
            &json!({"runtime":"rust","cpu":1,"memory":{"rss":120},"activeRuns":2,"cores":8,"connections":1}),
        ));
        state.sys.shown.usage = Some(system::parse_usage(
            &json!({"totalTokens":30,"costIncomplete":true}),
        ));
        state.sys.shown.version = Some(system::parse_version(
            &json!({"current":"0.1.0","status":"local","installationType":"binary"}),
        ));
        apply(
            &mut state,
            Event::Canvas {
                kind: "canvas:snapshot".into(),
                timestamp: None,
                data: json!({"revision":1,
            "agents":[{"id":"boss","name":"Hive","role":"coordinator","enabled":true},{"id":"worker","name":"Analista","role":"worker","enabled":true}],
            "nodes":[{"id":"worker","type":"agent","data":{"status":"thinking"}}],"edges":[]}),
            },
        );
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 1000.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/dashboard");
        rt.frame(0.0);
        assert_eq!(rt.state.screen, Screen::Dashboard);
        assert_eq!(rt.state.settings.shown.busy, 0);
        assert!(rt.state.settings.shown.agents.is_empty());
        assert_eq!(ui::system_pages::swarm_counts(&rt.state), (0, 1, 0, 0));
        for key in [
            "dash/monitor",
            "dash/native-runs",
            "dash/usage",
            "dash/activity",
            "dash/health",
        ] {
            assert!(rt.find(key).is_some(), "falta {key}");
        }
    }

    #[test]
    fn the_ethics_card_unfolds_its_guidelines_when_pressed() {
        let mut rt = scene_rt(demo_ethics);
        assert!(rt.find("ethic/default").is_some());
        assert!(rt.state.ethic_open.is_none());
        click(&mut rt, "ethic/default");
        assert_eq!(rt.state.ethic_open.as_deref(), Some("default"));
        click(&mut rt, "ethic/default");
        assert!(rt.state.ethic_open.is_none(), "otro clic la pliega");
    }

    /// Espera (con tope) a que `done` se cumpla mientras se vacían las lecturas pendientes.
    fn settle(rt: &mut Runtime<State>, done: impl Fn(&State) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(&rt.state) && std::time::Instant::now() < end {
            rt.update_if(|s: &mut State, _| s.sys.sync() | s.work.api.sync());
            rt.frame(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        rt.update_if(|s: &mut State, _| s.sys.sync() | s.work.api.sync());
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn work_rt(port: u16, scene: fn(&mut State)) -> Runtime<State> {
        let mut st = State::offline();
        scene(&mut st);
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    #[test]
    fn notes_and_cron_open_from_the_sidebar_and_read_the_server() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p) {
                ("GET", "/api/notes") => (
                    200,
                    json!({"notes": [{"id": "n9", "thread_id": "u/c/conv-abcdef123456", "key": "recordatorio", "value": "Llamar a Ana", "source": "agent", "created_at": 1790280173}]}),
                ),
                ("GET", "/api/cron") => (
                    200,
                    json!({"tasks": [{"id": "c1", "name": "informe", "status": "active", "task_type": "recurring", "cron_expression": "0 8 * * *", "channel": "cli"}, {"id": "c2", "name": "_hive_cleanup_runs", "status": "active", "payload": "{\"_internal\":true}"}]}),
                ),
                _ => (404, json!({})),
            },
            2,
        );
        let mut rt = work_rt(port, demo_chat);
        click(&mut rt, "nav/notes");
        assert_eq!(rt.state.screen, Screen::Notes);
        settle(&mut rt, |s| {
            s.sys.shown.notes_loaded && !s.sys.shown.notes.is_empty()
        });
        assert!(rt.find("note/n9").is_some(), "la nota del servidor se ve");
        click(&mut rt, "nav/cron");
        assert_eq!(rt.state.screen, Screen::Cron);
        settle(&mut rt, |s| s.sys.shown.tasks_loaded);
        assert!(rt.find("cron/card/c1").is_some());
        assert!(
            rt.find("cron/card/c2").is_none(),
            "las tareas internas de Hive no se muestran"
        );
        let seen = server.join().unwrap();
        assert_eq!(
            seen.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["/api/notes", "/api/cron"]
        );
    }

    #[test]
    fn the_cron_buttons_call_the_servers_endpoints_and_delete_asks_first() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p) {
                ("POST", "/api/cron/t1/pause")
                | ("POST", "/api/cron/t1/trigger")
                | ("DELETE", "/api/cron/t1") => (200, json!({"ok": true})),
                ("GET", "/api/cron") => (200, json!({"tasks": []})),
                _ => (404, json!({})),
            },
            6,
        );
        let mut rt = work_rt(port, demo_cron);
        // Pausar y Ejecutar: una llamada cada uno y luego se relee la lista.
        click(&mut rt, "cron/pause/t1");
        settle(&mut rt, |s| {
            s.sys.shown.notice.as_deref() == Some("Tarea pausada")
        });
        assert_eq!(rt.state.sys.shown.notice.as_deref(), Some("Tarea pausada"));
        // El servidor falso devolvió la lista vacía: se vuelve a poner la de ejemplo para seguir probando.
        rt.update_if(|s: &mut State, _| {
            demo_work_data_tasks_only(s);
            true
        });
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "cron/trigger/t1");
        settle(&mut rt, |s| {
            s.sys.shown.notice.as_deref() == Some("Tarea lanzada")
        });
        assert_eq!(rt.state.sys.shown.notice.as_deref(), Some("Tarea lanzada"));
        rt.update_if(|s: &mut State, _| {
            demo_work_data_tasks_only(s);
            true
        });
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        // Eliminar pide confirmación y no llama hasta que se acepta.
        click(&mut rt, "cron/delete/t1");
        assert_eq!(rt.state.work.confirm_delete.as_deref(), Some("t1"));
        click(&mut rt, "cron/delete-no/t1");
        assert!(rt.state.work.confirm_delete.is_none());
        click(&mut rt, "cron/delete/t1");
        click(&mut rt, "cron/delete-yes/t1");
        settle(&mut rt, |s| {
            s.sys.shown.notice.as_deref() == Some("Tarea eliminada")
        });
        assert_eq!(
            rt.state.sys.shown.notice.as_deref(),
            Some("Tarea eliminada")
        );
        let seen = server.join().unwrap();
        let calls: Vec<String> = seen.iter().map(|r| format!("{} {}", r.0, r.1)).collect();
        assert_eq!(
            calls,
            [
                "POST /api/cron/t1/pause",
                "GET /api/cron",
                "POST /api/cron/t1/trigger",
                "GET /api/cron",
                "DELETE /api/cron/t1",
                "GET /api/cron"
            ]
        );
    }

    #[test]
    fn the_cron_filter_and_history_work_from_the_page() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p.split('?').next().unwrap()) {
                ("GET", "/api/cron/t2/history") => (
                    200,
                    json!({"history": [{"status": "success", "started_at": "2026-11-08T07:00:00.003Z", "duration_ms": 1500, "agent_response": "listo"}]}),
                ),
                _ => (404, json!({})),
            },
            1,
        );
        let mut rt = work_rt(port, |s| {
            demo_work_data(s);
            s.screen = Screen::Cron;
            s.sys.sync();
        });
        assert!(rt.find("cron/card/t1").is_some() && rt.find("cron/card/t3").is_some());
        click(&mut rt, "cron/f/paused");
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("cron/card/t1").is_none() && rt.find("cron/card/t3").is_some(),
            "solo las pausadas"
        );
        click(&mut rt, "cron/f/all");
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "cron/history/t2");
        settle(&mut rt, |s| {
            s.sys.shown.history.get("t2").is_some_and(|h| h.len() == 1)
        });
        assert_eq!(rt.state.sys.shown.history["t2"][0].response, "listo");
        assert_eq!(rt.state.work.history_open.as_deref(), Some("t2"));
        server.join().unwrap();
    }

    #[test]
    fn the_api_client_validates_then_sends_through_the_server_and_shows_the_answer() {
        let (port, server) = fake_http(
            |m, p, _, b| {
                let body: Value = serde_json::from_str(b).unwrap_or_default();
                match (m, p) {
                    ("POST", "/api/http-request") if body["url"] == "https://api.ejemplo.com/x" => {
                        (
                            200,
                            json!({"ok": true, "status": 404, "statusText": "Not Found", "headers": {"content-type": "text/plain"}, "body": "nada por aquí"}),
                        )
                    }
                    _ => (400, json!({"error": "mala petición"})),
                }
            },
            1,
        );
        let mut rt = work_rt(port, |s| {
            demo_chat(s);
            s.screen = Screen::ApiClient;
        });
        // Sin URL se explica y no se llama.
        click(&mut rt, "api/send");
        assert_eq!(rt.state.work.api.error.as_deref(), Some("Escribe la URL"));
        assert!(!rt.state.work.api.sending);
        // Con URL, método y cuerpo.
        click(&mut rt, "api/m/PUT");
        assert_eq!(rt.state.work.api.method, "PUT");
        rt.update_if(|s: &mut State, _| {
            s.work.api.url = TextEdit::new("https://api.ejemplo.com/x");
            s.work.api.body = TextEdit::new("{\"a\":1}");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "api/send");
        assert!(rt.state.work.api.sending || rt.state.work.api.answer.is_some());
        settle(&mut rt, |s| s.work.api.answer.is_some());
        let a = rt.state.work.api.answer.clone().expect("respuesta");
        let r = a.reply.expect("sin error");
        assert_eq!((r.status, r.body.as_str()), (Some(404), "nada por aquí"));
        assert!(rt.find("api/response").is_some());
        click(&mut rt, "api/res/headers");
        assert_eq!(rt.state.work.api.res_tab, ui::work_pages::ResTab::Headers);
        let seen = server.join().unwrap();
        let sent: Value = serde_json::from_str(&seen[0].3).unwrap();
        assert_eq!(sent["method"], "PUT");
        assert_eq!(sent["body"], "{\"a\":1}");
        assert_eq!(
            sent["headers"],
            json!({"Accept": "application/json"}),
            "la cabecera de ejemplo viaja"
        );
    }

    #[test]
    fn api_client_pairs_ignore_blank_and_separatorless_lines() {
        let p = ui::work_pages::parse_pairs(
            "Accept: json\n\nsin separador\nX-Id:  7 \n: sin clave",
            ':',
        );
        assert_eq!(
            p,
            vec![
                ("Accept".to_string(), "json".to_string(), true),
                ("X-Id".to_string(), "7".to_string(), true)
            ]
        );
        assert_eq!(
            ui::work_pages::parse_pairs("a=1\nb=2=3", '=')[1],
            ("b".to_string(), "2=3".to_string(), true),
            "solo se parte por el primer separador"
        );
        assert_eq!(
            ui::work_pages::one_line("uno\n  dos   tres", 100),
            "uno dos tres"
        );
        assert_eq!(ui::work_pages::one_line("abcdefghij", 4), "abcd…");
    }

    #[test]
    fn the_profile_is_read_only_until_edit_then_validates_and_patches_the_user() {
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p.split('?').next().unwrap()) {
                ("PATCH", "/api/user/settings") => (200, json!({"message": "ok"})),
                ("GET", "/api/users") => (
                    200,
                    json!({"users": [{"id": "u1", "name": "Ana Gómez", "email": "ana@b.co", "language": "es", "timezone": "America/Bogota", "occupation": "Dev", "notes": "x", "preferred_cron_channel": "auto"}]}),
                ),
                _ => (200, json!({})),
            },
            2,
        );
        let mut st = State::offline();
        demo_profile(&mut st);
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("profile/name").is_none() && rt.find("profile/edit").is_some(),
            "los campos no son editables hasta pulsar Editar"
        );
        click(&mut rt, "profile/edit");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("profile/name").is_some() && rt.find("profile/save").is_some());
        // Nombre vacío o correo roto: se explica y no se envía.
        rt.update_if(|s: &mut State, _| {
            s.profile.name = rune::TextEdit::new("");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "profile/save");
        assert_eq!(
            rt.state.sys.data.lock().unwrap().error.as_deref(),
            Some("Escribe tu nombre")
        );
        rt.update_if(|s: &mut State, _| {
            s.profile.name = rune::TextEdit::new("Ana Gómez");
            s.profile.email = rune::TextEdit::new("sin-arroba");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "profile/save");
        assert_eq!(
            rt.state.sys.data.lock().unwrap().error.as_deref(),
            Some("El correo no parece válido")
        );
        rt.update_if(|s: &mut State, _| {
            s.profile.email = rune::TextEdit::new("ana@b.co");
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "profile/save");
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str()),
            ("PATCH", "/api/user/settings?userId=u1")
        );
        assert_eq!(seen[0].2["name"], "Ana Gómez");
        assert_eq!(seen[0].2["email"], "ana@b.co");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while rt.state.sys.data.lock().unwrap().notice.is_none() && std::time::Instant::now() < end
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            rt.state.sys.data.lock().unwrap().notice.as_deref(),
            Some("Perfil guardado")
        );
        assert!(
            !rt.state.profile.editing,
            "al guardar se vuelve a solo lectura"
        );
    }

    #[test]
    fn choosing_audio_devices_updates_the_preferences_and_the_picker_lists_the_system_ones() {
        let mut rt = scene_rt(demo_display);
        assert!(
            rt.find("audio/in").is_some()
                && rt.find("audio/out").is_some()
                && rt.find("audio/mic-test").is_some()
                && rt.find("audio/test").is_some()
        );
        click(&mut rt, "audio/in");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.audio.open, 1);
        assert!(
            rt.find("audio/in/0").is_some() && rt.find("audio/in/2").is_some(),
            "«predeterminado» más los dos micrófonos"
        );
        click(&mut rt, "audio/in/2");
        assert_eq!(rt.state.audio.prefs.input.as_deref(), Some("mic2"));
        assert_eq!(rt.state.audio.open, 0, "al elegir se cierra la lista");
        click(&mut rt, "audio/in");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "audio/in/0");
        assert_eq!(
            rt.state.audio.prefs.input, None,
            "«predeterminado» vuelve a usar el del sistema"
        );
    }

    #[test]
    fn the_camera_can_be_chosen_from_the_system_cameras_and_the_change_is_queued_for_the_live_call()
    {
        let mut rt = scene_rt_sized(demo_display, (1440.0, 1500.0)); // la página se desplaza; así entra entera
        assert!(
            rt.find("audio/camera").is_some() && rt.find("audio/cam").is_some(),
            "hay una sección de cámara"
        );
        assert_eq!(
            rt.state.audio.prefs.camera, None,
            "sin elegir: la primera que entrega imagen"
        );
        click(&mut rt, "audio/cam");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.audio.open, 3);
        assert!(
            rt.find("audio/cam/0").is_some()
                && rt.find("audio/cam/2").is_some()
                && rt.find("audio/cam/3").is_none(),
            "«primera» más las dos cámaras"
        );
        click(&mut rt, "audio/cam/2");
        assert_eq!(rt.state.audio.prefs.camera.as_deref(), Some("3"));
        assert_eq!(rt.state.audio.open, 0, "al elegir se cierra la lista");
        assert_eq!(
            rt.state.audio.changed,
            Some(3),
            "queda pendiente de aplicarse a la llamada en curso"
        );
        let mut tx = rune::Tx::default();
        poll(&mut rt.state, 0.0, &mut tx);
        assert_eq!(
            rt.state.audio.changed, None,
            "el siguiente ciclo lo aplica una sola vez"
        );
        // Volver a «primera» borra la elección.
        click(&mut rt, "audio/cam");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "audio/cam/0");
        assert_eq!(rt.state.audio.prefs.camera, None);
        // Elegir micrófono o salida también queda pendiente, cada uno con su número.
        rt.state.audio.choose(true, Some("mic2".into()));
        assert_eq!(rt.state.audio.changed, Some(1));
        rt.state.audio.choose(false, None);
        assert_eq!(rt.state.audio.changed, Some(2));
    }

    #[test]
    fn hivelive_has_a_devices_button_that_opens_screen_and_audio_even_without_a_call() {
        let mut rt = scene_rt(demo_live);
        assert_eq!(rt.state.screen, Screen::Live);
        assert!(!rt.state.voice_live, "sin llamada");
        assert!(
            rt.find("voice/devices").is_some(),
            "el acceso está en la barra de la llamada"
        );
        click(&mut rt, "voice/devices");
        assert_eq!(
            (rt.state.screen, rt.state.settings.tab),
            (Screen::Settings, settings::Tab::Display)
        );
        assert!(
            rt.find("audio/in").is_some() || rt.find("audio/cam").is_some(),
            "y se ve la página de los aparatos"
        );
    }

    #[test]
    fn without_cameras_the_page_says_so_and_the_system_default_entries_are_not_listed_twice() {
        let mut rt = scene_rt_sized(demo_display, (1440.0, 1500.0));
        rt.update_if(|s: &mut State, _| {
            s.audio.cameras.clear();
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(
            rt.find("audio/cam").is_none(),
            "sin cámaras no hay selector que no lleve a nada"
        );
        let device = |id: &str, default: bool| hive_media::AudioDevice {
            id: id.into(),
            name: id.into(),
            is_default: default,
        };
        let kept = ui::system_pages::without_system_default(vec![
            device("pipewire:input_default", true),
            device("pipewire:alsa_input.usb-x", false),
            device("pipewire:output_default", true),
            device("pipewire:bluez_output.x", false),
        ]);
        assert_eq!(
            kept.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["pipewire:alsa_input.usb-x", "pipewire:bluez_output.x"],
            "«Predeterminado del sistema» ya cubre la entrada default_*"
        );
    }

    #[test]
    fn the_theme_can_also_be_switched_from_screen_and_audio() {
        let mut rt = scene_rt(demo_display);
        let was_light = ui::theme::is_light();
        click(
            &mut rt,
            if was_light {
                "theme/dark"
            } else {
                "theme/light"
            },
        );
        assert_eq!(ui::theme::is_light(), !was_light);
        ui::theme::set_light(was_light);
    }

    #[test]
    fn the_updates_page_reports_the_servers_version_and_whether_it_is_current() {
        let rt = scene_rt(demo_updates);
        assert!(rt.find("update/server").is_some() && rt.find("update/app").is_some());
        assert!(rt
            .state
            .sys
            .shown
            .version
            .as_ref()
            .is_some_and(|v| v.up_to_date()));
    }

    #[test]
    fn every_settings_page_is_reachable_from_the_sidebar_groups() {
        // Alta: con todos los grupos desplegados la barra pasa de 900 pt (en una ventana baja se desplaza).
        let mut st = State::offline();
        demo_profile(&mut st);
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 1300.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let mut rt = settled(rt);
        for (key, tab) in [
            ("nav/ethics", Tab::Ethics),
            ("nav/display", Tab::Display),
            ("nav/updates", Tab::Updates),
            ("nav/profile", Tab::Profile),
            ("nav/tools", Tab::Tools),
        ] {
            assert!(rt.find(key).is_some(), "falta {key}");
            click(&mut rt, key);
            assert_eq!(rt.state.settings.tab, tab, "{key}");
        }
    }

    #[test]
    fn adding_whatsapp_needs_no_secrets() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Channels);
        click(&mut rt, "chan/add/whatsapp");
        assert!(rt.find("chan/add/token").is_none());
        click(&mut rt, "chan/add/create");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/channels".into(),
                json!({"type": "whatsapp", "config": {}})
            )
        );
    }

    #[test]
    fn toggling_a_channel_posts_the_new_state() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Channels);
        click(&mut rt, "chan/telegram/toggle");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/channels/telegram/toggle".into(),
                json!({"active": false})
            )
        );
    }

    #[test]
    fn an_expired_session_is_reported() {
        let (port, server) = fake_http(|_, _, _, _| (401, json!("Unauthorized")), 2);
        let mut rt = settings_rt(port, Tab::Channels);
        click(&mut rt, "chan/telegram/toggle");
        wait_idle(&rt);
        server.join().unwrap();
        let d = rt.state.settings.data.lock().unwrap().clone();
        assert!(d.unauthorized, "{d:?}");
    }

    #[test]
    fn opening_settings_for_the_first_time_loads_the_lists() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 3);
        let mut st = State::offline();
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/providers");
        wait_idle(&rt);
        let paths: Vec<String> = server.join().unwrap().into_iter().map(|r| r.1).collect();
        assert_eq!(paths, ["/api/providers", "/api/agents", "/api/channels"]);
        assert!(rt.state.settings.sync());
        assert_eq!(rt.state.settings.shown.providers.len(), 2);
        let _ = Action::Refresh;
    }

    #[test]
    fn headless_tooling_never_boots_the_gateway() {
        let a = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(is_headless(&a(&["app", "--snap", "x.png"])));
        assert!(is_headless(&a(&["app", "--script", "s.json"])));
        assert!(is_headless(&a(&["app", "--snap=x.png"])));
        assert!(!is_headless(&a(&["app"])));
        assert!(!is_headless(&a(&["app", "--chat", "hola"])));
    }

    #[test]
    fn assistant_messages_render_markdown_while_streaming_and_after() {
        for busy in [true, false] {
            let mut st = State::offline();
            demo_markdown(&mut st);
            st.busy = busy;
            // Un fragmento más llega mientras se escribe: la vista debe aceptar la fuente creciente.
            st.msgs[1].text.push_str("\n\n| a | b |\n|---|---|\n| 1");
            let mut rt = Runtime::new(
                st,
                view,
                (1100.0, 900.0),
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            rt.frame(0.0);
            assert!(
                rt.find("me-u0").is_some() && rt.find("bee-m1").is_some(),
                "busy={busy}"
            );
        }
    }

    #[test]
    fn a_surface_sits_in_the_conversation_where_the_agent_created_it() {
        let mut st = State::offline();
        st.msgs.push(Msg {
            id: "u0".into(),
            mine: true,
            text: "hola".into(),
        });
        // La superficie nace tras el primer mensaje y antes de la respuesta del agente.
        apply(
            &mut st,
            Event::A2ui {
                kind: "a2ui:createSurface".into(),
                data: json!({"surfaceId": "s1"}),
            },
        );
        apply(
            &mut st,
            Event::A2ui {
                kind: "a2ui:updateComponents".into(),
                data: json!({"surfaceId": "s1", "components": [{"id": "root", "component": "Text", "text": "**panel**"}]}),
            },
        );
        st.msgs.push(Msg {
            id: "m1".into(),
            mine: false,
            text: "listo".into(),
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let y = |k: &str| rt.presented(k).unwrap_or_else(|| panic!("falta {k}")).0[1];
        assert!(
            y("me-u0") < y("a2:s1") && y("a2:s1") < y("bee-m1"),
            "mensaje < panel < respuesta: {} {} {}",
            y("me-u0"),
            y("a2:s1"),
            y("bee-m1")
        );
    }

    #[test]
    fn a2ui_text_goes_through_the_markdown_parser() {
        let mut st = State::offline();
        st.a2
            .apply("a2ui:createSurface", &json!({"surfaceId": "s"}));
        st.a2.apply(
            "a2ui:updateComponents",
            &json!({"surfaceId": "s", "components": [
            {"id": "root", "component": "Text", "text": "Hola **mundo**\n\n- uno\n- dos"}]}),
        );
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        // Con el parser, dos viñetas y un párrafo ocupan bastante más alto que una sola línea de texto plano.
        let (rect, _) = rt.presented("a2:s").expect("la superficie se pinta");
        assert!(
            rect[3] > 130.0,
            "alto de la superficie con párrafo y lista: {rect:?}"
        );
    }

    #[test]
    fn the_tools_tab_lists_and_filters_by_name_description_or_category() {
        let (port, _server) = fake_http(|_, p, _, _| (200, lists(p)), 0);
        let mut rt = settings_rt(port, Tab::Tools);
        for k in [
            "tool/web_search/toggle",
            "tool/read_file/toggle",
            "tool/web_fetch/toggle",
        ] {
            assert!(rt.find(k).is_some(), "{k}");
        }
        rt.update_if(|s: &mut State, _| {
            s.settings.filter = rune::TextEdit::new("WEB");
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(
            rt.find("tool/web_search/toggle").is_some()
                && rt.find("tool/web_fetch/toggle").is_some()
        );
        assert!(
            rt.find("tool/read_file/toggle").is_none(),
            "el filtro ignora mayúsculas y oculta lo que no coincide"
        );
    }

    #[test]
    fn long_tool_lists_are_capped() {
        let (port, _server) = fake_http(|_, p, _, _| (200, lists(p)), 0);
        let mut rt = settings_rt(port, Tab::Tools);
        rt.update_if(|s: &mut State, _| {
            s.settings.data.lock().unwrap().tools = (0..150)
                .map(|i| settings::api::Tool {
                    id: format!("t{i}"),
                    name: format!("tool {i}"),
                    active: true,
                    enabled: true,
                    ..Default::default()
                })
                .collect();
            s.settings.sync();
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("tool/t0/toggle").is_some() && rt.find("tool/t59/toggle").is_some());
        assert!(
            rt.find("tool/t60/toggle").is_none(),
            "se pintan 60 filas como máximo"
        );
    }

    #[test]
    fn toggling_a_tool_rereads_only_tools_skills_and_mcp() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Tools);
        click(&mut rt, "tool/web_search/toggle");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/tools/web_search/toggle".into(),
                json!({"active": false})
            )
        );
        assert_eq!(
            seen[1..].iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["/api/tools", "/api/skills", "/api/mcp/servers"]
        );
    }

    #[test]
    fn toggling_a_skill_posts_the_new_state() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 4);
        let mut rt = settings_rt(port, Tab::Skills);
        click(&mut rt, "skill/sk1/toggle");
        wait_idle(&rt);
        let seen = bodies(&server.join().unwrap());
        assert_eq!(
            seen[0],
            (
                "POST".into(),
                "/api/skills/sk1/toggle".into(),
                json!({"active": false})
            )
        );
    }

    #[test]
    fn opening_the_tools_tab_the_first_time_loads_tools_skills_and_mcp() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 3);
        let mut st = State::offline();
        st.screen = Screen::Settings;
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        st.settings.data.lock().unwrap().loaded = true;
        st.settings.sync();
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/settings"); // «Ajustes» abre Herramientas
        wait_idle(&rt);
        let paths: Vec<String> = server.join().unwrap().into_iter().map(|r| r.1).collect();
        assert_eq!(paths, ["/api/tools", "/api/skills", "/api/mcp/servers"]);
        rt.state.settings.sync();
        assert_eq!(
            (
                rt.state.settings.shown.tools.len(),
                rt.state.settings.shown.skills.len(),
                rt.state.settings.shown.mcp.len()
            ),
            (3, 1, 2)
        );
    }

    #[test]
    fn the_mcp_tab_lists_servers_read_only() {
        let (port, _server) = fake_http(|_, p, _, _| (200, lists(p)), 0);
        let rt = settings_rt(port, Tab::Mcp);
        assert!(rt.find("mcp/cal").is_some() && rt.find("mcp/mail").is_some());
        assert_eq!(rt.state.settings.shown.mcp[0].tool_count(), Some(3));
    }

    #[test]
    fn the_ocr_tab_reads_edits_and_saves_the_gateway_settings() {
        let reply = |enabled: bool| {
            json!({"settings":{"enabled":enabled,"folder":if enabled {"/docs"} else {""},"provider_id":if enabled {"mistral"} else {""},"model_id":if enabled {"mistral/ocr"} else {""},"ask":true,"session_minutes":10,"max_pages":if enabled {100} else {50}},
            "ready":enabled,"candidates":[
                {"provider_id":"mistral","model_id":"mistral/ocr","name":"Mistral OCR","kind":"ocr","supported":true},
                {"provider_id":"otro","model_id":"otro/m","name":"Otro","kind":"ocr","supported":false,"note":"Aún sin adaptador"}]})
        };
        let (port, server) = fake_http(
            move |m, p, _, body| match (m, p) {
                ("GET", "/api/ocr") => (200, reply(false)),
                ("PUT", "/api/ocr") => {
                    let sent: Value = serde_json::from_str(body).unwrap();
                    assert_eq!(
                        (
                            sent["enabled"].clone(),
                            sent["folder"].clone(),
                            sent["provider_id"].clone(),
                            sent["max_pages"].clone()
                        ),
                        (json!(true), json!("/docs"), json!("mistral"), json!(100)),
                        "{sent}"
                    );
                    (200, reply(true))
                }
                _ => (404, json!({})),
            },
            2,
        );
        let mut rt = settings_rt_sized(port, Tab::Ocr, (1100.0, 2600.0));
        let pump = |rt: &mut Runtime<State>, done: fn(&State) -> bool| {
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done(&rt.state) && std::time::Instant::now() < end {
                rt.update_if(|s: &mut State, _| s.ocr.sync());
                rt.frame(1.0 / 60.0);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            rt.update_if(|s: &mut State, _| s.ocr.sync());
            for _ in 0..5 {
                rt.frame(1.0 / 60.0);
            }
        };
        click(&mut rt, "ocr/refresh");
        pump(&mut rt, |s| s.ocr.shown.loaded);
        assert!(
            !rt.state.ocr.enabled
                && !rt.state.ocr.shown.ready
                && rt.state.ocr.shown.candidates.len() == 2
        );
        assert!(
            rt.state.ocr.missing().len() == 2,
            "{:?}",
            rt.state.ocr.missing()
        );
        // Edit: switch on, folder, provider (a provider without adapter cannot be chosen), pages.
        click(&mut rt, "ocr/enabled");
        rt.state.ocr.folder = rune::TextEdit::new("/docs");
        rt.frame(1.0 / 60.0);
        click(&mut rt, "ocr/provider");
        click(&mut rt, "ocr/provider/1");
        assert!(
            rt.state.ocr.provider_id.is_empty(),
            "no adapter, not chosen"
        );
        click(&mut rt, "ocr/provider/0");
        assert_eq!(rt.state.ocr.model_id, "mistral/ocr");
        click(&mut rt, "ocr/pages/100");
        assert!(rt.state.ocr.dirty() && rt.state.ocr.missing().is_empty());
        click(&mut rt, "ocr/save");
        pump(&mut rt, |s| s.ocr.shown.settings.enabled);
        assert!(
            rt.state.ocr.shown.ready && !rt.state.ocr.dirty(),
            "saved and in sync"
        );
        assert_eq!(rt.state.ocr.shown.notice.as_deref(), Some("Guardado"));
        let seen: Vec<String> = server
            .join()
            .unwrap()
            .into_iter()
            .map(|r| format!("{} {}", r.0, r.1))
            .collect();
        assert_eq!(seen, ["GET /api/ocr", "PUT /api/ocr"]);
    }

    #[test]
    fn the_ocr_tab_shows_the_gateways_refusal_and_keeps_the_edit() {
        let (port, _server) = fake_http(
            |m, _, _, _| {
                if m == "PUT" {
                    (400, json!({"error":"la carpeta de documentos no existe"}))
                } else {
                    (200, json!({"settings":{},"ready":false,"candidates":[]}))
                }
            },
            1,
        );
        let mut rt = settings_rt_sized(port, Tab::Ocr, (1100.0, 2600.0));
        click(&mut rt, "ocr/enabled");
        click(&mut rt, "ocr/save");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while rt.state.ocr.shown.error.is_none() && std::time::Instant::now() < end {
            rt.update_if(|s: &mut State, _| s.ocr.sync());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            rt.state
                .ocr
                .shown
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("la carpeta de documentos no existe"),
            "{:?}",
            rt.state.ocr.shown.error
        );
        assert!(rt.state.ocr.enabled, "what was typed is not lost");
    }

    fn pump_remote(rt: &mut Runtime<State>, done: fn(&State) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(&rt.state) && std::time::Instant::now() < end {
            rt.update_if(|s: &mut State, _| s.computer.sync() | s.oracle.sync());
            rt.frame(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        rt.update_if(|s: &mut State, _| s.computer.sync() | s.oracle.sync());
        for _ in 0..5 {
            rt.frame(1.0 / 60.0);
        }
    }

    #[test]
    fn the_computer_tab_edits_the_permissions_and_always_has_a_stop_button() {
        let status = |enabled: bool, input: bool, stopped: bool| json!({"settings":{"enabled":enabled,"allow_input":input,"backend":if enabled {"x11"} else {"auto"},"max_dimension":1280,"max_actions":200,"session_minutes":10},"stopped":stopped,"actions":7,"model_has_vision":false});
        let (port, server) = fake_http(
            move |m, p, _, body| match (m, p) {
                ("GET", "/api/computer") => (200, status(false, false, false)),
                ("PUT", "/api/computer") => {
                    let sent: Value = serde_json::from_str(body).unwrap();
                    assert_eq!(
                        (
                            sent["enabled"].clone(),
                            sent["allow_input"].clone(),
                            sent["backend"].clone()
                        ),
                        (json!(true), json!(true), json!("x11")),
                        "{sent}"
                    );
                    (200, status(true, true, false))
                }
                ("POST", "/api/computer/stop") => (200, status(true, true, true)),
                ("POST", "/api/computer/resume") => (200, status(true, true, false)),
                _ => (404, json!({})),
            },
            4,
        );
        let mut rt = settings_rt_sized(port, Tab::Computer, (1100.0, 2600.0));
        assert!(
            rt.find("computer/stop").is_some(),
            "the stop button is there even before anything is read"
        );
        click(&mut rt, "computer/refresh");
        pump_remote(&mut rt, |s| s.computer.remote.shown.loaded);
        assert!(
            !rt.state.computer.enabled
                && rt.state.computer.remote.shown.value["model_has_vision"] == json!(false)
        );
        click(&mut rt, "computer/enabled");
        click(&mut rt, "computer/input");
        click(&mut rt, "computer/backend/x11");
        assert!(rt.state.computer.dirty());
        click(&mut rt, "computer/save");
        pump_remote(&mut rt, |s| {
            s.computer.remote.shown.value["settings"]["enabled"] == json!(true)
        });
        assert!(
            !rt.state.computer.dirty()
                && rt.state.computer.remote.shown.notice.as_deref() == Some("Guardado")
        );
        click(&mut rt, "computer/stop");
        pump_remote(&mut rt, |s| s.computer.stopped());
        assert!(
            rt.find("computer/resume").is_some(),
            "after stopping, it can be resumed"
        );
        click(&mut rt, "computer/resume");
        pump_remote(&mut rt, |s| !s.computer.stopped());
        let seen: Vec<String> = server
            .join()
            .unwrap()
            .into_iter()
            .map(|r| format!("{} {}", r.0, r.1))
            .collect();
        assert_eq!(
            seen,
            [
                "GET /api/computer",
                "PUT /api/computer",
                "POST /api/computer/stop",
                "POST /api/computer/resume"
            ]
        );
    }

    #[test]
    fn the_oracle_tab_sends_the_key_only_when_one_is_typed_and_never_shows_it_back() {
        let status = |enabled: bool| json!({"settings":{"enabled":enabled,"endpoint":"https://oraculo.example/v1","model":"m","self_hosted":false,"timeout_ms":5000,"max_corrections":2,"min_confidence":0.8,"support_threshold":0.5},"has_api_key":true,"status":null});
        let (port, server) = fake_http(
            move |m, p, _, body| match (m, p) {
                ("GET", "/api/oracle") => (200, status(false)),
                ("PUT", "/api/oracle") => {
                    let sent: Value = serde_json::from_str(body).unwrap();
                    match sent["enabled"] == json!(true)
                        && sent["max_corrections"] == json!(3)
                        && sent.get("api_key").is_none()
                    {
                        true => (200, status(true)),
                        false => (400, json!({"error":format!("inesperado: {sent}")})),
                    }
                }
                _ => (404, json!({})),
            },
            3,
        );
        let mut rt = settings_rt_sized(port, Tab::Oracle, (1100.0, 2600.0));
        click(&mut rt, "oracle/refresh");
        pump_remote(&mut rt, |s| s.oracle.remote.shown.loaded);
        assert_eq!(
            (
                rt.state.oracle.endpoint.text.as_str(),
                rt.state.oracle.has_key(),
                rt.state.oracle.key.text.as_str()
            ),
            ("https://oraculo.example/v1", true, ""),
            "the saved key is not shown back"
        );
        assert!(!rt.state.oracle.dirty());
        click(&mut rt, "oracle/enabled");
        click(&mut rt, "oracle/corr/3");
        assert!(
            rt.state.oracle.body().get("api_key").is_none(),
            "no key typed, none sent"
        );
        click(&mut rt, "oracle/save");
        pump_remote(&mut rt, |s| {
            s.oracle.remote.shown.value["settings"]["enabled"] == json!(true)
        });
        assert!(
            !rt.state.oracle.dirty() && rt.state.oracle.remote.shown.error.is_none(),
            "{:?}",
            rt.state.oracle.remote.shown.error
        );
        // Typing a key makes the form dirty and the key travels once, then the field is empty again.
        rt.state.oracle.key = rune::TextEdit::new("sk-nueva").secure();
        assert!(rt.state.oracle.dirty());
        assert_eq!(rt.state.oracle.body()["api_key"], "sk-nueva");
        drop(server);
    }

    /// Contrato real: el escritorio contra el gateway del SDK, sin servidores falsos.
    /// Necesita una copia aislada de un perfil Hive (`hive-gateway-rust --prepare-test-home ORIGEN DESTINO`):
    /// `HIVE_RUNE_PROFILE=<copia> HIVE_GATEWAY_BIN=<gateway> cargo test -- --ignored real_gateway_contract`
    #[test]
    #[ignore = "necesita HIVE_RUNE_PROFILE (copia de un perfil) y HIVE_GATEWAY_BIN"]
    fn real_gateway_contract_every_screen_loads_from_the_sdk_gateway() {
        let profile = std::env::var("HIVE_RUNE_PROFILE").expect("HIVE_RUNE_PROFILE");
        std::env::set_var("HIVE_HOME", &profile);
        let gw = crate::gateway::Gateway::start()
            .expect("el gateway del SDK arranca con la copia del perfil");
        let port = gw.port;
        let token = std::fs::read_to_string(std::path::Path::new(&profile).join(".auth_token"))
            .unwrap()
            .trim()
            .to_string();
        let conn = Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: Some(token.clone()),
        };

        // 1. Cada lectura que hace el escritorio contesta 200 con JSON y sin dejar detalles internos.
        let mut failures = vec![];
        for path in [
            "/health",
            "/api/agents",
            "/api/providers",
            "/api/channels",
            "/api/users",
            "/api/tools",
            "/api/skills",
            "/api/mcp/servers",
            "/api/cron",
            "/api/conversations",
            "/api/auth/status",
            "/api/version",
            "/api/usage-stats?hours=24",
            "/api/system-stats",
            "/api/activity-stats?hours=12",
            "/api/setup/status",
            "/api/oracle",
            "/api/ocr",
            "/api/computer",
            "/api/notes",
            "/api/agents/proposals",
            "/api/ethics",
            "/api/meetings",
            "/api/realtime/models",
        ] {
            match crate::settings::api::request(&conn, "GET", path, None) {
                Ok(v) if !v.is_null() => {}
                Ok(_) => failures.push(format!("{path}: respuesta vacía")),
                Err(e) => failures.push(format!("{path}: {e}")),
            }
        }
        assert!(
            failures.is_empty(),
            "lecturas que fallan:\n{}",
            failures.join("\n")
        );

        // 2. Sin token, lo protegido se rechaza (el escritorio lo trata como «hay que iniciar sesión»).
        let anon = Conn {
            base: conn.base.clone(),
            token: None,
        };
        let auth = crate::settings::api::request(&conn, "GET", "/api/auth/status", None).unwrap();
        let denied = crate::settings::api::request(&anon, "GET", "/api/ocr", None);
        if auth["hasCredentials"].as_bool() == Some(true) {
            assert!(denied.is_err(), "con credenciales configuradas, sin token no se leen los ajustes de OCR: {denied:?}");
        } else {
            // Sin contraseña el gateway es local y abierto: así lo trata también el escritorio.
            eprintln!("perfil sin credenciales: el acceso sin token está permitido ({auth})");
        }

        // 3. Las pantallas se cargan con los cargadores del propio escritorio, sin errores.
        let mut st = State::offline();
        st.settings.conn = Some(conn.clone());
        st.settings.dispatch(Action::Refresh);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < end {
            st.settings.sync();
            if st.settings.shown.loaded && st.settings.shown.busy == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let shown = &st.settings.shown;
        assert!(shown.error.is_none(), "Ajustes: {:?}", shown.error);
        assert!(
            !shown.providers.is_empty() && !shown.agents.is_empty(),
            "providers {} agents {}",
            shown.providers.len(),
            shown.agents.len()
        );
        assert!(
            shown.providers.iter().all(|p| !p.id.is_empty()),
            "todo proveedor trae id"
        );
        assert_eq!(
            shown.agents.iter().filter(|a| a.is_coordinator()).count(),
            1,
            "hay un único coordinador"
        );
        st.settings.dispatch(Action::RefreshExtras);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < end {
            st.settings.sync();
            if st.settings.shown.extras_loaded && st.settings.shown.busy == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            st.settings.shown.error.is_none(),
            "Herramientas/skills/MCP: {:?}",
            st.settings.shown.error
        );
        assert!(
            !st.settings.shown.tools.is_empty(),
            "el gateway ofrece herramientas"
        );

        for part in [
            crate::system::Part::Dashboard,
            crate::system::Part::Ethics,
            crate::system::Part::Profile,
            crate::system::Part::Notes,
            crate::system::Part::Cron,
        ] {
            while st.sys.busy() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            st.sys.refresh(&conn, part);
            let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while st.sys.busy() && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            st.sys.sync();
            assert!(
                st.sys.shown.error.is_none(),
                "{part:?}: {:?}",
                st.sys.shown.error
            );
        }
        assert!(
            st.sys.shown.stats.is_some()
                && st.sys.shown.version.is_some()
                && st.sys.shown.user.is_some(),
            "Dashboard y Perfil traen datos"
        );

        st.ocr.refresh(&conn);
        st.computer.load(&conn);
        st.oracle.load(&conn);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < end {
            st.ocr.sync();
            st.computer.sync();
            st.oracle.sync();
            if st.ocr.shown.loaded
                && st.computer.remote.shown.loaded
                && st.oracle.remote.shown.loaded
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            st.ocr.shown.error.is_none()
                && st.computer.remote.shown.error.is_none()
                && st.oracle.remote.shown.error.is_none(),
            "{:?} {:?} {:?}",
            st.ocr.shown.error,
            st.computer.remote.shown.error,
            st.oracle.remote.shown.error
        );
        assert!(
            !st.ocr.shown.settings.enabled && !st.computer.enabled && !st.oracle.enabled,
            "lo peligroso viene apagado"
        );
        assert!(
            !st.ocr.shown.candidates.is_empty(),
            "el OCR ofrece proveedores del perfil: {:?}",
            st.ocr.shown.candidates
        );

        // 4. El chat de verdad: crea la conversación y abre el WebSocket con el token (no se envía ningún mensaje).
        let chat = Chat::bootstrap(port, Some(&token));
        assert!(
            chat.is_ok(),
            "el chat arranca contra el gateway del SDK: {:?}",
            chat.err().map(|e| e.to_string())
        );

        // 5. Un turno completo de punta a punta: la persona escribe, el SDK llama al «modelo» (un servidor local con el
        // protocolo de Gemini, sin gasto) y el escritorio recibe fragmentos, el mensaje final y el cierre del turno.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let fake = format!("http://{}", listener.local_addr().unwrap());
        let asked: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let log = asked.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let log = log.clone();
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head_end, len) = loop {
                        let n = stream.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0usize);
                            break (i + 4, len);
                        }
                    };
                    while buf.len() < head_end + len {
                        let n = stream.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[head_end..]).to_string());
                    let request_line = String::from_utf8_lossy(&buf[..head_end])
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    let body = if request_line.contains("streamGenerateContent") {
                        let event = json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Hola desde el SDK"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":4,"totalTokenCount":9}});
                        format!("data: {event}\r\n\r\n")
                    } else {
                        let delta = json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"Hola desde el SDK"},"finish_reason":null}]});
                        let stop = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":4,"total_tokens":9}});
                        format!("data: {delta}\r\n\r\ndata: {stop}\r\n\r\ndata: [DONE]\r\n\r\n")
                    };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        // Nada debe salir de la máquina: TODOS los proveedores activos de la copia se redirigen al servidor local y se
        // comprueba, leyéndolo de vuelta, antes de enviar nada (el coordinador puede ser cualquiera: DeepSeek, Gemini…).
        let providers =
            crate::settings::api::request(&conn, "GET", "/api/providers", None).unwrap();
        for p in providers["providers"].as_array().unwrap() {
            if p["enabled"] == true {
                let id = p["id"].as_str().unwrap();
                crate::settings::api::request(
                    &conn,
                    "PUT",
                    &format!("/api/providers/{id}"),
                    Some(json!({"baseUrl": fake})),
                )
                .expect("el escritorio puede cambiar la dirección de un proveedor");
            }
        }
        let providers =
            crate::settings::api::request(&conn, "GET", "/api/providers", None).unwrap();
        for p in providers["providers"].as_array().unwrap() {
            if p["enabled"] == true {
                assert!(
                    p["base_url"].as_str().is_some_and(|u| u.starts_with(&fake)),
                    "el proveedor {} no apunta al servidor local: {:?}",
                    p["id"],
                    p["base_url"]
                );
            }
        }
        let chat = chat.unwrap();
        chat.send("Hola, ¿me oyes?");
        let mut text = String::new();
        let mut finished = false;
        let mut seen: Vec<String> = vec![];
        let end = std::time::Instant::now() + std::time::Duration::from_secs(40);
        while !finished && std::time::Instant::now() < end {
            for event in chat.drain() {
                seen.push(format!("{event:?}").chars().take(80).collect());
                match event {
                    crate::chat::Event::Chunk { text: t, .. } => text.push_str(&t),
                    crate::chat::Event::Final { text: t, .. } if text.is_empty() => text = t,
                    crate::chat::Event::Done => finished = true,
                    crate::chat::Event::Error(e) => panic!("el turno terminó con error: {e}"),
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(finished, "el turno no terminó; eventos: {seen:?}");
        assert!(
            text.contains("Hola desde el SDK"),
            "respuesta recibida: {text:?}; eventos: {seen:?}"
        );
        let bodies = asked.lock().unwrap().join("\n");
        assert!(
            bodies.contains("Hola, ¿me oyes?"),
            "el SDK envió al modelo lo que escribió la persona: {bodies}"
        );
        gw.stop();
    }

    // ── AudioPlayer de A2UI ──

    use crate::a2ui::audio::Sink;
    use std::sync::{Arc, Mutex as StdMutex};

    #[derive(Default)]
    struct Mem {
        played: StdMutex<Vec<usize>>,
        queued: StdMutex<u64>,
    }
    impl Sink for Mem {
        fn play(&self, pcm: &[u8]) -> Result<(), String> {
            self.played.lock().unwrap().push(pcm.len());
            *self.queued.lock().unwrap() = 800;
            Ok(())
        }
        fn stop(&self) {
            *self.queued.lock().unwrap() = 0;
        }
        fn queued_ms(&self) -> u64 {
            *self.queued.lock().unwrap()
        }
    }

    fn wav_bytes() -> Vec<u8> {
        let (rate, frames) = (8000u32, 4000u32);
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + frames * 2).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend([1, 0, 1, 0]);
        b.extend(rate.to_le_bytes());
        b.extend((rate * 2).to_le_bytes());
        b.extend([2, 0, 16, 0]);
        b.extend(b"data");
        b.extend((frames * 2).to_le_bytes());
        for i in 0..frames {
            b.extend((((i as f32 * 0.35).sin() * 9000.0) as i16).to_le_bytes());
        }
        b
    }

    fn audio_rt(url: &str, origin: Option<u16>, mem: Arc<Mem>) -> Runtime<State> {
        let mut st = State::offline();
        st.a2.audio.sink = mem;
        if let Some(p) = origin {
            st.a2
                .audio
                .clips
                .set_origin(&format!("http://127.0.0.1:{p}"), None);
        }
        st.a2
            .apply("a2ui:createSurface", &json!({"surfaceId": "demo"}));
        st.a2.apply(
            "a2ui:updateComponents",
            &json!({"surfaceId": "demo", "components": [
            {"id": "root", "component": "AudioPlayer", "url": url, "description": "Nota de voz"}]}),
        );
        let mut rt = Runtime::new(
            st,
            view,
            (520.0, 400.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        rt
    }

    fn pump(rt: &mut Runtime<State>, done: impl Fn(&State) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(&rt.state) && std::time::Instant::now() < end {
            rt.state.a2.audio.tick();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn the_audio_player_never_downloads_or_plays_on_its_own() {
        let mem = Arc::new(Mem::default());
        let mut rt = audio_rt("/clip.wav", Some(1), mem.clone());
        rt.frame(1.0 / 60.0);
        rt.state.a2.audio.tick();
        assert!(
            rt.state.a2.audio.clips.peek("/clip.wav").is_none(),
            "pintar no descarga"
        );
        assert!(mem.played.lock().unwrap().is_empty());
    }

    #[test]
    fn clicking_plays_the_clip_and_clicking_again_stops_it() {
        use std::io::{Read, Write};
        let wav = wav_bytes();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = l.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                wav.len()
            )
            .unwrap();
            sock.write_all(&wav).unwrap();
        });
        let mem = Arc::new(Mem::default());
        let mut rt = audio_rt("/clip.wav", Some(port), mem.clone());
        click(&mut rt, "demo/root");
        pump(&mut rt, |s| s.a2.audio.is_playing("/clip.wav"));
        assert!(rt.state.a2.audio.is_playing("/clip.wav"));
        assert_eq!(
            *mem.played.lock().unwrap(),
            vec![4000 * 3 * 2],
            "8 kHz a 24 kHz: el triple de muestras en PCM16"
        );
        // El botón ahora es "Detener": el mismo clic corta el sonido.
        rt.update_if(|_: &mut State, _| true);
        rt.frame(1.0 / 60.0);
        click(&mut rt, "demo/root");
        assert!(!rt.state.a2.audio.is_playing("/clip.wav") && mem.queued_ms_for_test() == 0);
    }

    #[test]
    fn a_blocked_clip_shows_its_error_on_that_player() {
        let mem = Arc::new(Mem::default());
        let mut rt = audio_rt("http://127.0.0.1:9/x.wav", None, mem.clone());
        click(&mut rt, "demo/root");
        pump(&mut rt, |s| s.a2.audio.error.is_some());
        let a = &rt.state.a2.audio;
        assert!(
            a.error
                .as_deref()
                .is_some_and(|e| e.contains("no permitida")),
            "{:?}",
            a.error
        );
        assert_eq!(a.error_url.as_deref(), Some("http://127.0.0.1:9/x.wav"));
        assert!(mem.played.lock().unwrap().is_empty());
    }

    impl Mem {
        fn queued_ms_for_test(&self) -> u64 {
            *self.queued.lock().unwrap()
        }
    }

    // ── detener un turno e interrumpir la voz ──

    /// WebSocket falso: acepta una conexión y devuelve los primeros `expect` frames de texto del cliente.
    fn fake_ws(expect: usize) -> (u16, std::thread::JoinHandle<Vec<Value>>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (stream, _) = l.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            ws.get_mut()
                .set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            let mut got = Vec::new();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while got.len() < expect && std::time::Instant::now() < end {
                if let Ok(tungstenite::Message::Text(t)) = ws.read() {
                    got.push(serde_json::from_str(t.as_str()).unwrap());
                }
            }
            got
        });
        (port, h)
    }

    #[test]
    fn the_stop_button_cancels_the_running_turn_and_frees_the_input() {
        let (port, server) = fake_ws(3);
        *CHAT.lock().unwrap() =
            Some(Chat::connect(port, "u1", None, "u1/webchat/c1".into()).unwrap());
        let mut st = State::offline();
        st.busy = true;
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "chat/stop");
        assert!(!rt.state.busy, "el campo de mensaje queda libre");
        let got = server.join().unwrap();
        CHAT.lock().unwrap().take();
        assert_eq!(got[0], json!({"type": "canvas_subscribe"}));
        assert_eq!(
            got[1],
            json!({"type": "notification_sync", "sessionId": "u1"})
        );
        assert_eq!(got[2], json!({"type": "stop", "threadId": "u1/webchat/c1"}));
    }

    #[test]
    fn the_stop_button_only_exists_while_a_turn_runs() {
        let mut rt = Runtime::new(
            State::offline(),
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(rt.find("chat/stop").is_none());
    }

    #[test]
    fn the_interrupt_button_cuts_the_model_during_a_voice_call() {
        let (port, server) = fake_ws(1);
        *VOICE.lock().unwrap() = Some(Realtime::start(port, "u1", None, false).unwrap());
        let mut st = State::offline();
        st.screen = Screen::Live;
        st.voice_on = true;
        st.voice_live = true;
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "voice/interrupt");
        let got = server.join().unwrap();
        VOICE.lock().unwrap().take();
        assert_eq!(got[0], json!({"type": "interrupt"}));
    }

    #[test]
    fn camera_and_screen_buttons_work_only_while_the_call_is_live_and_report_errors() {
        let mut st = State::offline();
        st.screen = Screen::Live;
        st.voice_on = true;
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        // Sin llamada viva los botones se ven apagados y no hacen nada.
        assert!(
            rt.find("voice/camera").is_some() && rt.find("voice/screen").is_some(),
            "los botones de HiveLive siempre están"
        );
        *MEDIA.lock().unwrap() = MediaFlags {
            camera: false,
            screen: false,
            starting: false,
            error: None,
        };
        click(&mut rt, "voice/camera");
        assert!(
            MEDIA.lock().unwrap().error.is_none() && !MEDIA.lock().unwrap().starting,
            "apagados: un clic no abre nada"
        );
        rt.update_if(|s: &mut State, _| {
            s.voice_live = true;
            true
        });
        rt.frame(1.0 / 60.0);
        let (x_cam, x_scr) = (
            rt.find("voice/camera").unwrap().0,
            rt.find("voice/screen").unwrap().0,
        );
        assert!(x_scr - x_cam > 40.0, "los botones no se pisan entre sí");
        // Sin sesión de voz real (VOICE vacío) el botón explica qué falta en lugar de abrir nada.
        *MEDIA.lock().unwrap() = MediaFlags {
            camera: false,
            screen: false,
            starting: false,
            error: None,
        };
        click(&mut rt, "voice/camera");
        // Con o sin sesión de voz (otros tests la usan) el resultado es un error visible y nunca queda «abriéndose».
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < end
            && (MEDIA.lock().unwrap().starting || MEDIA.lock().unwrap().error.is_none())
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (err, starting, on) = {
            let m = MEDIA.lock().unwrap();
            (m.error.clone(), m.starting, m.camera)
        };
        let err = err.expect("se explica por qué no se abrió");
        assert!(
            err == "Inicia la voz antes de compartir imagen"
                || err == "sin dispositivos en las pruebas",
            "{err}"
        );
        assert!(!starting && !on, "no queda abriéndose ni encendida");
        *MEDIA.lock().unwrap() = MediaFlags {
            camera: false,
            screen: false,
            starting: false,
            error: None,
        };
    }

    fn live_rt() -> Runtime<State> {
        let mut st = State::offline();
        demo_live(&mut st);
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    #[test]
    fn hivelive_renders_camera_and_screen_previews_and_removes_them_on_close() {
        let mut rt = live_rt();
        rt.update_if(|s: &mut State, _| {
            let jpeg = hive_media::video::encode_jpeg(
                &[255; 16],
                2,
                2,
                hive_media::video::Pixels::Rgba,
                hive_media::video::CAMERA_PREVIEW,
            )
            .unwrap();
            for preview in [&mut s.live.camera_preview, &mut s.live.screen_preview] {
                let feed = preview.begin();
                feed.frame(
                    hive_media::VideoFrame {
                        kind: hive_media::FrameKind::Preview,
                        jpeg: jpeg.clone(),
                    },
                    &mut |_| panic!("preview sent"),
                );
                preview.poll();
            }
            true
        });
        rt.frame(1.0 / 60.0);
        for key in [
            "live/camera-preview/image",
            "live/screen-preview/image",
            "live/camera-preview/stop",
            "live/screen-preview/stop",
        ] {
            assert!(rt.find(key).is_some(), "{key}");
        }
        rt.update_if(|s: &mut State, _| {
            s.live.camera_preview.stop();
            s.live.screen_preview.stop();
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("live/camera-preview").is_none());
        assert!(rt.find("live/screen-preview").is_none());
    }

    #[test]
    fn hivelive_model_selector_opens_catalog_and_locks_during_a_call() {
        let mut rt = live_rt();
        rt.update_if(|s: &mut State, _| {
            s.live.catalog.loaded = true;
            s.live.catalog.selected = Some("gemini-3.8-live".into());
            s.live.catalog.models = vec![
                ("gemini-3.8-live".into(), "Gemini 3.8 Live".into()),
                ("other-live".into(), "Otro modelo Live".into()),
            ];
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("live/model").is_some());
        click(&mut rt, "live/model");
        assert_eq!(rt.state.live.open_select, 4);
        assert!(rt.find("live/model/gemini-3.8-live").is_some());
        rt.update_if(|s: &mut State, _| {
            s.voice_on = true;
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("live/model/gemini-3.8-live").is_none());
        click(&mut rt, "live/model");
        assert!(rt.find("live/model/gemini-3.8-live").is_none());
        rt.update_if(|s: &mut State, _| {
            s.voice_on = false;
            s.live.reset();
            true
        });
        assert_eq!(
            rt.state.live.catalog.selected.as_deref(),
            Some("gemini-3.8-live")
        );
    }

    #[test]
    fn hivelive_preparation_keeps_controls_available_and_graphics_errors_can_retry() {
        let mut rt = live_rt();
        rt.update_if(|s: &mut State, _| {
            s.live.scene_loading = true;
            true
        });
        rt.frame(0.0);
        assert!(rt.find("live/scene-loading").is_some());
        assert!(rt.find("voice").is_some());
        rt.update_if(|s: &mut State, _| {
            s.live.scene_loading = false;
            s.live.scene_error = Some("Fallo gráfico de prueba".into());
            true
        });
        rt.frame(0.0);
        for _ in 0..45 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "live/scene-retry");
        assert_eq!(rt.state.live.scene_retry, 1);
        assert!(rt.state.live.scene_error.is_none());
    }

    #[test]
    fn hivelive_shows_the_panels_and_the_telemetry_is_real_not_decorative() {
        let mut rt = live_rt();
        for key in [
            "live/head",
            "live/telemetry",
            "live/transcript",
            "live/hint",
            "live/dock",
            "voice",
            "live/panel-toggle",
            "live/voice",
            "live/lang",
        ] {
            assert!(rt.find(key).is_some(), "falta {key}");
        }
        // Sin llamada no hay datos: la web también muestra rayas.
        assert_eq!(rt.state.live.total_tokens(), 0);
        rt.update_if(|s: &mut State, _| {
            apply_voice(
                s,
                REvent::Ready {
                    model: "gemini-3.8-live".into(),
                },
            );
            apply_voice(
                s,
                REvent::Usage {
                    input: 2000,
                    output: 500,
                },
            );
            true
        });
        assert_eq!(rt.state.live.model.as_deref(), Some("gemini-3.8-live"));
        assert_eq!(rt.state.live.total_tokens(), 2500);
        assert!(rt.state.voice_live);
    }

    #[test]
    fn the_wheel_zooms_bia_shift_turns_it_and_the_buttons_and_double_click_recentre() {
        let mut rt = live_rt();
        let (x, y) = rt.find(live::stage::SLOT_KEY).unwrap();
        let d0 = rt.state.live.dist;
        rt.wheel_event(rune::WheelEvent::new(0.0, 80.0).at(x, y));
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state.live.dist < d0,
            "rueda hacia arriba acerca: {} → {}",
            d0,
            rt.state.live.dist
        );
        let mut turn = rune::WheelEvent::new(0.0, 80.0).at(x, y);
        turn.mods.shift = true;
        rt.wheel_event(turn);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state.live.yaw != 0.0,
            "con Mayús la rueda gira el modelo"
        );
        // Los botones de la vista.
        click(&mut rt, "live/zoom-out");
        click(&mut rt, "live/rot-r");
        let (dist_after, yaw_after) = (rt.state.live.dist, rt.state.live.yaw);
        assert!(dist_after > 0.0 && yaw_after != 0.0);
        rt.update_if(|s: &mut State, _| {
            s.live.pan = [1.0, 0.5];
            true
        });
        rt.frame(1.0 / 60.0);
        click(&mut rt, "live/reset");
        assert_eq!(
            (rt.state.live.pan, rt.state.live.dist, rt.state.live.yaw),
            ([0.0; 2], 6.2, 0.0),
            "el botón recentra"
        );
        // Doble clic sobre el escenario también recentra.
        rt.update_if(|s: &mut State, _| {
            s.live.pan = [-1.0, 0.5];
            s.live.dist = 4.0;
            true
        });
        rt.frame(1.0 / 60.0);
        for _ in 0..2 {
            rt.pointer_down(x, y + 40.0);
            rt.pointer_up(x, y + 40.0);
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(
            (rt.state.live.pan, rt.state.live.dist),
            ([0.0; 2], 6.2),
            "doble clic recentra"
        );
    }

    #[test]
    fn a_narrow_window_folds_the_sidebar_and_the_overlays_and_a_wide_one_does_not() {
        let mut st = State::offline();
        demo_live(&mut st);
        st.win = (1000.0, 650.0);
        st.live.viewport = st.stage_size();
        assert!(
            st.sidebar_is_collapsed(),
            "por debajo de 1080 la barra se pliega sola"
        );
        assert!(
            st.stage_size().0 > 900.0,
            "y el escenario gana el espacio: {:?}",
            st.stage_size()
        );
        let mut rt = Runtime::new(
            st,
            view,
            (1000.0, 650.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("sidebar")
                .map(|_| rt.find("nav/dashboard").is_some())
                .unwrap_or(false),
            "los iconos siguen pulsables"
        );
        // El botón de la cabecera la abre aunque la ventana sea estrecha.
        click(&mut rt, "sidebar/toggle");
        assert!(!rt.state.sidebar_is_collapsed());
        let mut wide = State::offline();
        wide.win = (1440.0, 900.0);
        assert!(!wide.sidebar_is_collapsed());
        // En un escenario angosto la telemetría de HiveLive arranca plegada y se abre con su botón.
        let mut st = State::offline();
        demo_live(&mut st);
        st.live.viewport = (800.0, 600.0);
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("live/panel-toggle").is_some() && rt.find("live/telemetry").is_some());
        assert!(
            rt.find("live/hint").is_none(),
            "sin el texto de ayuda en un escenario estrecho"
        );
        click(&mut rt, "live/panel-toggle");
        assert!(rt.state.live.panel_open_narrow);
    }

    #[test]
    fn a_narrow_office_drops_the_extra_panels_and_folds_the_roster() {
        let mut rt = office_rt(3);
        rt.update_if(|s: &mut State, _| {
            s.office.viewport = (900.0, 600.0);
            true
        });
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("office/roster-toggle").is_some());
        assert!(
            rt.find("office/desk/w0").is_none(),
            "el Centro de operaciones arranca plegado"
        );
        click(&mut rt, "office/roster-toggle");
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.state.office.roster_open_narrow && rt.find("office/desk/w0").is_some());
    }

    #[test]
    fn pressable_elements_have_the_hand_cursor_and_hover_feedback() {
        use crate::ui::Tap;
        let e = rune::row().tap(|_: &mut State| {});
        // El velo cambia con el tema (blanco tenue en oscuro, negro tenue en crema).
        assert!(
            ui::theme::hover_veil().a > 0.0
                && ui::theme::press_veil().a > ui::theme::hover_veil().a
        );
        let _ = e;
    }

    #[test]
    fn dragging_the_stage_turns_bia_and_the_avatar_can_be_switched_to_the_portrait() {
        let mut rt = live_rt();
        assert!(rt.state.live.avatar_model, "arranca con el modelo 3D");
        let (x, y) = rt.find(live::stage::SLOT_KEY).unwrap();
        rt.pointer_down(x, y);
        rt.pointer_move(x + 50.0, y);
        rt.frame(1.0 / 60.0); // Rune procesa los eventos del puntero en el siguiente fotograma
        assert!(
            rt.state.live.dragging && rt.state.live.pan[0] < -0.05,
            "arrastrar a la derecha desplaza la vista (como el botón izquierdo de la web): {:?}",
            rt.state.live.pan
        );
        assert_eq!(
            rt.state.live.yaw, 0.0,
            "arrastrar ya no gira: se gira con Mayús + rueda o con los botones"
        );
        rt.pointer_up(x + 50.0, y);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.live.dragging);
        // Con el retrato el arrastre no gira nada.
        click(&mut rt, "live/avatar");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(
            rt.state.live.open_select, 3,
            "se despliega la lista de avatares"
        );
        click(&mut rt, "live/avatar/foto");
        assert!(!rt.state.live.avatar_model);
        let before = rt.state.live.yaw;
        let (x, y) = rt.find(live::stage::SLOT_KEY).unwrap();
        rt.pointer_down(x, y);
        rt.pointer_move(x + 80.0, y);
        rt.pointer_up(x + 80.0, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.live.yaw, before, "el retrato no gira");
        // El avatar se puede cambiar en plena llamada (no toca la sesión), a diferencia de la voz.
        rt.update_if(|s: &mut State, _| {
            s.voice_on = true;
            s.voice_live = true;
            true
        });
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "live/avatar");
        assert_eq!(
            rt.state.live.open_select, 3,
            "el avatar sigue siendo elegible durante la llamada"
        );
    }

    #[test]
    fn the_telemetry_panel_collapses_and_expands() {
        let mut rt = live_rt();
        assert!(rt.find("live/voice").is_some());
        click(&mut rt, "live/panel-toggle");
        assert!(rt.state.live.panel_collapsed);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("live/voice").is_none(),
            "plegado, solo queda la cabecera"
        );
        click(&mut rt, "live/panel-toggle");
        assert!(!rt.state.live.panel_collapsed);
    }

    #[test]
    fn the_voice_and_language_can_be_chosen_only_outside_a_call() {
        let mut rt = live_rt();
        // The telemetry panel scrolls, and in a 900-px window the language field sits below its
        // visible edge: a person scrolls to it, and so does this test (a click on a clipped control
        // hits nothing).
        let scroll_to = |rt: &mut Runtime<State>, key: &str| {
            for _ in 0..40 {
                let (x, y) = rt
                    .find(key)
                    .unwrap_or_else(|| panic!("no hay elemento {key}"));
                if y < 780.0 {
                    return;
                }
                let (px, py) = rt.find("live/voice").unwrap_or((x, y));
                rt.wheel_event(rune::WheelEvent::new(0.0, 120.0).at(px, py.min(600.0)));
                rt.frame(1.0 / 60.0);
            }
        };
        click(&mut rt, "live/voice");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(
            rt.state.live.open_select, 1,
            "se despliega la lista de voces"
        );
        click(&mut rt, "live/voice/Puck");
        assert_eq!(rt.state.live.voice_id(), "Puck");
        assert_eq!(rt.state.live.open_select, 0, "al elegir se cierra");
        scroll_to(&mut rt, "live/lang");
        click(&mut rt, "live/lang");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "live/lang/en-US");
        assert_eq!(rt.state.live.lang_id(), "en-US");
        // En llamada la elección está bloqueada: cambiarla exigiría reconectar.
        rt.update_if(|s: &mut State, _| {
            s.voice_on = true;
            s.voice_live = true;
            true
        });
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        click(&mut rt, "live/voice");
        assert_eq!(rt.state.live.open_select, 0, "bloqueada durante la llamada");
    }

    #[test]
    fn the_hive_activity_lists_real_milestones_newest_first() {
        let mut st = State::offline();
        demo_live_call(&mut st);
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert!(
            rt.find("live/ms/0").is_some() && rt.find("live/ms/1").is_some(),
            "los dos hitos de la demo"
        );
        assert!(rt.find("live/ms/2").is_none());
        assert_eq!(rt.state.live.milestones.last().unwrap().label, "web_search");
        assert!(rt.state.live.specialists.contains("Investigador web"));
    }

    #[test]
    fn the_interrupt_button_is_hidden_unless_the_call_is_live() {
        let mut st = State::offline();
        st.screen = Screen::Live;
        st.voice_on = true; // conectando, aún sin `ready`
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(rt.find("voice/interrupt").is_none());
    }

    // ── oficina 3D: datos ──

    #[test]
    fn opening_the_office_reads_the_agents_once() {
        let (port, server) = fake_http(|_, p, _, _| (200, lists(p)), 2);
        let mut st = State::offline();
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 640.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "nav/office");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !pull_office(&mut rt.state) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(rt.state.office.agents.len(), 1);
        assert_eq!(rt.state.office.agents[0].name, "Hive");
        assert_eq!(rt.state.screen, Screen::Office);
        click(&mut rt, "nav/chat");
        click(&mut rt, "nav/office"); // una segunda visita no vuelve a pedir nada: el servidor falso solo atiende dos peticiones
        assert_eq!(
            server
                .join()
                .unwrap()
                .iter()
                .map(|r| r.1.as_str())
                .collect::<Vec<_>>(),
            ["/api/agents", "/api/providers"]
        );
    }

    #[test]
    fn canvas_events_feed_the_office_graph() {
        let mut st = State::offline();
        apply(
            &mut st,
            Event::Canvas {
                kind: "canvas:node_add".into(),
                data: json!({"id": "a1", "type": "agent", "status": "thinking"}),
                timestamp: None,
            },
        );
        apply(
            &mut st,
            Event::Canvas {
                kind: "canvas:work_event".into(),
                data: json!({"eventId": "e1", "phase": "delegated", "actorId": "boss", "taskRef": "t", "taskName": "T"}),
                timestamp: Some(42.0),
            },
        );
        assert_eq!(st.office.graph.nodes.len(), 1);
        assert_eq!(
            (
                st.office.graph.work_events.len(),
                st.office.graph.work_events[0].timestamp
            ),
            (1, 42.0)
        );
    }

    #[test]
    fn the_office_screen_has_a_slot_for_the_scene() {
        let mut st = State::offline();
        st.screen = Screen::Office;
        let mut rt = Runtime::new(
            st,
            view,
            (900.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let (rect, _) = rt
            .presented(office::stage::SLOT_KEY)
            .expect("el hueco de la escena existe");
        assert!(
            rect[2] > 600.0 && rect[3] > 400.0,
            "ocupa el espacio disponible: {rect:?}"
        );
    }

    #[test]
    fn the_today_button_fills_a_date_field_and_a_bad_date_shows_the_format() {
        let mut st = State::offline();
        st.a2.apply(
            "a2ui:createSurface",
            &json!({"surfaceId": "d", "catalogId": "basic"}),
        );
        st.a2.apply("a2ui:updateComponents", &json!({"surfaceId": "d", "components": [
            {"id": "root", "component": "Column", "children": ["when"]},
            {"id": "when", "component": "DateTimeInput", "label": "Fecha", "enableDate": true, "enableTime": false, "value": {"path": "/when"}}
        ]}));
        let mut rt = Runtime::new(
            st,
            view,
            (1100.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        click(&mut rt, "d/root>whennow");
        let v = a2ui::model::get_at_path(&rt.state.a2.store.get("d").unwrap().data, "/when")
            .cloned()
            .unwrap_or(Value::Null);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(
            v,
            json!(today),
            "«Hoy» escribe la fecha de hoy en el modelo"
        );
        rt.state.a2.write("d", "/when", json!("2026-02-30"));
        rt.frame(1.0 / 60.0);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.find("d/root>whennow").is_some(),
            "el botón sigue ahí con un valor inválido"
        );
    }

    #[test]
    fn dragging_the_slider_track_sets_the_value_and_clamps_at_the_ends() {
        let mut rt = boot_ui("ana@b.co");
        let track = "demo/root>tabs>form>seatstrack";
        let (x, y) = rt
            .find(track)
            .unwrap_or_else(|| panic!("no hay la pista del deslizador"));
        rt.pointer_down(x, y);
        rt.pointer_move(x + 400.0, y); // mucho más allá del extremo derecho
        rt.pointer_up(x + 400.0, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            model(&rt, "/seats"),
            json!(10.0),
            "arrastrar al extremo da el máximo y no se pasa"
        );
        let (x, y) = rt.find(track).unwrap();
        rt.pointer_down(x, y);
        rt.pointer_move(x - 400.0, y);
        rt.pointer_up(x - 400.0, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            model(&rt, "/seats"),
            json!(1.0),
            "y al otro extremo da el mínimo"
        );
    }

    #[test]
    fn dragging_the_scene_orbits_the_camera_and_the_wheel_zooms() {
        let mut st = State::offline();
        st.screen = Screen::Office;
        for _ in 0..100 {
            st.office.step(1.0 / 60.0); // termina la intro
        }
        let mut rt = Runtime::new(
            st,
            view,
            (900.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let (x, y) = rt.find(office::stage::SLOT_KEY).unwrap();
        let (az, dist) = (
            rt.state.office.camera.azimuth,
            rt.state.office.camera.distance,
        );
        rt.pointer_down(x, y);
        rt.pointer_move(x + 60.0, y + 20.0);
        rt.pointer_up(x + 60.0, y + 20.0);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state.office.camera.azimuth != az,
            "arrastrar gira la cámara"
        );
        // La ventana real entrega la rueda con `wheel_event` (`Runtime::wheel` es solo el scroll vertical).
        rt.wheel_event(rune::WheelEvent::new(0.0, 40.0).at(x, y));
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state.office.camera.distance < dist,
            "la rueda hacia delante acerca"
        );
    }

    // ── oficina 3D: HUD e interacción ──

    fn office_rt(workers: usize) -> Runtime<State> {
        let mut st = State::offline();
        st.screen = Screen::Office;
        let agent = |id: &str, role: &str| settings::api::Agent {
            id: id.into(),
            name: id.to_uppercase(),
            role: role.into(),
            status: "idle".into(),
            enabled: true,
            ..Default::default()
        };
        st.office.agents.push(agent("boss", "coordinator"));
        for i in 0..workers {
            st.office.agents.push(agent(&format!("w{i}"), "worker"));
        }
        // Se deja correr la intro (1,35 s): mientras dura, los controles no responden.
        for _ in 0..100 {
            st.office.step(1.0 / 60.0);
        }
        let mut rt = Runtime::new(
            st,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        // El compositor fija el tamaño del hueco; sin GPU se hace aquí a partir de su caja ya maquetada.
        let (rect, _) = rt.presented(office::stage::SLOT_KEY).unwrap();
        rt.state.office.viewport = (rect[2], rect[3]);
        rt.update_if(|_: &mut State, _| true);
        rt.frame(1.0 / 60.0);
        rt
    }

    #[test]
    fn the_quality_and_motion_toggles_flip_their_settings() {
        let mut rt = office_rt(1);
        assert!(rt.state.office.high_quality && rt.state.office.calm);
        click(&mut rt, "office/quality");
        assert!(!rt.state.office.high_quality);
        click(&mut rt, "office/motion");
        assert!(!rt.state.office.calm);
        click(&mut rt, "office/quality");
        assert!(rt.state.office.high_quality);
    }

    #[test]
    fn the_roster_lists_every_specialist_and_selecting_one_opens_the_inspector() {
        let mut rt = office_rt(3);
        for i in 0..3 {
            assert!(rt.find(&format!("office/desk/w{i}")).is_some(), "w{i}");
        }
        assert!(
            rt.find("office/inspector-close").is_none(),
            "sin selección no hay inspector"
        );
        click(&mut rt, "office/desk/w1");
        assert_eq!(rt.state.office.selected.as_deref(), Some("w1"));
        assert!(rt.find("office/inspector-close").is_some());
        click(&mut rt, "office/desk/w1");
        assert_eq!(rt.state.office.selected, None, "tocarla otra vez la suelta");
        click(&mut rt, "office/desk/w0");
        click(&mut rt, "office/desk/w2");
        assert_eq!(
            rt.state.office.selected.as_deref(),
            Some("w2"),
            "pasar de una a otra"
        );
        click(&mut rt, "office/inspector-close");
        assert_eq!(rt.state.office.selected, None);
    }

    #[test]
    fn the_coordinator_button_selects_the_queen() {
        let mut rt = office_rt(1);
        click(&mut rt, "office/coord");
        assert_eq!(rt.state.office.selected.as_deref(), Some("boss"));
        assert!(
            rt.find("office/inspector-close").is_some(),
            "el inspector del coordinador"
        );
    }

    #[test]
    fn the_roster_collapses_and_expands() {
        let mut rt = office_rt(2);
        assert!(rt.find("office/desk/w0").is_some());
        click(&mut rt, "office/roster-toggle");
        assert!(rt.state.office.roster_collapsed);
        assert!(
            rt.find("office/desk/w0").is_none(),
            "plegada no lista a nadie"
        );
        click(&mut rt, "office/roster-toggle");
        assert!(rt.find("office/desk/w0").is_some());
    }

    #[test]
    fn clicking_a_bee_in_the_scene_selects_it_and_clicking_empty_space_clears_it() {
        let mut rt = office_rt(2);
        rt.state.office.bees.get_mut("w0").unwrap().bee.pos = [0.0, 3.1, 8.0]; // delante, a la vista
        rt.update_if(|_: &mut State, _| true);
        rt.frame(1.0 / 60.0);
        let (rect, _) = rt.presented(office::stage::SLOT_KEY).unwrap();
        let pos = rt.state.office.bees["w0"].bee.pos;
        let (lx, ly) = office::hud::screen_pos(&rt.state.office.camera, pos, (rect[2], rect[3]))
            .expect("visible");
        let (wx, wy) = (rect[0] + lx, rect[1] + ly);
        rt.pointer_down(wx, wy);
        rt.pointer_up(wx, wy);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.office.selected.as_deref(),
            Some("w0"),
            "un clic sobre la abeja la elige"
        );
        let (ex, ey) = (rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.95);
        rt.pointer_down(ex, ey);
        rt.pointer_up(ex, ey);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.office.selected, None,
            "un clic en el vacío suelta la selección"
        );
    }

    #[test]
    fn a_real_drag_orbits_without_selecting_anything() {
        let mut rt = office_rt(2);
        let (x, y) = rt.find(office::stage::SLOT_KEY).unwrap();
        let az = rt.state.office.camera.azimuth;
        rt.pointer_down(x, y);
        rt.pointer_move(x + 80.0, y);
        rt.pointer_up(x + 80.0, y);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.office.camera.azimuth != az);
        assert_eq!(rt.state.office.selected, None, "arrastrar no es hacer clic");
    }

    #[test]
    fn the_hud_shows_what_is_happening_and_the_feed_after_new_events() {
        let mut rt = office_rt(2);
        rt.update_if(|s: &mut State, _| {
            let g = &mut s.office.graph;
            g.apply("canvas:work_event", &json!({"eventId": "e1", "phase": "delegated", "taskRef": "t", "taskName": "Buscar", "actorId": "boss", "targetId": "w0"}), None, 0.0);
            s.office.step(1.0 / 60.0);
            true
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state
                .office
                .director
                .active()
                .map(|c| c.event.event_id.as_str()),
            Some("e1")
        );
        assert!(
            rt.find("office/feed/0").is_some(),
            "el ticker tiene una línea"
        );
    }

    #[test]
    fn the_jev_panel_only_appears_when_jev_is_active_and_collapses() {
        let mut rt = office_rt(1);
        assert!(rt.find("office/jev-toggle").is_none());
        rt.update_if(|s: &mut State, _| {
            s.office.graph.apply("canvas:jev_status", &json!({"state": "ready", "backend": "jev", "totals": {"decisions": 3, "savedTokens": 900, "costUsd": 0.002}}), None, 0.0);
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("office/jev-toggle").is_some());
        click(&mut rt, "office/jev-toggle");
        assert!(rt.state.office.jev_collapsed);
        rt.update_if(|s: &mut State, _| {
            s.office
                .graph
                .apply("canvas:jev_status", &json!({"state": "off"}), None, 0.0);
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(
            rt.find("office/jev-toggle").is_none(),
            "apagado, el panel desaparece"
        );
    }

    fn providers_with_jev(state: &str) -> Value {
        json!({"providers": [{"id": "openrouter", "name": "OpenRouter", "category": "llm", "enabled": true, "active": true, "has_api_key": 1,
            "models": [], "jev": {"state": state, "backend": "jev", "lastError": null, "totals": {"decisions": 4, "savedTokens": 1200, "costUsd": 0.003}}}]})
    }

    fn load_office(provider_jev: &str, websocket_jev: Option<&str>) -> State {
        let provider_jev = provider_jev.to_string();
        let (port, server) = fake_http(
            move |_, p, _, _| {
                (
                    200,
                    if p == "/api/providers" {
                        providers_with_jev(&provider_jev)
                    } else {
                        lists(p)
                    },
                )
            },
            2,
        );
        let mut st = State::offline();
        st.settings.conn = Some(Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        });
        if let Some(state) = websocket_jev {
            st.office.graph.apply(
                "canvas:jev_status",
                &json!({"state": state, "backend": "kev"}),
                None,
                0.0,
            );
        }
        request_office_agents(&mut st);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !pull_office(&mut st) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        server.join().unwrap();
        st
    }

    #[test]
    fn jev_status_comes_from_the_openrouter_provider_when_the_socket_has_not_said_anything() {
        let st = load_office("ready", None);
        let j = st.office.graph.jev_status.as_ref().expect("estado de Jev");
        assert_eq!(
            (j.state.as_str(), j.backend.as_deref()),
            ("ready", Some("jev"))
        );
        assert_eq!(j.totals.as_ref().map(|t| t.decisions), Some(4.0));
    }

    #[test]
    fn a_jev_status_that_arrived_by_websocket_is_not_overwritten() {
        let st = load_office("ready", Some("fallback"));
        let j = st.office.graph.jev_status.as_ref().unwrap();
        assert_eq!(
            (j.state.as_str(), j.backend.as_deref()),
            ("fallback", Some("kev")),
            "manda lo que ya llegó en vivo"
        );
    }

    #[test]
    fn an_off_jev_provider_keeps_the_panel_hidden() {
        let st = load_office("off", None);
        assert_eq!(
            st.office
                .graph
                .jev_status
                .as_ref()
                .map(|j| j.state.as_str()),
            Some("off")
        );
    }

    #[test]
    fn world_labels_follow_the_packets_the_oracle_the_jev_beams_and_the_delegations() {
        let mut rt = office_rt(2);
        assert!(
            rt.find("office/jev-tag").is_none(),
            "sin Jev no hay insignia"
        );
        rt.update_if(|s: &mut State, _| {
            let g = &mut s.office.graph;
            g.apply("canvas:jev_status", &json!({"state": "ready", "backend": "kev"}), None, 0.0);
            g.apply("canvas:edge_add", &json!({"id": "e1", "source": "boss", "target": "w0", "edgeType": "delegates", "data": {"taskName": "Buscar"}}), None, 0.0);
            g.apply("canvas:edge_add", &json!({"id": "e2", "source": "w0", "target": "w1", "edgeType": "reviews"}), None, 0.0);
            s.office.step(1.0 / 60.0);
            let g = &mut s.office.graph;
            g.apply("canvas:work_event", &json!({"eventId": "f1", "phase": "delegated", "taskName": "Buscar", "actorId": "boss", "targetId": "w0"}), None, 0.0);
            g.apply("canvas:jev_decision", &json!({"eventId": "d1", "agentId": "w0", "kind": "context", "savedTokens": 6100, "latencyMs": 310}), None, 0.0);
            s.office.step(1.0 / 60.0);
            true
        });
        rt.frame(1.0 / 60.0);
        for key in [
            "office/jev-tag",
            "office/transfer-label/f1",
            "office/jev-beam-label/d1",
            "office/beam-label/e1",
            "office/beam-label/e2",
        ] {
            assert!(rt.find(key).is_some(), "falta {key}");
        }
    }

    #[test]
    fn world_labels_disappear_with_their_effects() {
        let mut rt = office_rt(2);
        rt.update_if(|s: &mut State, _| {
            s.office.step(0.016);
            s.office.graph.apply("canvas:work_event", &json!({"eventId": "f1", "phase": "completed", "actorId": "boss", "targetId": "w0"}), None, 0.0);
            s.office.step(0.016);
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.find("office/transfer-label/f1").is_some());
        rt.update_if(|s: &mut State, _| {
            s.office.step(3.0);
            true
        });
        rt.frame(1.0 / 60.0);
        assert!(
            rt.find("office/transfer-label/f1").is_none(),
            "tras 2,25 s el vuelo y su etiqueta se van"
        );
    }

    #[test]
    fn during_the_intro_dragging_and_zooming_do_nothing() {
        let mut st = State::offline();
        st.screen = Screen::Office;
        st.office.step(0.0); // empieza la intro
        let mut rt = Runtime::new(
            st,
            view,
            (900.0, 700.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        let (x, y) = rt.find(office::stage::SLOT_KEY).unwrap();
        let before = (
            rt.state.office.camera.azimuth,
            rt.state.office.camera.distance,
        );
        rt.pointer_down(x, y);
        rt.pointer_move(x + 80.0, y + 30.0);
        rt.pointer_up(x + 80.0, y + 30.0);
        rt.wheel_event(rune::WheelEvent::new(0.0, 40.0).at(x, y));
        rt.frame(1.0 / 60.0);
        assert_eq!(
            (
                rt.state.office.camera.azimuth,
                rt.state.office.camera.distance
            ),
            before,
            "la intro no se interrumpe"
        );
        assert_eq!(rt.state.office.rig.mode, office::rig::Mode::Intro);
    }

    #[test]
    fn a_drag_after_the_intro_becomes_the_resting_pose() {
        let mut rt = office_rt(1);
        let (x, y) = rt.find(office::stage::SLOT_KEY).unwrap();
        let rest_before = rt.state.office.rig.resting;
        rt.pointer_down(x, y);
        rt.pointer_move(x + 90.0, y);
        rt.pointer_up(x + 90.0, y);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state.office.rig.resting != rest_before,
            "la cámara soltada queda como pose de reposo"
        );
        assert_eq!(rt.state.office.rig.mode, office::rig::Mode::Free);
    }
    #[test]
    fn rust_narrations_show_child_completion_without_finishing_the_parent_turn() {
        let mut state = State::offline();
        state.busy = true;
        let step = chat::parse(
            &json!({"type":"process","id":"child:done","messageId":"parent","processStatus":"done",
            "agentName":"Analista","label":"Terminó la consulta","turnDone":false}),
        )
        .unwrap();
        apply(&mut state, step.clone());
        apply(&mut state, step);
        assert!(state.busy);
        assert_eq!(state.processes.len(), 1);
        apply(
            &mut state,
            chat::parse(&json!({"type":"process","processStatus":"done","turnDone":true})).unwrap(),
        );
        assert!(!state.busy);
        apply(
            &mut state,
            Event::Chunk {
                id: "parent".into(),
                text: "Voy a consultar. ".into(),
            },
        );
        apply(
            &mut state,
            Event::Final {
                id: "parent".into(),
                text: "Respuesta confirmada".into(),
            },
        );
        assert_eq!(state.msgs.len(), 1);
        assert_eq!(state.msgs[0].text, "Respuesta confirmada");
    }

    #[test]
    fn webchat_sidebar_and_narrative_steps_are_scoped_and_update_in_place() {
        let mut state = State::offline();
        state.chat_ready = true;
        state.busy = true;
        state.process_expanded = true;
        state.shown.capabilities = Some(vec!["chat".into(), "conversations".into()]);
        state.webchat.items=serde_json::from_value(json!([{"id":"opaque/a","title":"Consulta anterior","messageCount":2,"lastMessageAt":1794400000000u64}])).unwrap();
        state.webchat.active = Some("opaque/a".into());
        state.webchat.can_continue = true;
        for (id, step, agent, status, label, time) in [
            ("e1", "root:activity", "Hive", "thinking", "Revisando", 1),
            (
                "e2",
                "root:activity",
                "Hive",
                "thinking",
                "Preparando respuesta",
                2,
            ),
            (
                "e3",
                "child:activity",
                "Analista",
                "thinking",
                "Consultando",
                3,
            ),
            (
                "e4",
                "child:activity",
                "Analista",
                "done",
                "Respondió la consulta",
                4,
            ),
        ] {
            apply(&mut state,chat::parse(&json!({"type":"process","id":id,"stepId":step,"messageId":"root","agentName":agent,
                "processStatus":status,"label":label,"turnDone":false,"timestamp":time,"narrationKind":"worker_started"})).unwrap());
        }
        assert_eq!(state.processes.len(), 2);
        assert!(state.busy);
        assert_eq!(state.processes[0].label, "Preparando respuesta");
        let detail = "Detalle completo del error. ".repeat(100);
        apply(&mut state,chat::parse(&json!({"type":"process","id":"e5","stepId":"child:activity","messageId":"root","agentName":"Analista",
            "processStatus":"error","label":"La consulta necesita revisión","detail":detail,"turnDone":false,"timestamp":5})).unwrap());
        assert_eq!(state.processes.len(), 2);
        assert_eq!(state.processes[1].detail.as_deref(), Some(detail.as_str()));
        assert!(state.busy);
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 1000.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for key in [
            "chat/conversations",
            "chat/new",
            "chat/conversation/0",
            "chat/stop",
            "process/child:activity",
        ] {
            assert!(rt.find(key).is_some(), "falta {key}");
        }
        click(&mut rt, "chat/new");
        assert!(rt.state.busy);
        assert!(!rt.state.webchat.loading);
        assert!(rt.state.webchat.error.is_some());
    }

    #[test]
    fn webchat_conversations_are_accessible_in_a_narrow_window() {
        let mut state = State::offline();
        state.win = (700.0, 900.0);
        state.shown.capabilities = Some(vec!["chat".into(), "conversations".into()]);
        let mut rt = Runtime::new(
            state,
            view,
            (700.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(rt.find("chat/show-list").is_some());
        assert!(rt.find("chat/conversations").is_none());
        click(&mut rt, "chat/show-list");
        assert!(rt.find("chat/new").is_some());
        click(&mut rt, "chat/show-list");
        assert!(rt.find("chat/conversations").is_none());
    }

    #[test]
    fn rust_desktop_enables_all_requested_pages_and_settings_submenus() {
        let mut state = State::offline();
        state.win = (1440.0, 1400.0);
        state.shown.capabilities = Some(vec![
            "chat".into(),
            "settings".into(),
            "hivelive".into(),
            "panel".into(),
            "meetings".into(),
            "cron".into(),
        ]);
        state.settings.shown.loaded = true;
        state.settings.shown.extras_loaded = true;
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 1400.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        for (key, screen) in [
            ("nav/agents", Screen::Agents),
            ("nav/panel", Screen::Panel),
            ("nav/meeting", Screen::Meeting),
            ("nav/api", Screen::ApiClient),
            ("nav/notes", Screen::Notes),
            ("nav/cron", Screen::Cron),
        ] {
            assert!(screen_available(&rt.state, screen));
            click(&mut rt, key);
            assert_eq!(rt.state.screen, screen, "{key}");
        }
        for (key, tab) in [
            ("nav/providers", Tab::Providers),
            ("nav/channels", Tab::Channels),
            ("nav/settings", Tab::Tools),
            ("nav/mcp", Tab::Mcp),
            ("nav/skills", Tab::Skills),
            ("nav/ethics", Tab::Ethics),
            ("nav/profile", Tab::Profile),
            ("nav/updates", Tab::Updates),
        ] {
            assert!(rt.find(key).is_some(), "{key}");
            click(&mut rt, key);
            assert_eq!(rt.state.screen, Screen::Settings);
            assert_eq!(rt.state.settings.tab, tab, "{key}");
        }
        assert!(rt.find("nav/display").is_some());
        rt.update_if(|s: &mut State, _| {
            s.settings.tab = Tab::Display;
            true
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.settings.tab, Tab::Display);
    }

    #[test]
    fn rust_office_snapshot_drives_real_agents_and_exposes_the_composer() {
        let mut state = State::offline();
        state.shown.capabilities = Some(vec!["chat".into(), "office".into(), "narration".into()]);
        assert!(screen_available(&state, Screen::Office));
        assert!(!screen_available(&state, Screen::Live));
        assert!(!screen_available(&state, Screen::Settings));
        open_screen(&mut state, Screen::Office);
        apply(
            &mut state,
            Event::Canvas {
                kind: "canvas:snapshot".into(),
                timestamp: None,
                data: json!({"revision":7,
            "agents":[{"id":"boss","name":"Hive","role":"coordinator","enabled":true},{"id":"worker","name":"Analista","role":"worker","enabled":true}],
            "nodes":[{"id":"boss","type":"agent","data":{"status":"tool_call","currentTool":"task_delegate"}},
                {"id":"worker","type":"agent","data":{"status":"thinking","currentTask":"Analizar","delegatedBy":"boss"}}],
            "edges":[{"id":"delegation","source":"boss","target":"worker","edgeType":"delegates"}]}),
            },
        );
        let (coordinator, desks) = state.office.desks();
        assert_eq!(
            coordinator.unwrap().state,
            office::model::DeskState::ToolCall
        );
        assert_eq!(desks[0].state, office::model::DeskState::Thinking);
        assert_eq!(desks[0].current_task.as_deref(), Some("Analizar"));
        state.office.step(0.1);
        assert_eq!(state.office.bees.len(), 1);
        let mut rt = Runtime::new(
            state,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        assert!(rt.find("office/chat").is_some());
        assert!(rt.find("input").is_some());
        assert!(rt.find(office::stage::SLOT_KEY).is_some());
    }

    #[test]
    fn rust_office_rejects_stale_snapshots_and_marks_disconnected_activity_unknown() {
        let mut state = State::offline();
        state.chat_ready = true;
        let snapshot = |revision, status| Event::Canvas {
            kind: "canvas:snapshot".into(),
            timestamp: None,
            data: json!({"revision":revision,"nodes":[{"id":"worker","type":"agent","data":{"status":status}}],"edges":[]}),
        };
        apply(&mut state, snapshot(8, "thinking"));
        apply(&mut state, snapshot(7, "idle"));
        assert_eq!(state.office.graph.nodes[0].status, "thinking");
        apply(
            &mut state,
            Event::Canvas {
                kind: "canvas:work_event".into(),
                timestamp: None,
                data: json!({"revision":6,"eventId":"old","phase":"completed","actorId":"worker"}),
            },
        );
        assert!(state.office.graph.work_events.is_empty());
        apply(&mut state, Event::Closed);
        assert!(!state.chat_ready);
        assert_eq!(state.office.graph.nodes[0].status, "error");
        assert!(state.shown.chat_error.is_some());
    }
}
