//! Cliente de chat del gateway: REST para preparar la conversación y WebSocket (/ws) para el turno.
//! Protocolo tomado de `hive-ui/src/stores/useWebSocketStore.ts` y `core/src/gateway/webchat-turn.ts`.

use std::{
    net::TcpStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde_json::{json, Value};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessStep {
    pub turn_id: String,
    pub kind: String,
    pub timestamp: u64,
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub agent: Option<String>,
    pub status: String,
    pub finished: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Progress(ProcessStep),
    Connected {
        agent: Option<String>,
    },
    /// Fragmento de la respuesta; los que comparten `id` se concatenan.
    Chunk {
        id: String,
        text: String,
    },
    /// Mensaje completo (si el turno ya se emitió en fragmentos, el cliente lo ignora).
    Final {
        id: String,
        text: String,
    },
    /// Fin del turno (`process` en `done` o `error`).
    Done,
    Error(String),
    Closed,
    /// Frame `a2ui:*` del canvas: tipo y `data`.
    A2ui {
        kind: String,
        data: Value,
    },
    /// Frame `canvas:*` (grafo, eventos de trabajo y decisiones de Jev) con la hora del mensaje, si la trae.
    Canvas {
        kind: String,
        data: Value,
        timestamp: Option<f64>,
    },
    /// Aviso del servidor (recordatorios, resultados de tareas programadas). Hay que confirmarlo con `ack`.
    Notification {
        id: String,
        content: String,
    },
    /// El gateway pide permiso para ejecutar una herramienta (política `ask`). El turno espera
    /// hasta `expires_in_ms`; sin respuesta se deniega. Se contesta con `approval_frame`.
    Approval {
        id: String,
        tool: String,
        arguments: Value,
        reason: Option<String>,
        expires_in_ms: u64,
    },
}

/// Plazo que se asume si el servidor no lo dice, y sus límites (los del gateway: 1 s a 1 h).
const APPROVAL_DEFAULT_MS: u64 = 120_000;

pub struct Chat {
    pub thread_id: String,
    pub user_id: String,
    pub token: Option<String>,
    out: Sender<String>,
    events: Arc<Mutex<Vec<QueuedEvent>>>,
}
struct QueuedEvent {
    thread_id: Option<String>,
    event: Event,
}

/// Por qué no se pudo preparar la conversación.
#[derive(Debug, PartialEq)]
pub enum BootError {
    /// El gateway exige credenciales y no hay un token válido: hay que iniciar sesión.
    NeedsLogin,
    Msg(String),
}

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootError::NeedsLogin => write!(f, "Inicia sesión para continuar"),
            BootError::Msg(m) => write!(f, "{m}"),
        }
    }
}

impl From<String> for BootError {
    fn from(m: String) -> Self {
        BootError::Msg(m)
    }
}

impl From<&str> for BootError {
    fn from(m: &str) -> Self {
        BootError::Msg(m.to_string())
    }
}

/// Lo que se averigua por REST antes de abrir el WebSocket.
#[derive(Debug, PartialEq)]
pub struct Prepared {
    pub user_id: String,
    pub thread_id: String,
    pub token: Option<String>,
}

/// Inicia sesión con correo y contraseña y devuelve el token. El token vive solo en memoria.
pub fn login(base: &str, email: &str, password: &str) -> Result<String, String> {
    let resp = ureq::post(&format!("{base}/api/auth/login"))
        .send_json(json!({"email": email, "password": password}));
    match resp {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|e| e.to_string())?;
            v["authToken"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "El gateway no devolvió un token".into())
        }
        Err(ureq::Error::Status(401, _)) => Err("Correo o contraseña incorrectos".into()),
        Err(e) => Err(format!("No se pudo iniciar sesión: {e}")),
    }
}

