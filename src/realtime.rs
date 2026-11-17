//! Sesión de voz en tiempo real (`/realtime`): micrófono -> gateway -> Gemini Live -> altavoz.
//! Protocolo de `core/src/gateway/realtime/{protocol,index}.ts` y `hive-ui/src/stores/realtimeStore.ts`.
//!
//! No hay frame de arranque: la sesión empieza al abrir el socket y el servidor avisa con `ready`.
//! El micrófono va como binario PCM16 mono a 16 kHz, la voz del modelo vuelve como binario PCM16
//! mono a 24 kHz. Parar es cerrar el socket.

use std::{
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

use crate::voice::Voice;

#[derive(Clone, Debug, PartialEq)]
pub enum REvent {
    Pricing {
        input: Option<f64>,
        output: Option<f64>,
    },
    Ready {
        model: String,
    },
    /// Fragmento incremental; no hay marca de parcial/final, el turno cierra con `TurnComplete`.
    Transcript {
        assistant: bool,
        text: String,
    },
    Interrupted,
    TurnComplete,
    /// El modelo llamó a una herramienta puente (p. ej. `consultar_a_bee`).
    Tool(String),
    /// Consumo acumulado de la sesión (tokens), que alimenta el contador de costo.
    Usage {
        input: u64,
        output: u64,
    },
    /// Cómo suena un trozo de la voz del modelo (nivel y timbre), para mover la boca de BIA con el audio real.
    /// `delay_ms` es lo que el audio espera en la cola del altavoz antes de sonar.
    Mouth {
        level: f32,
        bright: f32,
        delay_ms: u32,
    },
    /// Un hito de la colmena (delegación, herramienta, verificación…).
    Narration {
        kind: String,
        label: String,
        detail: Option<String>,
        agent: String,
    },
    Error(String),
    Closed,
}

enum Out {
    Pcm(Vec<u8>),
    Json(String),
}

pub struct Realtime {
    out: SyncSender<Out>,
    events: Arc<Mutex<Vec<REvent>>>,
    voice: Arc<Mutex<Voice>>,
    stop: Arc<AtomicBool>,
}

fn video_frame(jpeg: &[u8], source: Option<&str>) -> String {
    use base64::Engine;
    let mut v = json!({"type": "video", "data": base64::engine::general_purpose::STANDARD.encode(jpeg), "mimeType": "image/jpeg"});
    if let Some(src) = source {
        v["source"] = json!(src);
    }
    v.to_string()
}

pub fn url(
    port: u16,
    session: &str,
    token: Option<&str>,
    voice: &str,
    lang: &str,
    speaker: bool,
) -> String {
    let mut u = format!(
        "ws://127.0.0.1:{port}/realtime?session={}&voice={}&lang={}",
        crate::chat::urlenc(session),
        crate::chat::urlenc(voice),
        crate::chat::urlenc(lang)
    );
    if let Some(t) = token {
        u.push_str(&format!("&token={}", crate::chat::urlenc(t)));
    }
    if speaker {
        u.push_str("&altavoz=1");
    }
    u
}

impl Realtime {
    /// Abre la sesión. El audio empieza a fluir solo cuando llega `ready`.
    pub fn start(
        port: u16,
        session: &str,
        token: Option<&str>,
        speaker: bool,
    ) -> Result<Realtime, String> {
        Self::start_with(port, session, token, "Kore", "es-CO", speaker)
    }

    /// Como `start`, con la voz y el idioma elegidos (las de la lista de la web).
    pub fn start_with(
        port: u16,
        session: &str,
        token: Option<&str>,
        voice: &str,
        lang: &str,
        speaker: bool,
    ) -> Result<Realtime, String> {
        Self::open(url(port, session, token, voice, lang, speaker))
    }
    pub fn start_in_thread(
        port: u16,
        session: &str,
        token: Option<&str>,
        voice: &str,
        lang: &str,
        speaker: bool,
        thread: &str,
    ) -> Result<Realtime, String> {
        Self::open(format!(
            "{}&threadId={}",
            url(port, session, token, voice, lang, speaker),
            crate::chat::urlenc(thread)
        ))
    }
    fn open(url: String) -> Result<Realtime, String> {
        let (mut socket, _) =
            tungstenite::connect(url).map_err(|e| format!("No se pudo abrir la voz: {e}"))?;
        if let MaybeTlsStream::Plain(s) = socket.get_mut() {
            let _ = s.set_read_timeout(Some(Duration::from_millis(20)));
            let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
        }
        let (out, rx) = mpsc::sync_channel::<Out>(128);
        let events = Arc::new(Mutex::new(Vec::new()));
        let voice = Arc::new(Mutex::new(Voice::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, ev, vc, st) = (out.clone(), events.clone(), voice.clone(), stop.clone());
        thread::spawn(move || run(socket, rx, tx, ev, vc, st));
        Ok(Realtime {
            out,
            events,
            voice,
            stop,
        })
    }

    /// Cambia el micrófono en plena llamada (`None` = el del sistema). Si el nuevo no abre, se vuelve
    /// al del sistema para no dejar la llamada sorda, y se explica por qué.
    pub fn switch_mic(&self, device: Option<&str>) -> Result<(), String> {
        let open = |device: Option<&str>| {
            let tx = self.out.clone();
            self.voice
                .lock()
                .unwrap()
                .start_mic(device, move |pcm| drop(tx.try_send(Out::Pcm(pcm))))
        };
        open(device).map_err(|e| {
            let _ = open(None);
            e.message
        })
    }

    /// Cambia la salida en plena llamada. Lo que estaba sonando se corta (suena por otro aparato).
    pub fn switch_output(&self, device: Option<&str>) -> Result<(), String> {
        let mut voice = self.voice.lock().unwrap();
        voice.start_player(device).map_err(|e| {
            let _ = voice.start_player(None);
            e.message
        })
    }

    /// Texto escrito dentro de la sesión de voz.
    pub fn text(&self, content: &str) {
        if self
            .out
            .try_send(Out::Json(
                json!({"type": "text", "content": content}).to_string(),
            ))
            .is_err()
        {
            self.events.lock().unwrap().push(REvent::Error(
                "La conexión de voz está ocupada; no se envió el texto".into(),
            ));
        }
    }

    /// Fotogramas JPEG para el modelo, con el mismo formato que la web:
    /// `{"type":"video","data":base64,"mimeType":"image/jpeg"[,"source":"screen"]}`.
    /// Función para el hilo de captura: manda solo los fotogramas pensados para el modelo (~1 por segundo);
    /// la vista previa nunca sale de la aplicación.
    pub fn video_sink(
        &self,
        source: Option<&'static str>,
    ) -> impl FnMut(hive_media::VideoFrame) + Send + 'static {
        let out = self.out.clone();
        move |f| {
            if f.kind == hive_media::FrameKind::Model {
                let _ = out.try_send(Out::Json(video_frame(&f.jpeg, source)));
            }
        }
    }

    /// Barge-in manual: calla lo que suena y corta la generación del modelo.
    pub fn interrupt(&self) {
        self.voice.lock().unwrap().interrupt();
        if self
            .out
            .try_send(Out::Json(json!({"type": "interrupt"}).to_string()))
            .is_err()
        {
            self.events.lock().unwrap().push(REvent::Error(
                "No se pudo enviar la interrupción; puedes colgar la llamada".into(),
            ));
        }
    }

    pub fn drain(&self) -> Vec<REvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    /// Cierra el socket y libera micrófono y altavoz.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Realtime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    mut socket: WebSocket<MaybeTlsStream<TcpStream>>,
    outgoing: Receiver<Out>,
    mic_tx: SyncSender<Out>,
    events: Arc<Mutex<Vec<REvent>>>,
    voice: Arc<Mutex<Voice>>,
    stop: Arc<AtomicBool>,
) {
    let push = |e: REvent| events.lock().unwrap().push(e);
    let mut last_ping = Instant::now();
    let mut ready = false;
    'main: loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        for _ in 0..8 {
            let Ok(o) = outgoing.try_recv() else { break };
            let msg = match o {
                Out::Pcm(b) => Message::Binary(b.into()),
                Out::Json(s) => Message::text(s),
            };
            if socket.send(msg).is_err() {
                push(REvent::Error("La conexión de voz dejó de responder".into()));
                break 'main;
            }
        }
        if last_ping.elapsed() > Duration::from_secs(20) {
            let _ = socket.send(Message::text(json!({"type": "ping"}).to_string()));
            last_ping = Instant::now();
        }
        match socket.read() {
            // Voz del modelo: PCM16 mono a 24 kHz, directo al altavoz.
            Ok(Message::Binary(b)) => {
                // Cuánto audio hay ya en cola: este trozo sonará después, y la boca debe esperarlo.
                let delay_ms = {
                    let vc = voice.lock().unwrap();
                    let queued = vc.queued_ms();
                    vc.enqueue(&b);
                    queued.min(5_000) as u32
                };
                let sound = crate::live::lipsync::analyze(&b);
                push(REvent::Mouth {
                    level: sound.level,
                    bright: sound.bright,
                    delay_ms,
                });
            }
            Ok(Message::Text(t)) => {
                let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else {
                    continue;
                };
                match parse(&v) {
                    Some(REvent::Ready { model }) => {
                        if ready {
                            continue;
                        }
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        // Audio enviado antes de `ready` se pierde: recién ahora se abre el micrófono.
                        let mut vc = voice.lock().unwrap();
                        let tx = mic_tx.clone();
                        // El micrófono y la salida que se eligieron en «Pantalla y audio» (o los del sistema).
                        let prefs = crate::audio_prefs::load();
                        let opened = vc.start_player(prefs.output.as_deref()).and_then(|_| {
                            vc.start_mic(prefs.input.as_deref(), move |pcm| {
                                drop(tx.try_send(Out::Pcm(pcm)))
                            })
                        });
                        drop(vc);
                        match opened {
                            Ok(()) => {
                                ready = true;
                                push(REvent::Ready { model });
                            }
                            Err(e) => {
                                push(REvent::Error(format!(
                                    "No se pudo abrir el audio: {}",
                                    e.message
                                )));
                                break;
                            }
                        }
                    }
                    Some(REvent::Interrupted) => {
                        voice.lock().unwrap().interrupt();
                        push(REvent::Interrupted);
                    }
                    Some(REvent::Closed) => break,
                    Some(e) => push(e),
                    None => {}
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                push(REvent::Error("La conexión de voz se interrumpió".into()));
                break;
            }
        }
    }
    {
        let mut vc = voice.lock().unwrap();
        vc.stop_mic();
        vc.stop_player();
    }
    let _ = socket.close(None);
    push(REvent::Closed);
}

