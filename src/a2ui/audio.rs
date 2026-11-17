//! `AudioPlayer` de A2UI: descarga, decodifica y reproduce un clip con `hive-media`.
//!
//! Nunca suena solo: el agente decide la URL, pero reproducir exige un clic. La descarga usa la
//! misma política de URLs que las imágenes (ver `images::fetch`) y el clip se recorta a dos minutos.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SymError, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

use super::images::{fetch, Origin};

pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_SECONDS: u32 = 120;
pub const OUT_RATE: u32 = 24_000;

/// Decodifica cualquier formato soportado a mono `f32`, recortado a `max_secs`.
pub fn decode(bytes: Vec<u8>, max_secs: u32) -> Result<(Vec<f32>, u32), String> {
    let stream = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| "formato de audio no reconocido".to_string())?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or("el archivo no tiene audio")?
        .clone();
    let rate = track
        .codec_params
        .sample_rate
        .ok_or("el audio no declara su frecuencia")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| "códec de audio no soportado".to_string())?;
    let cap = rate as usize * max_secs as usize;
    let mut mono = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(_)) | Err(SymError::ResetRequired) => break,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                let channels = spec.channels.count().max(1);
                let mut samples = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                samples.copy_interleaved_ref(buf);
                mono.extend(
                    samples
                        .samples()
                        .chunks(channels)
                        .map(|f| f.iter().sum::<f32>() / channels as f32),
                );
                if mono.len() >= cap {
                    mono.truncate(cap);
                    break;
                }
            }
            Err(SymError::DecodeError(_)) => continue, // un paquete dañado no tumba el clip
            Err(_) => break,
        }
    }
    if mono.is_empty() {
        return Err("el audio está vacío o no se pudo decodificar".into());
    }
    Ok((mono, rate))
}