/// Estado, credenciales, usuario y conversación nueva. Sin tocar el WebSocket.
pub fn prepare(base: &str, token: Option<&str>) -> Result<Prepared, BootError> {
    prepare_mode(base, token, true)
}
fn prepare_mode(base: &str, token: Option<&str>, reuse: bool) -> Result<Prepared, BootError> {
    let status: Value = get(base, "/api/setup/status", None)?;
    if !status["configured"].as_bool().unwrap_or(false) {
        return Err(status["error"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!("Hive no está configurado: completa el asistente en {base}/setup")
            })
            .into());
    }
    let auth: Value = get(base, "/api/auth/status", None)?;
    if auth["hasCredentials"].as_bool().unwrap_or(false) && token.is_none() {
        return Err(BootError::NeedsLogin);
    }
    let users = get(base, "/api/users", token)?;
    let user_id = users
        .as_array()
        .or_else(|| users["users"].as_array())
        .and_then(|u| u.first())
        .and_then(|u| u["id"].as_str())
        .ok_or("El gateway no tiene usuarios")?
        .to_string();
    let conversations: Value = if reuse {
        match ureq::get(&format!("{base}/api/conversations?channel=webchat"))
            .set_opt_auth(token)
            .call()
        {
            Ok(response) => response.into_json().map_err(|error| error.to_string())?,
            Err(ureq::Error::Status(404 | 501, _)) => json!({"conversations":[]}),
            Err(ureq::Error::Status(401, _)) => return Err(BootError::NeedsLogin),
            Err(error) => {
                return Err(format!("No se pudieron cargar las conversaciones: {error}").into())
            }
        }
    } else {
        json!({"conversations":[]})
    };
    if let Some(id) = conversations["conversations"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|row| row["id"].as_str())
    {
        return Ok(Prepared {
            user_id,
            thread_id: id.to_owned(),
            token: token.map(str::to_owned),
        });
    }
    let conv: Value = match ureq::post(&format!("{base}/api/conversations"))
        .set_opt_auth(token)
        .send_json(json!({}))
    {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(401, _)) => return Err(BootError::NeedsLogin),
        Err(e) => return Err(format!("No se pudo crear la conversación: {e}").into()),
    };
    let thread_id = conv["conversation"]["id"]
        .as_str()
        .ok_or("Respuesta sin id de conversación")?
        .to_string();
    Ok(Prepared {
        user_id,
        thread_id,
        token: token.map(str::to_string),
    })
}

impl Chat {
    /// Explicit CLI probes keep their isolated conversation instead of appending to a saved chat.
    pub fn bootstrap_fresh(port: u16, token: Option<&str>) -> Result<Chat, BootError> {
        let prepared = prepare_mode(&format!("http://127.0.0.1:{port}"), token, false)?;
        Self::connect(
            port,
            &prepared.user_id,
            prepared.token.as_deref(),
            prepared.thread_id,
        )
        .map_err(BootError::from)
    }
    /// Prepara una conversación nueva y abre el WebSocket. `token`: el de un inicio de sesión;
    /// sin él se prueba `HIVE_AUTH_TOKEN`. Un token que el gateway rechaza cuenta como `NeedsLogin`.
    pub fn bootstrap(port: u16, token: Option<&str>) -> Result<Chat, BootError> {
        let base = format!("http://127.0.0.1:{port}");
        let token = token
            .map(str::to_string)
            .or_else(|| std::env::var("HIVE_AUTH_TOKEN").ok());
        let p = prepare(&base, token.as_deref())?;
        Ok(Self::connect(
            port,
            &p.user_id,
            p.token.as_deref(),
            p.thread_id,
        )?)
    }

