//! Reuniones: transcripción en vivo con micrófono y, si se quiere, el audio del equipo.
//!
//! Misma ruta que la web: `POST /api/meetings` crea la sesión, el audio viaja por `/meeting-stream`
//! en trozos (`audio_chunk`, base64) y el servidor contesta con `transcript_segment`. La web manda
//! `audio/webm` grabado por el navegador; aquí se manda `audio/wav` (PCM16 mono a 16 kHz) que el
//! servicio de voz también acepta. Los trozos en silencio no se mandan: no hay nada que transcribir y
//! cada trozo cuesta una llamada al modelo de voz.

use std::{
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use base64::Engine;
use serde_json::{json, Value};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

use crate::settings::api::{request, ApiError, Conn};

pub const RATE: u32 = 16_000;
/// Duración de cada trozo que se manda a transcribir (la web usa 5 s).
pub const CHUNK_SECS: f32 = 5.0;
/// Por debajo de este nivel (RMS normalizado, ≈ −50 dBFS) un trozo se considera silencio.
const SILENCE_RMS: f32 = 0.003;

// ─── audio puro ──────────────────────────────────────────────────────────────

pub fn samples(pcm: &[u8]) -> Vec<i16> {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

/// Suma dos pistas con saturación; la más corta se completa con silencio.
pub fn mix(a: &[i16], b: &[i16]) -> Vec<i16> {
    (0..a.len().max(b.len()))
        .map(|i| {
            let s = a.get(i).copied().unwrap_or(0) as i32 + b.get(i).copied().unwrap_or(0) as i32;
            s.clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

pub fn rms(s: &[i16]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let sum: f64 = s.iter().map(|&x| (x as f64 / 32768.0).powi(2)).sum();
    (sum / s.len() as f64).sqrt() as f32
}

pub fn is_silent(s: &[i16]) -> bool {
    rms(s) < SILENCE_RMS
}

/// WAV PCM16 mono: cabecera de 44 bytes y las muestras en little-endian.
pub fn wav(s: &[i16], rate: u32) -> Vec<u8> {
    let data = (s.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2).to_le_bytes()); // bytes por segundo
    out.extend(2u16.to_le_bytes()); // alineación
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for x in s {
        out.extend(x.to_le_bytes());
    }
    out
}

/// Acumula el micrófono y el audio del equipo por separado y entrega trozos mezclados.
#[derive(Default)]
pub struct Chunker {
    mic: Vec<i16>,
    system: Vec<i16>,
}

impl Chunker {
    pub fn push_mic(&mut self, pcm: &[u8]) {
        self.mic.extend(samples(pcm));
    }

    pub fn push_system(&mut self, pcm: &[u8]) {
        self.system.extend(samples(pcm));
    }

    /// Un trozo de `n` muestras cuando alguna de las dos pistas lo completa; la otra aporta lo que tenga.
    pub fn take(&mut self, n: usize) -> Option<Vec<i16>> {
        if self.mic.len() < n && self.system.len() < n {
            return None;
        }
        let (a, b) = (self.mic.len().min(n), self.system.len().min(n));
        let chunk = mix(&self.mic[..a], &self.system[..b]);
        self.mic.drain(..a);
        self.system.drain(..b);
        Some(chunk)
    }

    /// Lo que queda al parar, si dura al menos `min` muestras.
    pub fn flush(&mut self, min: usize) -> Option<Vec<i16>> {
        let chunk = mix(&self.mic, &self.system);
        self.mic.clear();
        self.system.clear();
        (chunk.len() >= min).then_some(chunk)
    }
}

// ─── REST ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// `active`, `stopped` o `report_ready`.
    pub status: String,
    pub segments: u64,
    pub report_path: Option<String>,
}

fn ok(v: Value) -> Result<Value, ApiError> {
    if v["ok"].as_bool().unwrap_or(true) {
        Ok(v)
    } else {
        Err(ApiError::Status(
            200,
            v["error"]
                .as_str()
                .unwrap_or("el gateway rechazó la petición")
                .to_string(),
        ))
    }
}

pub fn create(conn: &Conn, title: &str) -> Result<String, ApiError> {
    let v = ok(request(
        conn,
        "POST",
        "/api/meetings",
        Some(json!({"title": title})),
    )?)?;
    v["session"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::Net("respuesta sin id de sesión".into()))
}

pub fn list(conn: &Conn) -> Result<Vec<Session>, ApiError> {
    let v = ok(request(conn, "GET", "/api/meetings", None)?)?;
    Ok(v["sessions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(Session {
                        id: s["id"].as_str()?.to_string(),
                        title: s["title"].as_str().unwrap_or("").to_string(),
                        status: s["status"].as_str().unwrap_or("").to_string(),
                        segments: s["segment_count"].as_u64().unwrap_or(0),
                        report_path: s["report_path"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn stop(conn: &Conn, id: &str) -> Result<(), ApiError> {
    ok(request(
        conn,
        "POST",
        &format!("/api/meetings/{}/stop", crate::chat::urlenc(id)),
        Some(json!({})),
    )?)
    .map(drop)
}

/// Pide el informe gerencial (una llamada al modelo de lenguaje) y devuelve la ruta del `.docx`.
pub fn report(conn: &Conn, id: &str) -> Result<String, ApiError> {
    let v = ok(request(
        conn,
        "POST",
        &format!("/api/meetings/{}/report", crate::chat::urlenc(id)),
        Some(json!({})),
    )?)?;
    v["report_path"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::Net("respuesta sin ruta del informe".into()))
}

// ─── sesión en vivo ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum MEvent {
    Segment {
        seq: u64,
        speaker: Option<String>,
        text: String,
    },
    Stopped {
        segments: u64,
    },
    Error(String),
    Closed,
}

pub struct Meeting {
    chunker: Arc<Mutex<Chunker>>,
    out: Sender<String>,
    events: Arc<Mutex<Vec<MEvent>>>,
    stop: Arc<AtomicBool>,
}

pub fn url(port: u16, meeting_id: &str) -> String {
    format!(
        "ws://127.0.0.1:{port}/meeting-stream?meetingSessionId={}",
        crate::chat::urlenc(meeting_id)
    )
}

fn chunk_frame(s: &[i16]) -> String {
    json!({"type": "audio_chunk", "audio": base64::engine::general_purpose::STANDARD.encode(wav(s, RATE)), "mime_type": "audio/wav"}).to_string()
}

impl Meeting {
    /// Abre el socket de la sesión `meeting_id` y empieza a trocear lo que le lleguen por `mic_sink` y
    /// `system_sink`. `chunk_secs` es la duración de cada trozo (5 s en uso real).
    #[cfg(test)]
    pub fn start(port: u16, meeting_id: &str, chunk_secs: f32) -> Result<Meeting, String> {
        Self::start_authenticated(port, meeting_id, chunk_secs, None)
    }
    pub fn start_authenticated(
        port: u16,
        meeting_id: &str,
        chunk_secs: f32,
        token: Option<&str>,
    ) -> Result<Meeting, String> {
        let mut endpoint = url(port, meeting_id);
        if let Some(token) = token {
            endpoint.push_str(&format!("&token={}", crate::chat::urlenc(token)));
        }
        let (mut socket, _) = tungstenite::connect(endpoint).map_err(|_| {
            "No se pudo abrir la reunión: revisa la conexión y la sesión".to_owned()
        })?;
        if let MaybeTlsStream::Plain(s) = socket.get_mut() {
            let _ = s.set_read_timeout(Some(Duration::from_millis(50)));
        }
        let (out, rx) = mpsc::channel::<String>();
        let events = Arc::new(Mutex::new(Vec::new()));
        let chunker = Arc::new(Mutex::new(Chunker::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ev, ch, st, tx) = (events.clone(), chunker.clone(), stop.clone(), out.clone());
        let n = (RATE as f32 * chunk_secs) as usize;
        thread::spawn(move || run(socket, rx, tx, ev, ch, st, n));
        Ok(Meeting {
            chunker,
            out,
            events,
            stop,
        })
    }

    /// Función para el hilo de audio del micrófono (bloques PCM16 a 16 kHz).
    pub fn mic_sink(&self) -> impl FnMut(Vec<u8>) + Send + 'static {
        let c = self.chunker.clone();
        move |pcm| c.lock().unwrap().push_mic(&pcm)
    }

    /// Ídem para el audio del equipo.
    pub fn system_sink(&self) -> impl FnMut(Vec<u8>) + Send + 'static {
        let c = self.chunker.clone();
        move |pcm| c.lock().unwrap().push_system(&pcm)
    }

    pub fn drain(&self) -> Vec<MEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }

    /// Manda lo que quede (si dura más de un segundo), cierra la sesión en el servidor y deja que
    /// lleguen las últimas transcripciones antes de cerrar el socket.
    pub fn finish(&self) {
        if let Some(rest) = self.chunker.lock().unwrap().flush(RATE as usize) {
            if !is_silent(&rest) {
                let _ = self.out.send(chunk_frame(&rest));
            }
        }
        let _ = self.out.send(json!({"type": "meeting_stop"}).to_string());
    }

    /// Cierra el socket ya (sin esperar transcripciones pendientes).
    pub fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Meeting {
    fn drop(&mut self) {
        self.close();
    }
}

fn run(
    mut socket: WebSocket<MaybeTlsStream<TcpStream>>,
    outgoing: Receiver<String>,
    _tx: Sender<String>,
    events: Arc<Mutex<Vec<MEvent>>>,
    chunker: Arc<Mutex<Chunker>>,
    stop: Arc<AtomicBool>,
    chunk_samples: usize,
) {
    let push = |e: MEvent| events.lock().unwrap().push(e);
    let mut last_ping = Instant::now();
    // Tras `meeting_stop` se espera a `meeting_stopped` (o 20 s) para recoger las últimas transcripciones.
    let mut stopping_since: Option<Instant> = None;
    loop {
        if stop.load(Ordering::SeqCst)
            || stopping_since.is_some_and(|t| t.elapsed() > Duration::from_secs(20))
        {
            break;
        }
        // Los trozos completos salen en cuanto están; el silencio no se manda.
        loop {
            let chunk = chunker.lock().unwrap().take(chunk_samples);
            match chunk {
                Some(c) if !is_silent(&c) => {
                    if socket.send(Message::text(chunk_frame(&c))).is_err() {
                        push(MEvent::Closed);
                        return;
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
        while let Ok(frame) = outgoing.try_recv() {
            if frame.contains("\"meeting_stop\"") {
                stopping_since = Some(Instant::now());
            }
            if socket.send(Message::text(frame)).is_err() {
                push(MEvent::Closed);
                return;
            }
        }
        if last_ping.elapsed() > Duration::from_secs(20) {
            let _ = socket.send(Message::text(json!({"type": "ping"}).to_string()));
            last_ping = Instant::now();
        }
        match socket.read() {
            Ok(Message::Text(t)) => {
                let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else {
                    continue;
                };
                match v["type"].as_str() {
                    Some("transcript_segment") => push(MEvent::Segment {
                        seq: v["seq"].as_u64().unwrap_or(0),
                        speaker: v["speaker"].as_str().map(str::to_string),
                        text: v["text"].as_str().unwrap_or("").trim().to_string(),
                    }),
                    Some("meeting_stopped") => {
                        push(MEvent::Stopped {
                            segments: v["segment_count"].as_u64().unwrap_or(0),
                        });
                        break;
                    }
                    Some("error") => push(MEvent::Error(
                        v["error"]
                            .as_str()
                            .unwrap_or("error desconocido")
                            .to_string(),
                    )),
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    let _ = socket.close(None);
    push(MEvent::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn tone(n: usize, amp: i16) -> Vec<i16> {
        (0..n)
            .map(|i| ((i as f32 * 0.3).sin() * amp as f32) as i16)
            .collect()
    }

    #[test]
    fn mixing_sums_saturates_and_pads_the_shorter_track() {
        assert_eq!(mix(&[100, 200, 300], &[1, 2]), [101, 202, 300]);
        assert_eq!(
            mix(&[30000], &[30000]),
            [i16::MAX],
            "satura, no da la vuelta"
        );
        assert_eq!(mix(&[-30000], &[-30000]), [i16::MIN]);
        assert!(mix(&[], &[]).is_empty());
    }

    #[test]
    fn the_wav_header_is_valid_pcm16_mono() {
        let w = wav(&[1, -2, 3], 16_000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(w[20..22].try_into().unwrap()), 1, "PCM");
        assert_eq!(u16::from_le_bytes(w[22..24].try_into().unwrap()), 1, "mono");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[28..32].try_into().unwrap()), 32_000);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 6);
        assert_eq!(samples(&w[44..]), [1, -2, 3], "las muestras salen tal cual");
        assert_eq!(w.len(), 50);
    }

    #[test]
    fn silence_is_told_from_speech() {
        assert!(is_silent(&vec![0; 16_000]));
        assert!(is_silent(&vec![20; 16_000]), "ruido de fondo mínimo");
        assert!(!is_silent(&tone(16_000, 3000)));
        assert!(is_silent(&[]));
    }

    #[test]
    fn the_chunker_mixes_both_tracks_and_waits_for_a_full_chunk() {
        let mut c = Chunker::default();
        assert!(c.take(4).is_none());
        c.push_mic(&pcm(&[1, 2]));
        assert!(c.take(4).is_none(), "aún no hay un trozo completo");
        c.push_system(&pcm(&[10, 20, 30, 40, 50]));
        assert_eq!(
            c.take(4).unwrap(),
            [11, 22, 30, 40],
            "mezcla; la pista corta aporta lo que tiene"
        );
        assert_eq!(
            c.flush(1).unwrap(),
            [50],
            "lo que sobra queda para el final"
        );
        assert!(c.flush(1).is_none());
        // Solo micrófono (sin audio del equipo).
        c.push_mic(&pcm(&[5, 6, 7, 8]));
        assert_eq!(c.take(4).unwrap(), [5, 6, 7, 8]);
    }

    #[test]
    fn flush_ignores_scraps_shorter_than_the_minimum() {
        let mut c = Chunker::default();
        c.push_mic(&pcm(&[1, 2, 3]));
        assert!(c.flush(100).is_none());
    }

    /// El servidor falso acepta la reunión, recibe lo que se manda y contesta como el gateway real.
    #[test]
    fn a_meeting_sends_wav_chunks_skips_silence_flushes_on_finish_and_reports_segments() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            ws.get_mut()
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let mut got: Vec<Value> = Vec::new();
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                if let Ok(Message::Text(t)) = ws.read() {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    match v["type"].as_str() {
                        Some("audio_chunk") => {
                            ws.send(Message::text(json!({"type": "transcript_segment", "seq": got.len(), "speaker": null, "text": " hola equipo "}).to_string())).unwrap();
                        }
                        Some("meeting_stop") => {
                            ws.send(Message::text(json!({"type": "meeting_stopped", "session_id": "m1", "segment_count": 2}).to_string())).unwrap();
                            got.push(v);
                            break;
                        }
                        _ => {}
                    }
                    if got
                        .last()
                        .map(|l| l["type"] != "meeting_stop")
                        .unwrap_or(true)
                    {
                        got.push(v);
                    }
                }
            }
            got
        });
        // Trozos de 0,25 s para que la prueba sea rápida.
        let m = Meeting::start(port, "m1", 0.25).unwrap();
        let (mut mic, mut sys) = (m.mic_sink(), m.system_sink());
        let n = 4000; // 0,25 s a 16 kHz
        mic(pcm(&vec![0; n])); // trozo 1: silencio, no se manda
        mic(pcm(&tone(n, 4000))); // trozo 2: voz
        sys(pcm(&tone(n, 4000))); // se mezcla con el micrófono
                                  // Quedan 1,5 s de voz sin completar un trozo: salen al terminar.
        mic(pcm(&tone(RATE as usize * 3 / 2, 4000)));
        let end = Instant::now() + Duration::from_secs(3);
        let mut events = Vec::new();
        while Instant::now() < end && !events.iter().any(|e| matches!(e, MEvent::Segment { .. })) {
            thread::sleep(Duration::from_millis(20));
            events.extend(m.drain());
        }
        m.finish();
        while Instant::now() < end && !events.contains(&MEvent::Closed) {
            thread::sleep(Duration::from_millis(20));
            events.extend(m.drain());
        }
        let got = server.join().unwrap();
        let chunks: Vec<&Value> = got.iter().filter(|v| v["type"] == "audio_chunk").collect();
        assert!(
            chunks.len() >= 2,
            "el trozo con voz y el resto final: {got:?}"
        );
        for c in &chunks {
            assert_eq!(c["mime_type"], "audio/wav");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(c["audio"].as_str().unwrap())
                .unwrap();
            assert_eq!(&bytes[0..4], b"RIFF");
            assert!(
                !is_silent(&samples(&bytes[44..])),
                "el silencio nunca se manda"
            );
        }
        assert_eq!(
            got.last().unwrap()["type"],
            "meeting_stop",
            "al terminar se cierra la sesión"
        );
        assert!(
            events.contains(&MEvent::Segment {
                seq: 0,
                speaker: None,
                text: "hola equipo".into()
            }),
            "{events:?}"
        );
        assert!(events.contains(&MEvent::Stopped { segments: 2 }));
        assert_eq!(events.last(), Some(&MEvent::Closed));
    }

    #[test]
    fn rest_calls_use_the_documented_paths_and_report_server_errors() {
        use crate::testutil::fake_http;
        let (port, server) = fake_http(
            |m, p, _, _| match (m, p) {
                ("POST", "/api/meetings") => (
                    200,
                    json!({"ok": true, "session": {"id": "m 1", "title": "Semanal"}}),
                ),
                ("GET", "/api/meetings") => (
                    200,
                    json!({"ok": true, "sessions": [{"id": "m1", "title": "Semanal", "status": "stopped", "segment_count": 7, "report_path": null}, {"title": "sin id"}]}),
                ),
                ("POST", "/api/meetings/m1/stop") => (200, json!({"ok": true})),
                ("POST", "/api/meetings/m1/report") => {
                    (200, json!({"ok": true, "report_path": "/tmp/informe.docx"}))
                }
                _ => (
                    200,
                    json!({"ok": false, "error": "La sesión no tiene segmentos transcritos."}),
                ),
            },
            5,
        );
        let c = Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: Some("t".into()),
        };
        assert_eq!(create(&c, "Semanal").unwrap(), "m 1");
        let l = list(&c).unwrap();
        assert_eq!(l.len(), 1, "una fila sin id se ignora");
        assert_eq!(
            (l[0].id.as_str(), l[0].status.as_str(), l[0].segments),
            ("m1", "stopped", 7)
        );
        stop(&c, "m1").unwrap();
        assert_eq!(report(&c, "m1").unwrap(), "/tmp/informe.docx");
        assert_eq!(
            report(&c, "otra").unwrap_err().to_string(),
            "La sesión no tiene segmentos transcritos."
        );
        let seen = server.join().unwrap();
        let body: Value = serde_json::from_str(&seen[0].3).unwrap();
        assert_eq!(body, json!({"title": "Semanal"}), "cuerpo de creación");
        assert_eq!(seen[2].1, "/api/meetings/m1/stop");
    }
}