/// Mono `f32` a PCM16 little-endian a 24 kHz (lo que espera `hive_media::Player`), por interpolación lineal.
pub fn to_pcm16_24k(samples: &[f32], rate: u32) -> Vec<u8> {
    let out_len = (samples.len() as u64 * OUT_RATE as u64 / rate.max(1) as u64) as usize;
    let mut out = Vec::with_capacity(out_len * 2);
    for i in 0..out_len {
        let pos = i as f64 * rate as f64 / OUT_RATE as f64;
        let (a, frac) = (pos.floor() as usize, pos.fract() as f32);
        let x = samples[a.min(samples.len() - 1)];
        let y = samples[(a + 1).min(samples.len() - 1)];
        let v = (x + (y - x) * frac).clamp(-1.0, 1.0);
        out.extend_from_slice(&((v * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    Loading,
    Ready { pcm: Arc<Vec<u8>>, seconds: f32 },
    Failed(String),
}

/// Quien hace sonar el PCM: el altavoz real, o una memoria en las pruebas.
pub trait Sink: Send + Sync {
    fn play(&self, pcm24k: &[u8]) -> Result<(), String>;
    fn stop(&self);
    fn queued_ms(&self) -> u64;
    /// Efecto breve: se suma a lo que suena sin cortarlo.
    fn cue(&self, pcm24k: &[u8]) -> Result<(), String> {
        self.play(pcm24k)
    }
}

/// Altavoz del sistema; se abre la primera vez que suena algo.
#[derive(Default)]
pub struct Speaker(Mutex<Option<hive_media::Player>>);

impl Sink for Speaker {
    fn play(&self, pcm: &[u8]) -> Result<(), String> {
        let mut slot = self.0.lock().unwrap();
        if slot.is_none() {
            *slot = Some(
                hive_media::Player::start(None, None)
                    .map_err(|e| format!("No se pudo abrir el altavoz: {}", e.message))?,
            );
        }
        let player = slot.as_ref().unwrap();
        player.interrupt();
        player.enqueue(pcm);
        Ok(())
    }

    fn cue(&self, pcm: &[u8]) -> Result<(), String> {
        let mut slot = self.0.lock().unwrap();
        if slot.is_none() {
            *slot = Some(
                hive_media::Player::start(None, None)
                    .map_err(|e| format!("No se pudo abrir el altavoz: {}", e.message))?,
            );
        }
        slot.as_ref().unwrap().enqueue(pcm);
        Ok(())
    }

    fn stop(&self) {
        if let Some(p) = self.0.lock().unwrap().as_ref() {
            p.interrupt();
        }
    }

    fn queued_ms(&self) -> u64 {
        self.0.lock().unwrap().as_ref().map_or(0, |p| p.queued_ms())
    }
}

#[derive(Default)]
struct Inner {
    origin: Option<Origin>,
    clips: HashMap<String, Clip>,
}

/// Caché de clips por URL; la descarga arranca solo cuando alguien quiere oírlos.
#[derive(Clone, Default)]
pub struct Clips {
    inner: Arc<Mutex<Inner>>,
    dirty: Arc<AtomicBool>,
}

impl Clips {
    pub fn set_origin(&self, base: &str, token: Option<&str>) {
        self.inner.lock().unwrap().origin = Some(Origin {
            base: base.to_string(),
            token: token.map(str::to_string),
        });
    }

    /// Estado sin iniciar nada (la vista lo usa al pintar).
    pub fn peek(&self, url: &str) -> Option<Clip> {
        self.inner.lock().unwrap().clips.get(url).cloned()
    }

    /// Estado de `url`; la primera vez arranca la descarga.
    pub fn request(&self, url: &str) -> Clip {
        let mut g = self.inner.lock().unwrap();
        if let Some(c) = g.clips.get(url) {
            return c.clone();
        }
        g.clips.insert(url.to_string(), Clip::Loading);
        let origin = g.origin.clone();
        drop(g);
        let (this, u) = (self.clone(), url.to_string());
        thread::spawn(move || {
            let result = fetch(&u, origin.as_ref(), MAX_BYTES)
                .and_then(|b| decode(b, MAX_SECONDS))
                .map(|(mono, rate)| {
                    let seconds = mono.len() as f32 / rate as f32;
                    Clip::Ready {
                        pcm: Arc::new(to_pcm16_24k(&mono, rate)),
                        seconds,
                    }
                });
            this.inner
                .lock()
                .unwrap()
                .clips
                .insert(u, result.unwrap_or_else(Clip::Failed));
            this.dirty.store(true, Ordering::SeqCst);
        });
        Clip::Loading
    }

    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }
}

/// Control de reproducción: un solo clip a la vez y solo bajo petición.
pub struct Audio {
    pub clips: Clips,
    pub sink: Arc<dyn Sink>,
    /// Clip que se está oyendo.
    playing: Option<String>,
    /// Clip pedido que aún se está descargando: suena al llegar.
    wanted: Option<String>,
    pub error: Option<String>,
    /// Clip al que pertenece `error`.
    pub error_url: Option<String>,
}

impl Default for Audio {
    fn default() -> Self {
        Audio {
            clips: Clips::default(),
            sink: Arc::new(Speaker::default()),
            playing: None,
            wanted: None,
            error: None,
            error_url: None,
        }
    }
}

impl Audio {
    /// Acción del usuario: oír `url` (descargándola si hace falta).
    pub fn play(&mut self, url: &str) {
        self.error = None;
        self.error_url = Some(url.to_string());
        self.sink.stop();
        self.playing = None;
        self.wanted = Some(url.to_string());
        self.advance();
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.playing = None;
        self.wanted = None;
    }

    /// Hace avanzar lo pedido cuando termina una descarga. Devuelve si algo cambió.
    pub fn tick(&mut self) -> bool {
        let mut changed = self.clips.take_dirty();
        if self.wanted.is_some() {
            changed |= self.advance();
        }
        if self.playing.is_some() && self.sink.queued_ms() == 0 {
            self.playing = None; // terminó el clip
            changed = true;
        }
        changed
    }

    fn advance(&mut self) -> bool {
        let Some(url) = self.wanted.clone() else {
            return false;
        };
        match self.clips.request(&url) {
            Clip::Loading => false,
            Clip::Failed(e) => {
                self.error = Some(e);
                self.wanted = None;
                true
            }
            Clip::Ready { pcm, .. } => {
                self.wanted = None;
                match self.sink.play(&pcm) {
                    Ok(()) => self.playing = Some(url),
                    Err(e) => self.error = Some(e),
                }
                true
            }
        }
    }

    pub fn is_playing(&self, url: &str) -> bool {
        self.playing.as_deref() == Some(url)
    }

    pub fn is_loading(&self, url: &str) -> bool {
        self.wanted.as_deref() == Some(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// WAV PCM16: `channels` canales, `rate` Hz, `secs` segundos de un tono de 440 Hz.
    pub fn wav(channels: u16, rate: u32, secs: f32) -> Vec<u8> {
        let frames = (rate as f32 * secs) as u32;
        let data_len = frames * channels as u32 * 2;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data_len).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(channels.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * channels as u32 * 2).to_le_bytes());
        b.extend((channels * 2).to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data_len.to_le_bytes());
        for i in 0..frames {
            let v = ((2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 12000.0)
                as i16;
            for _ in 0..channels {
                b.extend(v.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn decodes_wav_to_mono_at_its_own_rate() {
        let (mono, rate) = decode(wav(1, 8000, 0.5), MAX_SECONDS).unwrap();
        assert_eq!((rate, mono.len()), (8000, 4000));
        assert!(mono.iter().any(|v| v.abs() > 0.2), "tiene señal");
    }

    #[test]
    fn stereo_is_mixed_down_to_one_channel() {
        let (mono, rate) = decode(wav(2, 16000, 0.25), MAX_SECONDS).unwrap();
        assert_eq!((rate, mono.len()), (16000, 4000));
    }

    #[test]
    fn clips_are_cut_to_the_maximum_length() {
        let (mono, rate) = decode(wav(1, 8000, 3.0), 1).unwrap();
        assert_eq!(mono.len(), rate as usize, "un segundo");
    }

    #[test]
    fn junk_and_empty_audio_are_rejected() {
        assert!(decode(b"no es audio".to_vec(), MAX_SECONDS).is_err());
        assert!(
            decode(wav(1, 8000, 0.0), MAX_SECONDS).is_err(),
            "sin muestras"
        );
    }

    #[test]
    fn resampling_to_24k_scales_the_length_and_stays_in_range() {
        let (mono, rate) = decode(wav(1, 8000, 0.5), MAX_SECONDS).unwrap();
        let pcm = to_pcm16_24k(&mono, rate);
        assert_eq!(pcm.len(), 12000 * 2, "8 kHz a 24 kHz triplica las muestras");
        let peak = pcm
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
            .max()
            .unwrap();
        assert!((10000..=13000).contains(&peak), "pico {peak}");
        assert_eq!(
            to_pcm16_24k(&[0.5], 24000).len(),
            2,
            "una muestra no rompe la interpolación"
        );
    }

    // ── control de reproducción, con un altavoz de memoria ──

    #[derive(Default)]
    struct Mem {
        played: Mutex<Vec<usize>>,
        stops: Mutex<u32>,
        queued: Mutex<u64>,
    }
    impl Sink for Mem {
        fn play(&self, pcm: &[u8]) -> Result<(), String> {
            self.played.lock().unwrap().push(pcm.len());
            *self.queued.lock().unwrap() = 500;
            Ok(())
        }
        fn stop(&self) {
            *self.stops.lock().unwrap() += 1;
            *self.queued.lock().unwrap() = 0;
        }
        fn queued_ms(&self) -> u64 {
            *self.queued.lock().unwrap()
        }
    }

    /// Un servidor de un solo uso que sirve `body`.
    fn serve(body: Vec<u8>) -> (u16, thread::JoinHandle<()>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            s.write_all(&body).unwrap();
        });
        (port, h)
    }

    fn audio_with(mem: Arc<Mem>, port: Option<u16>) -> Audio {
        let a = Audio {
            sink: mem,
            ..Audio::default()
        };
        if let Some(p) = port {
            a.clips.set_origin(&format!("http://127.0.0.1:{p}"), None);
        }
        a
    }

    fn wait_for(a: &mut Audio, done: impl Fn(&Audio) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done(a) && std::time::Instant::now() < end {
            a.tick();
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn nothing_is_downloaded_or_played_until_the_user_asks() {
        let mem = Arc::new(Mem::default());
        let mut a = audio_with(mem.clone(), None);
        assert!(!a.tick());
        assert!(a.clips.peek("/x.wav").is_none(), "mirar no descarga");
        assert!(mem.played.lock().unwrap().is_empty());
    }

    #[test]
    fn play_downloads_then_plays_when_the_clip_arrives() {
        let (port, server) = serve(wav(1, 8000, 0.5));
        let mem = Arc::new(Mem::default());
        let mut a = audio_with(mem.clone(), Some(port));
        a.play("/clip.wav");
        assert!(
            a.is_loading("/clip.wav") && !a.is_playing("/clip.wav"),
            "mientras baja, no suena"
        );
        wait_for(&mut a, |a| a.is_playing("/clip.wav"));
        assert!(a.is_playing("/clip.wav"));
        assert_eq!(*mem.played.lock().unwrap(), vec![12000 * 2]);
        server.join().unwrap();
        // Al vaciarse la cola, el clip termina.
        *mem.queued.lock().unwrap() = 0;
        assert!(a.tick() && !a.is_playing("/clip.wav"));
    }

    #[test]
    fn stop_silences_and_cancels_a_pending_play() {
        let (port, server) = serve(wav(1, 8000, 0.5));
        let mem = Arc::new(Mem::default());
        let mut a = audio_with(mem.clone(), Some(port));
        a.play("/clip.wav");
        a.stop();
        assert!(!a.is_loading("/clip.wav"));
        thread::sleep(std::time::Duration::from_millis(300));
        a.tick();
        assert!(
            mem.played.lock().unwrap().is_empty(),
            "lo cancelado no suena al llegar"
        );
        server.join().unwrap();
    }

    #[test]
    fn a_blocked_or_bad_clip_reports_an_error_and_stays_silent() {
        let mem = Arc::new(Mem::default());
        let mut a = audio_with(mem.clone(), None);
        a.play("http://127.0.0.1:9/x.wav");
        wait_for(&mut a, |a| a.error.is_some());
        assert!(
            a.error
                .as_deref()
                .is_some_and(|e| e.contains("no permitida")),
            "{:?}",
            a.error
        );
        assert!(mem.played.lock().unwrap().is_empty());

        let (port, server) = serve(b"no es audio".to_vec());
        let mut a = audio_with(mem.clone(), Some(port));
        a.play("/mala.wav");
        wait_for(&mut a, |a| a.error.is_some());
        assert!(
            a.error
                .as_deref()
                .is_some_and(|e| e.contains("no reconocido")),
            "{:?}",
            a.error
        );
        server.join().unwrap();
    }

    #[test]
    fn playing_another_clip_replaces_the_current_one() {
        let (p1, s1) = serve(wav(1, 8000, 0.25));
        let mem = Arc::new(Mem::default());
        let mut a = audio_with(mem.clone(), Some(p1));
        a.play("/uno.wav");
        wait_for(&mut a, |a| a.is_playing("/uno.wav"));
        let stops_before = *mem.stops.lock().unwrap();
        a.play("/uno.wav"); // ya en caché: suena de nuevo sin otra descarga
        assert!(a.is_playing("/uno.wav"));
        assert!(
            *mem.stops.lock().unwrap() > stops_before,
            "se corta lo que sonaba antes de empezar"
        );
        assert_eq!(mem.played.lock().unwrap().len(), 2);
        s1.join().unwrap();
    }

    /// Hace sonar un tono corto y bajo por el altavoz real del sistema:
    /// `cargo test -- --ignored speaker_plays`.
    #[test]
    #[ignore]
    fn speaker_plays_a_decoded_clip_on_the_real_device() {
        let (mono, rate) = decode(wav(1, 8000, 0.4), MAX_SECONDS).unwrap();
        let quiet: Vec<f32> = mono.iter().map(|v| v * 0.25).collect();
        let speaker = Speaker::default();
        speaker
            .play(&to_pcm16_24k(&quiet, rate))
            .expect("abre el altavoz");
        assert!(
            speaker.queued_ms() > 300,
            "quedó en cola: {} ms",
            speaker.queued_ms()
        );
        let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while speaker.queued_ms() > 0 && std::time::Instant::now() < end {
            thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(speaker.queued_ms(), 0, "el clip terminó de sonar");
        speaker.stop();
    }
}