    pub fn connect(
        port: u16,
        session: &str,
        token: Option<&str>,
        thread_id: String,
    ) -> Result<Chat, String> {
        let mut url = format!("ws://127.0.0.1:{port}/ws?session={}", urlenc(session));
        if let Some(t) = token {
            url.push_str(&format!("&token={}", urlenc(t)));
        }
        let (mut socket, _) =
            tungstenite::connect(url).map_err(|e| format!("No se pudo abrir el WebSocket: {e}"))?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            // Lectura con tope para poder atender también los mensajes salientes en el mismo hilo.
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        }
        let (out, rx) = mpsc::channel::<String>();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        thread::spawn(move || run(socket, rx, sink));
        // Sin esta suscripción el servidor no envía superficies A2UI.
        let _ = out.send(json!({"type": "canvas_subscribe"}).to_string());
        // Avisos pendientes: al conectar y luego cada 30 s, como la web (el servidor solo reenvía los no entregados).
        let _ = out.send(json!({"type": "notification_sync", "sessionId": session}).to_string());
        let (tick, sid) = (out.clone(), session.to_string());
        thread::spawn(move || loop {
            thread::sleep(NOTIFICATION_SYNC);
            if tick
                .send(json!({"type": "notification_sync", "sessionId": sid}).to_string())
                .is_err()
            {
                return; // la conexión se cerró
            }
        });
        Ok(Chat {
            thread_id,
            user_id: session.to_string(),
            token: token.map(str::to_string),
            out,
            events,
        })
    }

    /// Envía un turno de usuario.
    pub fn send(&self, content: &str) {
        let frame = json!({
            "type": "message",
            "content": content,
            "threadId": self.thread_id,
            "timestamp": chrono_now(),
        });
        let _ = self.out.send(frame.to_string());
    }

    /// Confirma un aviso para que el servidor no lo vuelva a mandar.
    pub fn ack(&self, id: &str) {
        let _ = self
            .out
            .send(json!({"type": "notification_ack", "notificationId": id}).to_string());
    }

    /// Envía un frame ya armado (p. ej. `a2ui:action`).
    pub fn send_raw(&self, frame: &Value) {
        let _ = self.out.send(frame.to_string());
    }

    /// Pide cancelar el turno en curso.
    pub fn stop(&self) {
        let _ = self
            .out
            .send(json!({"type": "stop", "threadId": self.thread_id}).to_string());
    }

    pub fn drain(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
            .into_iter()
            .filter(|queued| {
                queued
                    .thread_id
                    .as_ref()
                    .is_none_or(|id| id == &self.thread_id)
            })
            .map(|queued| queued.event)
            .collect()
    }
    pub fn select_thread(&mut self, id: String) {
        self.thread_id = id;
    }
}

trait AuthExt {
    fn set_opt_auth(self, token: Option<&str>) -> Self;
}
impl AuthExt for ureq::Request {
    fn set_opt_auth(self, token: Option<&str>) -> Self {
        match token {
            Some(t) => self.set("Authorization", &format!("Bearer {t}")),
            None => self,
        }
    }
}

fn get(base: &str, path: &str, token: Option<&str>) -> Result<Value, BootError> {
    match ureq::get(&format!("{base}{path}"))
        .set_opt_auth(token)
        .call()
    {
        Ok(r) => r
            .into_json()
            .map_err(|e| BootError::Msg(format!("GET {path}: {e}"))),
        Err(ureq::Error::Status(401, _)) => Err(BootError::NeedsLogin),
        Err(e) => Err(BootError::Msg(format!("GET {path}: {e}"))),
    }
}

pub fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// El frame con el que se contesta una aprobación por el WebSocket del turno. Solo vale una vez;
/// el servidor ignora las repeticiones.
pub fn approval_frame(id: &str, approved: bool) -> Value {
    json!({"type": "approval", "approvalId": id, "approved": approved})
}

fn chrono_now() -> String {
    // El servidor solo lo guarda; epoch en ms basta y evita otra dependencia.
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    ms.to_string()
}

const NOTIFICATION_SYNC: std::time::Duration = std::time::Duration::from_secs(30);

fn run(
    mut socket: WebSocket<MaybeTlsStream<TcpStream>>,
    outgoing: Receiver<String>,
    events: Arc<Mutex<Vec<QueuedEvent>>>,
) {
    let push = |event: Event| {
        events.lock().unwrap().push(QueuedEvent {
            thread_id: None,
            event,
        })
    };
    // El servidor da un `connId` al conectar y lo comprueba con `canvas:ping`.
    let mut conn_id: Option<String> = None;
    loop {
        while let Ok(frame) = outgoing.try_recv() {
            if socket.send(Message::text(frame)).is_err() {
                push(Event::Closed);
                return;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if v["type"] == "ping" {
                    let _ = socket.send(Message::text(json!({"type": "pong"}).to_string()));
                    continue;
                }
                if v["type"] == "canvas:connected" {
                    conn_id = v["data"]["connId"]
                        .as_str()
                        .or_else(|| v["connId"].as_str())
                        .map(str::to_string);
                    continue;
                }
                if v["type"] == "canvas:ping" {
                    let id = v["data"]["connId"]
                        .as_str()
                        .or_else(|| v["connId"].as_str())
                        .map(str::to_string)
                        .or_else(|| conn_id.clone());
                    let _ = socket.send(Message::text(
                        json!({"type": "canvas:pong", "connId": id}).to_string(),
                    ));
                    continue;
                }
                if let Some(e) = parse(&v) {
                    // Una aprobación es de la persona, no del hilo que esté a la vista: si cambió de
                    // conversación mientras tanto, igualmente hay un turno esperando su respuesta.
                    let thread_id = if v["type"] == "approval_request" {
                        None
                    } else {
                        v["threadId"].as_str().map(str::to_owned)
                    };
                    events.lock().unwrap().push(QueuedEvent {
                        thread_id,
                        event: e,
                    });
                }
            }
            Ok(Message::Close(_)) => {
                push(Event::Closed);
                return;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                push(Event::Closed);
                return;
            }
        }
    }
}