/// Traduce un frame de texto del servidor. `pong` se ignora.
pub fn parse(v: &Value) -> Option<REvent> {
    let s = |k: &str| v[k].as_str().map(str::to_string);
    match v["type"].as_str()? {
        "pricing" => Some(REvent::Pricing {
            input: v["inputPer1M"].as_f64().filter(|price| *price >= 0.0),
            output: v["outputPer1M"].as_f64().filter(|price| *price >= 0.0),
        }),
        "ready" => Some(REvent::Ready {
            model: s("model").unwrap_or_default(),
        }),
        "transcript" => Some(REvent::Transcript {
            assistant: v["role"] == "assistant",
            text: s("text").unwrap_or_default(),
        }),
        "interrupted" => Some(REvent::Interrupted),
        "turn_complete" => Some(REvent::TurnComplete),
        "tool" => Some(REvent::Tool(s("name").unwrap_or_default())),
        "usage" => Some(REvent::Usage {
            input: v["inputTokens"].as_u64().unwrap_or(0),
            output: v["outputTokens"].as_u64().unwrap_or(0),
        }),
        "narration" => Some(REvent::Narration {
            kind: s("kind").unwrap_or_default(),
            label: s("label").unwrap_or_default(),
            detail: s("detail"),
            agent: s("agent").unwrap_or_default(),
        }),
        "error" => Some(REvent::Error(
            s("error").unwrap_or_else(|| "error desconocido".into()),
        )),
        "closed" => Some(REvent::Closed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_frames() {
        assert_eq!(
            parse(&json!({"type":"ready","model":"m","voice":"Kore"})),
            Some(REvent::Ready { model: "m".into() })
        );
        assert_eq!(
            parse(&json!({"type":"transcript","role":"assistant","text":"hola"})),
            Some(REvent::Transcript {
                assistant: true,
                text: "hola".into()
            })
        );
        assert_eq!(
            parse(&json!({"type":"transcript","role":"user","text":"x"})),
            Some(REvent::Transcript {
                assistant: false,
                text: "x".into()
            })
        );
        assert_eq!(
            parse(&json!({"type":"interrupted"})),
            Some(REvent::Interrupted)
        );
        assert_eq!(
            parse(&json!({"type":"turn_complete"})),
            Some(REvent::TurnComplete)
        );
        assert_eq!(
            parse(&json!({"type":"tool","name":"consultar_a_bee","ok":true})),
            Some(REvent::Tool("consultar_a_bee".into()))
        );
        assert_eq!(
            parse(&json!({"type":"error","error":"x"})),
            Some(REvent::Error("x".into()))
        );
        assert_eq!(
            parse(&json!({"type":"closed","reason":"inactividad"})),
            Some(REvent::Closed)
        );
        assert_eq!(
            parse(&json!({"type":"usage","inputTokens":120,"outputTokens":30,"totalTokens":150})),
            Some(REvent::Usage {
                input: 120,
                output: 30
            })
        );
        assert_eq!(
            parse(
                &json!({"type":"narration","kind":"delegated","status":"x","label":"Buscar","detail":null,"agent":"web","at":1})
            ),
            Some(REvent::Narration {
                kind: "delegated".into(),
                label: "Buscar".into(),
                detail: None,
                agent: "web".into()
            })
        );
        assert_eq!(parse(&json!({"type":"pong"})), None);
    }

    #[test]
    fn url_carries_session_voice_and_flags() {
        let u = url(18790, "u/1", Some("t k"), "Kore", "es-CO", true);
        assert!(u.starts_with("ws://127.0.0.1:18790/realtime?session=u%2F1&voice=Kore&lang=es-CO"));
        assert!(u.contains("&token=t%20k") && u.ends_with("&altavoz=1"));
        assert!(!url(1, "a", None, "Kore", "es-CO", false).contains("token"));
    }

    /// Ruta feliz contra un servidor falso que habla el protocolo. Abre el micrófono y el altavoz
    /// reales, así que no corre por defecto: `cargo test -- --ignored realtime_happy_path`.
    #[test]
    #[ignore]
    fn realtime_happy_path_against_a_fake_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            ws.send(Message::text(
                json!({"type":"ready","model":"fake","voice":"Kore"}).to_string(),
            ))
            .unwrap();
            ws.send(Message::text(
                json!({"type":"transcript","role":"user","text":"hola "}).to_string(),
            ))
            .unwrap();
            // Silencio de 200 ms a 24 kHz: ejercita el altavoz sin ruido.
            ws.send(Message::Binary(vec![0u8; 9600].into())).unwrap();
            ws.send(Message::text(json!({"type":"turn_complete"}).to_string()))
                .unwrap();
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let (mut frames, mut sizes_ok) = (0, true);
            let end = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < end {
                if let Ok(Message::Binary(b)) = ws.read() {
                    frames += 1;
                    sizes_ok &= b.len() == 1280;
                }
            }
            (frames, sizes_ok)
        });
        let rt = Realtime::start(port, "t", None, false).unwrap();
        let mut events = Vec::new();
        let end = Instant::now() + Duration::from_millis(1800);
        while Instant::now() < end {
            thread::sleep(Duration::from_millis(50));
            events.extend(rt.drain());
        }
        rt.stop();
        thread::sleep(Duration::from_millis(300));
        events.extend(rt.drain());
        let (frames, sizes_ok) = server.join().unwrap();
        assert!(
            events.contains(&REvent::Ready {
                model: "fake".into()
            }),
            "{events:?}"
        );
        assert!(events.contains(&REvent::Transcript {
            assistant: false,
            text: "hola ".into()
        }));
        assert!(events.contains(&REvent::TurnComplete));
        assert_eq!(events.last(), Some(&REvent::Closed));
        assert!(
            frames >= 10,
            "el micrófono debería mandar ~25 bloques en 1,5 s, mandó {frames}"
        );
        assert!(
            sizes_ok,
            "cada bloque debe ser de 1280 bytes (40 ms a 16 kHz PCM16)"
        );
    }

    #[test]
    fn video_frames_go_out_in_the_webs_format_and_previews_never_leave() {
        use base64::Engine;
        use hive_media::{FrameKind, VideoFrame};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            // Sin `ready` no se abre ni el micrófono ni el altavoz: solo viajan los textos.
            let mut got: Vec<Value> = Vec::new();
            while got.len() < 2 {
                if let Ok(Message::Text(t)) = ws.read() {
                    got.push(serde_json::from_str(t.as_str()).unwrap());
                }
            }
            got
        });
        let rt = Realtime::start(port, "t", None, false).unwrap();
        let (mut camera, mut screen) = (rt.video_sink(None), rt.video_sink(Some("screen")));
        camera(VideoFrame {
            kind: FrameKind::Preview,
            jpeg: vec![9, 9, 9],
        }); // la vista previa no sale
        camera(VideoFrame {
            kind: FrameKind::Model,
            jpeg: vec![0xff, 0xd8, 0xff, 0xd9],
        });
        screen(VideoFrame {
            kind: FrameKind::Preview,
            jpeg: vec![1],
        });
        screen(VideoFrame {
            kind: FrameKind::Model,
            jpeg: vec![0xff, 0xd8, 0x01, 0xff, 0xd9],
        });
        let got = server.join().unwrap();
        rt.stop();
        assert_eq!(got[0]["type"], "video");
        assert_eq!(got[0]["mimeType"], "image/jpeg");
        assert!(
            got[0].get("source").is_none(),
            "la cámara no lleva `source`, como en la web"
        );
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(got[0]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            bytes,
            [0xff, 0xd8, 0xff, 0xd9],
            "el JPEG viaja íntegro en base64"
        );
        assert_eq!(got[1]["source"], "screen");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(got[1]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(bytes, [0xff, 0xd8, 0x01, 0xff, 0xd9]);
    }

    #[test]
    fn the_models_audio_also_reports_how_it_sounds_so_the_mouth_can_follow_it() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            // Un tono de 220 Hz y un silencio, como PCM16 a 24 kHz; sin `ready` no se abre ningún altavoz.
            let tone: Vec<u8> = (0..960)
                .flat_map(|i| {
                    (((i as f32 / 24_000.0 * 220.0 * std::f32::consts::TAU).sin() * 0.3 * 32767.0)
                        as i16)
                        .to_le_bytes()
                })
                .collect();
            ws.send(Message::Binary(tone.into())).unwrap();
            ws.send(Message::Binary(vec![0u8; 960].into())).unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let rt = Realtime::start(port, "t", None, false).unwrap();
        let end = Instant::now() + Duration::from_secs(3);
        let mut mouths = Vec::new();
        while Instant::now() < end && mouths.len() < 2 {
            thread::sleep(Duration::from_millis(20));
            mouths.extend(rt.drain().into_iter().filter_map(|e| {
                if let REvent::Mouth { level, bright, .. } = e {
                    Some((level, bright))
                } else {
                    None
                }
            }));
        }
        rt.stop();
        server.join().unwrap();
        assert_eq!(mouths.len(), 2, "un evento por trozo de audio: {mouths:?}");
        assert!(
            mouths[0].0 > 0.3 && mouths[0].1 < 0.1,
            "el tono es fuerte y grave: {:?}",
            mouths[0]
        );
        assert_eq!(mouths[1], (0.0, 0.0), "el silencio no mueve la boca");
    }
}