/// Traduce un frame del servidor a un evento del cliente.
pub fn parse(v: &Value) -> Option<Event> {
    let s = |k: &str| v[k].as_str().map(str::to_string);
    match v["type"].as_str()? {
        "welcome" => Some(Event::Connected {
            agent: v["agent"]["name"].as_str().map(str::to_string),
        }),
        "message" | "response" => {
            let (id, text) = (
                s("id").unwrap_or_default(),
                s("content").unwrap_or_default(),
            );
            if v["isChunk"].as_bool().unwrap_or(false) {
                Some(Event::Chunk { id, text })
            } else if text.is_empty() {
                None // p. ej. el frame de imagen sin texto
            } else {
                Some(Event::Final { id, text })
            }
        }
        "process" if v["label"].as_str().is_some_and(|s| !s.is_empty()) => {
            Some(Event::Progress(ProcessStep {
                turn_id: s("messageId").unwrap_or_default(),
                kind: s("narrationKind")
                    .or_else(|| s("processKind"))
                    .unwrap_or_default(),
                timestamp: v["timestamp"].as_u64().unwrap_or(0),
                id: s("stepId")
                    .or_else(|| s("id"))
                    .or_else(|| s("messageId"))
                    .unwrap_or_default(),
                label: s("label").unwrap_or_default(),
                detail: s("detail"),
                agent: s("agentName"),
                status: s("processStatus").unwrap_or_else(|| "thinking".into()),
                finished: matches!(v["processStatus"].as_str(), Some("done" | "error"))
                    && v["turnDone"].as_bool().unwrap_or(true),
            }))
        }
        "process" => match v["processStatus"].as_str() {
            Some("done") | Some("error") => Some(Event::Done),
            _ => None,
        },
        "notification" => Some(Event::Notification {
            id: s("notificationId")?,
            content: s("content").unwrap_or_default(),
        }),
        "approval_request" => Some(Event::Approval {
            id: s("approvalId").filter(|id| !id.is_empty())?,
            tool: s("tool").filter(|tool| !tool.is_empty())?,
            arguments: v["arguments"].clone(),
            reason: s("reason").filter(|reason| !reason.trim().is_empty()),
            expires_in_ms: v["expiresInMs"]
                .as_u64()
                .unwrap_or(APPROVAL_DEFAULT_MS)
                .clamp(1_000, 3_600_000),
        }),
        "error" => Some(Event::Error(
            s("error").unwrap_or_else(|| "error desconocido".into()),
        )),
        t if t.starts_with("a2ui:") => Some(Event::A2ui {
            kind: t.to_string(),
            data: v["data"].clone(),
        }),
        // `canvas:ping` y `canvas:connected` son del enlace, no de la escena: los atiende `run`.
        t if t.starts_with("canvas:") && t != "canvas:ping" && t != "canvas:connected" => {
            Some(Event::Canvas {
                kind: t.to_string(),
                data: v["data"].clone(),
                timestamp: v["timestamp"].as_f64(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn webchat_filters_queued_messages_and_errors_when_switching_threads() {
        use super::*;
        let (out, _rx) = mpsc::channel();
        let events = Arc::new(Mutex::new(vec![
            QueuedEvent {
                thread_id: Some("a".into()),
                event: Event::Chunk {
                    id: "old".into(),
                    text: "old".into(),
                },
            },
            QueuedEvent {
                thread_id: Some("a".into()),
                event: Event::Error("old error".into()),
            },
            QueuedEvent {
                thread_id: Some("b".into()),
                event: Event::Final {
                    id: "new".into(),
                    text: "new".into(),
                },
            },
            QueuedEvent {
                thread_id: None,
                event: Event::Connected { agent: None },
            },
        ]));
        let mut chat = Chat {
            thread_id: "a".into(),
            user_id: "u".into(),
            token: None,
            out,
            events,
        };
        chat.select_thread("b".into());
        assert_eq!(
            chat.drain(),
            vec![
                Event::Final {
                    id: "new".into(),
                    text: "new".into()
                },
                Event::Connected { agent: None }
            ]
        );
        assert!(chat.drain().is_empty());
    }
    use super::*;

    #[test]
    fn chunk_and_final_are_told_apart() {
        let c = json!({"type":"message","id":"m1","content":"ho","isChunk":true});
        assert_eq!(
            parse(&c),
            Some(Event::Chunk {
                id: "m1".into(),
                text: "ho".into()
            })
        );
        let f = json!({"type":"message","id":"m1","content":"hola"});
        assert_eq!(
            parse(&f),
            Some(Event::Final {
                id: "m1".into(),
                text: "hola".into()
            })
        );
        assert_eq!(
            parse(&json!({"type":"message","id":"m2","content":""})),
            None
        );
    }

    #[test]
    fn notifications_need_an_id_and_carry_their_text() {
        let n = json!({"type":"notification","notificationId":"n1","content":"Reunión en 5 min"});
        assert_eq!(
            parse(&n),
            Some(Event::Notification {
                id: "n1".into(),
                content: "Reunión en 5 min".into()
            })
        );
        assert_eq!(
            parse(&json!({"type":"notification","content":"sin id"})),
            None,
            "sin id no se puede confirmar"
        );
    }

    #[test]
    fn an_approval_request_needs_an_id_and_a_tool_and_keeps_the_servers_deadline_within_its_limits()
    {
        let ask = json!({"type":"approval_request","approvalId":"a1","threadId":"t1","tool":"task_delegate","arguments":{"agent":"x"},"reason":"política","expiresInMs":30000});
        assert_eq!(
            parse(&ask),
            Some(Event::Approval {
                id: "a1".into(),
                tool: "task_delegate".into(),
                arguments: json!({"agent":"x"}),
                reason: Some("política".into()),
                expires_in_ms: 30_000
            })
        );
        assert_eq!(
            parse(&json!({"type":"approval_request","tool":"t"})),
            None,
            "sin id no se puede contestar"
        );
        assert_eq!(
            parse(&json!({"type":"approval_request","approvalId":"a"})),
            None
        );
        assert_eq!(
            parse(&json!({"type":"approval_request","approvalId":"","tool":"t"})),
            None
        );
        let bare =
            parse(&json!({"type":"approval_request","approvalId":"a","tool":"t","reason":"  "}))
                .unwrap();
        assert_eq!(
            bare,
            Event::Approval {
                id: "a".into(),
                tool: "t".into(),
                arguments: Value::Null,
                reason: None,
                expires_in_ms: 120_000
            }
        );
        for (sent, kept) in [(0u64, 1_000u64), (999_999_999, 3_600_000)] {
            match parse(
                &json!({"type":"approval_request","approvalId":"a","tool":"t","expiresInMs":sent}),
            ) {
                Some(Event::Approval { expires_in_ms, .. }) => assert_eq!(expires_in_ms, kept),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            approval_frame("a1", true),
            json!({"type":"approval","approvalId":"a1","approved":true})
        );
        assert_eq!(approval_frame("a1", false)["approved"], false);
    }

    #[test]
    fn an_approval_is_never_hidden_because_another_thread_is_on_screen() {
        use super::*;
        let (out, _rx) = mpsc::channel();
        let events = Arc::new(Mutex::new(vec![
            QueuedEvent {
                thread_id: None,
                event: Event::Approval {
                    id: "a".into(),
                    tool: "t".into(),
                    arguments: Value::Null,
                    reason: None,
                    expires_in_ms: 1000,
                },
            },
            QueuedEvent {
                thread_id: Some("otro".into()),
                event: Event::Chunk {
                    id: "c".into(),
                    text: "x".into(),
                },
            },
        ]));
        let chat = Chat {
            thread_id: "actual".into(),
            user_id: "u".into(),
            token: None,
            out,
            events,
        };
        let got = chat.drain();
        assert_eq!(
            got.len(),
            1,
            "el fragmento de otro hilo se descarta, la aprobación no"
        );
        assert!(matches!(got[0], Event::Approval { .. }));
    }

    /// De extremo a extremo con un servidor de WebSocket real: llega la petición (aunque traiga el hilo
    /// de otra conversación) y la respuesta sale con el id y el sentido que se eligieron.
    #[test]
    fn an_approval_request_arrives_and_the_answer_goes_back_by_id() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            ws.send(Message::text(json!({"type":"approval_request","approvalId":"ap-1","threadId":"otro","tool":"a2ui_createSurface","arguments":{"n":1},"reason":"ask","expiresInMs":5000}).to_string())).unwrap();
            loop {
                if let Ok(Message::Text(t)) = ws.read() {
                    let frame = serde_json::from_str::<Value>(t.as_str()).unwrap();
                    if frame["type"] == "approval" {
                        return frame;
                    }
                }
            }
        });
        let chat = Chat::connect(port, "u1", None, "actual".into()).unwrap();
        let mut seen = None;
        for _ in 0..100 {
            if let Some(event) = chat
                .drain()
                .into_iter()
                .find(|event| matches!(event, Event::Approval { .. }))
            {
                seen = Some(event);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let Some(Event::Approval {
            id,
            tool,
            expires_in_ms,
            ..
        }) = seen
        else {
            panic!("no llegó la petición")
        };
        assert_eq!(
            (id.as_str(), tool.as_str(), expires_in_ms),
            ("ap-1", "a2ui_createSurface", 5000)
        );
        chat.send_raw(&approval_frame(&id, false)); // el mismo camino que usa la app: el `outbox` de `poll`
        assert_eq!(
            server.join().unwrap(),
            json!({"type":"approval","approvalId":"ap-1","approved":false})
        );
    }

    #[test]
    fn turn_ends_on_process_done_or_error() {
        assert_eq!(
            parse(&json!({"type":"process","processStatus":"done"})),
            Some(Event::Done)
        );
        assert_eq!(
            parse(&json!({"type":"process","processStatus":"error"})),
            Some(Event::Done)
        );
        assert_eq!(
            parse(&json!({"type":"process","processStatus":"thinking"})),
            None
        );
    }

    #[test]
    fn welcome_and_error_frames() {
        let w = json!({"type":"welcome","agent":{"name":"Bee"}});
        assert_eq!(
            parse(&w),
            Some(Event::Connected {
                agent: Some("Bee".into())
            })
        );
        assert_eq!(
            parse(&json!({"type":"error","error":"x"})),
            Some(Event::Error("x".into()))
        );
        assert_eq!(parse(&json!({"type":"status"})), None);
        assert_eq!(
            parse(&json!({"type":"a2ui:createSurface","data":{"surfaceId":"s1"}})),
            Some(Event::A2ui {
                kind: "a2ui:createSurface".into(),
                data: json!({"surfaceId":"s1"})
            })
        );
    }

    /// El servidor manda un aviso; el cliente lo recibe y lo confirma con su id para que no se repita.
    #[test]
    fn a_notification_arrives_and_is_acknowledged_by_id() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            ws.send(Message::text(json!({"type":"notification","notificationId":"n7","content":"Tu informe está listo"}).to_string())).unwrap();
            let mut got = Vec::new();
            while got.len() < 3 {
                if let Ok(Message::Text(t)) = ws.read() {
                    got.push(serde_json::from_str::<Value>(t.as_str()).unwrap());
                }
            }
            got
        });
        let chat = Chat::connect(port, "u1", None, "u1/webchat/c1".into()).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = None;
        while seen.is_none() && std::time::Instant::now() < end {
            thread::sleep(Duration::from_millis(20));
            seen = chat.drain().into_iter().find_map(|e| {
                if let Event::Notification { id, content } = e {
                    Some((id, content))
                } else {
                    None
                }
            });
        }
        let (id, content) = seen.expect("llegó el aviso");
        assert_eq!(
            (id.as_str(), content.as_str()),
            ("n7", "Tu informe está listo")
        );
        chat.ack(&id);
        let got = server.join().unwrap();
        assert_eq!(
            got[1],
            json!({"type": "notification_sync", "sessionId": "u1"})
        );
        assert_eq!(
            got[2],
            json!({"type": "notification_ack", "notificationId": "n7"})
        );
    }

    #[test]
    fn url_encoding_escapes_reserved() {
        assert_eq!(urlenc("a/b c"), "a%2Fb%20c");
    }

    /// Cableado completo con un gateway falso: suscripción al canvas, superficies que llegan,
    /// y una acción del usuario que vuelve con la forma exacta del protocolo.
    #[test]
    fn surfaces_arrive_and_actions_go_back_over_the_socket() {
        use crate::a2ui::{model::action_frame, view::A2State};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let mut got = Vec::new();
            ws.send(Message::text(
                json!({"type":"a2ui:createSurface","data":{"surfaceId":"s1"}}).to_string(),
            ))
            .unwrap();
            ws.send(Message::text(json!({"type":"a2ui:updateComponents","data":{"surfaceId":"s1","components":[{"id":"root","component":"Text","text":"hola"}]}}).to_string())).unwrap();
            // Recibe lo que manda el cliente: primero la suscripción, luego la acción.
            while got.len() < 3 {
                if let Ok(Message::Text(t)) = ws.read() {
                    got.push(serde_json::from_str::<Value>(t.as_str()).unwrap());
                }
            }
            got
        });
        let chat = Chat::connect(port, "u1", None, "u1/webchat/c1".into()).unwrap();
        let mut a2 = A2State::default();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while a2.store.get("s1").and_then(|s| s.root.as_ref()).is_none()
            && std::time::Instant::now() < end
        {
            thread::sleep(Duration::from_millis(20));
            for ev in chat.drain() {
                if let Event::A2ui { kind, data } = ev {
                    a2.apply(&kind, &data);
                }
            }
        }
        assert_eq!(
            a2.store.get("s1").unwrap().root.as_deref(),
            Some("root"),
            "la superficie llegó y se armó"
        );
        chat.send_raw(&action_frame("submit", "s1", "btn", json!({"k": 1})));
        let got = server.join().unwrap();
        assert_eq!(got[0], json!({"type": "canvas_subscribe"}));
        assert_eq!(
            got[1],
            json!({"type": "notification_sync", "sessionId": "u1"}),
            "al conectar pide los avisos pendientes"
        );
        assert_eq!(got[2]["type"], "a2ui:action");
        assert_eq!(got[2]["data"]["name"], "submit");
        assert_eq!(got[2]["data"]["surfaceId"], "s1");
        assert_eq!(got[2]["data"]["context"], json!({"k": 1}));
    }

    // ── acceso con correo y contraseña, contra un gateway HTTP falso ──

    use crate::testutil::fake_http;

    #[test]
    fn login_returns_the_token_and_sends_the_credentials() {
        let (port, server) = fake_http(
            |_, _, _, body| {
                let b: Value = serde_json::from_str(body).unwrap();
                if b["email"] == "a@b.co" && b["password"] == "secreto" {
                    (200, json!({"authToken": "tok-123"}))
                } else {
                    (401, json!({"error": "bad"}))
                }
            },
            2,
        );
        let base = format!("http://127.0.0.1:{port}");
        assert_eq!(login(&base, "a@b.co", "secreto"), Ok("tok-123".to_string()));
        assert_eq!(
            login(&base, "a@b.co", "mal"),
            Err("Correo o contraseña incorrectos".to_string())
        );
        let seen = server.join().unwrap();
        assert!(seen
            .iter()
            .all(|(m, p, _, _)| m == "POST" && p == "/api/auth/login"));
    }

    #[test]
    fn prepare_asks_for_login_when_credentials_are_required() {
        let (port, server) = fake_http(
            |_, path, _, _| match path {
                "/api/setup/status" => (200, json!({"configured": true})),
                "/api/auth/status" => (200, json!({"hasCredentials": true})),
                _ => (500, json!({})),
            },
            2,
        );
        assert_eq!(
            prepare(&format!("http://127.0.0.1:{port}"), None),
            Err(BootError::NeedsLogin)
        );
        server.join().unwrap();
    }

    #[test]
    fn prepare_uses_the_token_on_every_protected_call() {
        let (port, server) = fake_http(
            |method, path, auth, _| match (method, path) {
                (_, "/api/setup/status") => (200, json!({"configured": true})),
                (_, "/api/auth/status") => (200, json!({"hasCredentials": true})),
                (_, "/api/users") if auth == Some("Bearer tok") => (200, json!([{"id": "u1"}])),
                ("GET", "/api/conversations?channel=webchat") if auth == Some("Bearer tok") => {
                    (200, json!({"conversations":[]}))
                }
                ("POST", "/api/conversations") if auth == Some("Bearer tok") => {
                    (201, json!({"conversation": {"id": "u1/webchat/c9"}}))
                }
                _ => (401, json!({"error": "no"})),
            },
            5,
        );
        let got = prepare(&format!("http://127.0.0.1:{port}"), Some("tok")).unwrap();
        assert_eq!(
            got,
            Prepared {
                user_id: "u1".into(),
                thread_id: "u1/webchat/c9".into(),
                token: Some("tok".into())
            }
        );
        let seen = server.join().unwrap();
        let protected: Vec<_> = seen
            .iter()
            .filter(|(_, p, _, _)| p == "/api/users" || p.starts_with("/api/conversations"))
            .collect();
        assert!(protected
            .iter()
            .all(|(_, _, a, _)| a.as_deref() == Some("Bearer tok")));
    }

    #[test]
    fn a_rejected_token_means_login_again() {
        let (port, server) = fake_http(
            |_, path, _, _| match path {
                "/api/setup/status" => (200, json!({"configured": true})),
                "/api/auth/status" => (200, json!({"hasCredentials": true})),
                _ => (401, json!({"error": "no"})),
            },
            3,
        );
        assert_eq!(
            prepare(&format!("http://127.0.0.1:{port}"), Some("viejo")),
            Err(BootError::NeedsLogin)
        );
        server.join().unwrap();
    }

    #[test]
    fn an_unconfigured_gateway_points_to_the_setup_wizard() {
        let (port, server) = fake_http(|_, _, _, _| (200, json!({"configured": false})), 1);
        match prepare(&format!("http://127.0.0.1:{port}"), None) {
            Err(BootError::Msg(m)) => assert!(m.contains("/setup"), "{m}"),
            other => panic!("{other:?}"),
        }
        server.join().unwrap();
    }

    #[test]
    fn canvas_frames_become_canvas_events_but_link_frames_do_not() {
        let e = parse(
            &json!({"type": "canvas:work_event", "timestamp": 1700.0, "data": {"eventId": "e1"}}),
        )
        .unwrap();
        assert_eq!(
            e,
            Event::Canvas {
                kind: "canvas:work_event".into(),
                data: json!({"eventId": "e1"}),
                timestamp: Some(1700.0)
            }
        );
        let e = parse(&json!({"type": "canvas:snapshot", "data": {"nodes": []}})).unwrap();
        assert!(matches!(
            e,
            Event::Canvas {
                timestamp: None,
                ..
            }
        ));
        assert_eq!(parse(&json!({"type": "canvas:ping", "connId": "c"})), None);
        assert_eq!(
            parse(&json!({"type": "canvas:connected", "connId": "c"})),
            None
        );
    }

    /// El servidor comprueba el enlace con `canvas:ping`: hay que responder con el `connId` que dio al conectar.
    #[test]
    fn the_client_answers_canvas_pings_with_its_connection_id() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            ws.send(Message::text(
                json!({"type": "canvas:connected", "data": {"connId": "conn-42"}}).to_string(),
            ))
            .unwrap();
            ws.send(Message::text(json!({"type": "canvas:ping"}).to_string()))
                .unwrap();
            let mut got = Vec::new();
            while got.len() < 3 {
                if let Ok(Message::Text(t)) = ws.read() {
                    got.push(serde_json::from_str::<Value>(t.as_str()).unwrap());
                }
            }
            got
        });
        let _chat = Chat::connect(port, "u1", None, "u1/webchat/c1".into()).unwrap();
        let got = server.join().unwrap();
        assert_eq!(got[0], json!({"type": "canvas_subscribe"}));
        assert_eq!(
            got[1],
            json!({"type": "notification_sync", "sessionId": "u1"})
        );
        assert_eq!(got[2], json!({"type": "canvas:pong", "connId": "conn-42"}));
    }
}
