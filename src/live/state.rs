//! Estado de la pantalla HiveLive: lo que se ve de la sesión de voz (telemetría, hitos de la colmena)
//! y las preferencias de voz e idioma. Todo viene de eventos reales de `/realtime`.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::realtime::REvent;

use super::lipsync::{self, Sound};
use super::model::Face;

/// Voces de Gemini Live y su carácter (las mismas que la web).
pub const VOICES: &[(&str, &str)] = &[
    ("Kore", "Femenina, firme"),
    ("Aoede", "Femenina, cálida"),
    ("Leda", "Femenina, juvenil"),
    ("Zephyr", "Femenina, luminosa"),
    ("Autonoe", "Femenina, serena"),
    ("Despina", "Femenina, suave"),
    ("Puck", "Masculina, animada"),
    ("Charon", "Masculina, grave"),
    ("Fenrir", "Masculina, enérgica"),
    ("Orus", "Masculina, neutra"),
];

/// Idiomas con acento regional; el primero es el predeterminado.
pub const LANGS: &[(&str, &str)] = &[
    ("es-CO", "Español (Colombia)"),
    ("es-MX", "Español (México)"),
    ("es-AR", "Español (Argentina)"),
    ("es-ES", "Español (España)"),
    ("es-US", "Español (EE. UU.)"),
    ("en-US", "Inglés (EE. UU.)"),
    ("pt-BR", "Portugués (Brasil)"),
];

/// Cuánto dura «hablando» tras el último fragmento de la respuesta, y «trabajando» tras un hito.
const SPEAKING_FOR: Duration = Duration::from_millis(1400);
const WORKING_FOR: Duration = Duration::from_secs(12);
const ERROR_FOR: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bia {
    Connecting,
    Idle,
    Listening,
    Working,
    Speaking,
    Error,
}

impl Bia {
    /// El texto de estado de la cabecera.
    pub fn label(self, agent: &str) -> String {
        match self {
            Bia::Connecting => "Abriendo".into(),
            Bia::Working => "La colmena trabaja".into(),
            Bia::Speaking => format!("{agent} habla"),
            Bia::Listening => "Escuchando".into(),
            Bia::Error => "Error".into(),
            Bia::Idle => "En reposo".into(),
        }
    }

    /// Color del estado (sRGB 0–1), el de `COLOR_ESTADO` de la web.
    pub fn tint(self) -> [f32; 3] {
        let hex = match self {
            Bia::Idle | Bia::Connecting => 0xd8a541,
            Bia::Listening => 0xffb43a,
            Bia::Working => 0x7a5ff1,
            Bia::Speaking => 0x42d3fc,
            Bia::Error => 0xf2557a,
        };
        [
            (hex >> 16 & 0xff) as f32 / 255.0,
            (hex >> 8 & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
        ]
    }
}

/// Un hito de la colmena tal como lo emite el agent-loop.
#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    pub kind: String,
    pub label: String,
    pub detail: Option<String>,
    pub agent: String,
}

pub struct Live {
    pub scene_loading: bool,
    pub scene_error: Option<String>,
    pub scene_retry: u64,
    pub camera_preview: super::preview::Preview,
    pub screen_preview: super::preview::Preview,
    pub catalog: super::catalog::Catalog,
    pub started: Option<Instant>,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Del último fragmento hablado por la persona al primero de la respuesta.
    pub latency_ms: Option<u64>,
    last_user: Option<Instant>,
    awaiting_reply: bool,
    last_assistant: Option<Instant>,
    last_milestone: Option<Instant>,
    error_at: Option<Instant>,
    pub error: Option<String>,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
    pub milestones: Vec<Milestone>,
    pub specialists: BTreeSet<String>,
    pub voice: usize,
    pub lang: usize,
    /// El panel de telemetría plegado a su cabecera.
    pub panel_collapsed: bool,
    /// En un escenario estrecho el panel arranca plegado; esto lo abre.
    pub panel_open_narrow: bool,
    /// Lista desplegada: 0 = ninguna, 1 = voz, 2 = idioma.
    pub open_select: u8,
    /// Tamaño del escenario en puntos (lo fija el compositor cada fotograma).
    pub viewport: (f32, f32),
    pub t: f32,
    /// Nivel de la voz de BIA, de 0 a 1 (sube al hablar y cae solo).
    pub level: f32,
    /// Avatar: el modelo 3D (por defecto) o el retrato.
    pub avatar_model: bool,
    /// Giro del modelo con el ratón (radianes) y si se está arrastrando.
    pub yaw: f32,
    /// Cámara del modelo (como `OrbitControls` de la web): desplazamiento y distancia.
    pub pan: [f32; 2],
    pub dist: f32,
    pub dragging: bool,
    /// Último `dx` del arrastre, para girar solo lo que se movió.
    pub drag_last: f32,
    pub drag_last_y: f32,
    /// Cómo suena la voz de BIA ahora (suavizado), lo último medido, y cuándo llegó (en el reloj de `t`).
    pub mouth: Sound,
    mouth_target: Sound,
    audio_t: f32,
    /// Trozos de audio que aún no han sonado: (cuándo suenan, cómo suenan).
    pending: std::collections::VecDeque<(f32, Sound)>,
    /// Si en esta llamada ya llegó audio del modelo: entonces la boca sigue al audio y no a las palabras.
    has_audio: bool,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            scene_loading: false,
            scene_error: None,
            scene_retry: 0,
            camera_preview: Default::default(),
            screen_preview: Default::default(),
            catalog: Default::default(),
            started: None,
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            latency_ms: None,
            last_user: None,
            awaiting_reply: false,
            last_assistant: None,
            last_milestone: None,
            error_at: None,
            error: None,
            input_price: None,
            output_price: None,
            milestones: Vec::new(),
            specialists: BTreeSet::new(),
            voice: 0,
            lang: 0,
            panel_collapsed: false,
            panel_open_narrow: false,
            open_select: 0,
            viewport: (0.0, 0.0),
            t: 0.0,
            level: 0.0,
            avatar_model: true,
            yaw: 0.0,
            pan: [0.0; 2],
            dist: crate::live::bia_gpu::EYE[2],
            dragging: false,
            drag_last: 0.0,
            drag_last_y: 0.0,
            mouth: Sound::default(),
            mouth_target: Sound::default(),
            audio_t: -10.0,
            pending: std::collections::VecDeque::new(),
            has_audio: false,
        }
    }
}

impl Live {
    pub fn voice_id(&self) -> &'static str {
        VOICES[self.voice.min(VOICES.len() - 1)].0
    }

    pub fn lang_id(&self) -> &'static str {
        LANGS[self.lang.min(LANGS.len() - 1)].0
    }

    /// Nueva llamada: la telemetría de la anterior no se arrastra.
    pub fn reset(&mut self) {
        *self = Live {
            scene_loading: self.scene_loading,
            scene_error: self.scene_error.take(),
            scene_retry: self.scene_retry,
            catalog: std::mem::take(&mut self.catalog),
            voice: self.voice,
            lang: self.lang,
            panel_collapsed: self.panel_collapsed,
            viewport: self.viewport,
            t: self.t,
            avatar_model: self.avatar_model,
            yaw: self.yaw,
            pan: self.pan,
            dist: self.dist,
            ..Default::default()
        };
    }

    pub fn on_event(&mut self, ev: &REvent, now: Instant) {
        match ev {
            REvent::Pricing { input, output } => {
                self.input_price = *input;
                self.output_price = *output;
            }
            REvent::Ready { model } => {
                self.error = None;
                self.started = Some(now);
                self.model = Some(model.clone());
            }
            REvent::Transcript {
                assistant: false, ..
            } => {
                self.last_user = Some(now);
                self.awaiting_reply = true;
            }
            REvent::Transcript {
                assistant: true, ..
            } => {
                if self.awaiting_reply {
                    self.latency_ms = self
                        .last_user
                        .map(|t| now.duration_since(t).as_millis() as u64);
                    self.awaiting_reply = false;
                }
                self.last_assistant = Some(now);
                self.level = 1.0;
            }
            REvent::Mouth {
                level,
                bright,
                delay_ms,
            } => {
                self.has_audio = true;
                // El trozo suena cuando termine lo que ya había en la cola del altavoz.
                self.pending.push_back((
                    self.t + *delay_ms as f32 / 1000.0,
                    Sound {
                        level: *level,
                        bright: *bright,
                    },
                ));
                if self.pending.len() > 256 {
                    self.pending.pop_front();
                }
            }
            REvent::Usage { input, output } => {
                self.input_tokens = *input;
                self.output_tokens = *output;
            }
            REvent::Narration {
                kind,
                label,
                detail,
                agent,
            } => {
                if matches!(kind.as_str(), "delegated" | "worker_started" | "tool_call") {
                    self.last_milestone = Some(now);
                }
                if kind == "group_ready" || kind == "verified" {
                    self.last_milestone = None; // la colmena terminó
                }
                if !agent.is_empty() {
                    self.specialists.insert(agent.clone());
                }
                self.milestones.push(Milestone {
                    kind: kind.clone(),
                    label: label.clone(),
                    detail: detail.clone(),
                    agent: agent.clone(),
                });
                if self.milestones.len() > 40 {
                    self.milestones.remove(0);
                }
            }
            REvent::Error(error) => {
                self.error_at = Some(now);
                self.error = Some(error.clone());
            }
            REvent::Closed => {
                self.started = None;
                self.last_milestone = None;
            }
            REvent::Interrupted | REvent::TurnComplete | REvent::Tool(_) => {}
        }
    }

    /// Qué está haciendo BIA ahora, según lo último que llegó.
    pub fn bia(&self, voice_on: bool, voice_live: bool, now: Instant) -> Bia {
        let recent = |t: Option<Instant>, d: Duration| t.is_some_and(|t| now.duration_since(t) < d);
        if recent(self.error_at, ERROR_FOR) {
            Bia::Error
        } else if voice_on && !voice_live {
            Bia::Connecting
        } else if !voice_live {
            Bia::Idle
        } else if recent(self.last_milestone, WORKING_FOR) {
            Bia::Working
        } else if recent(self.last_assistant, SPEAKING_FOR) {
            Bia::Speaking
        } else {
            Bia::Listening
        }
    }

    /// Avanza el reloj y deja caer el nivel de la voz.
    pub fn step(&mut self, dt: f32) {
        self.t += dt;
        self.level = (self.level - dt * 1.6).max(0.0);
        // Los trozos de audio cuya hora ha llegado pasan a ser lo que suena ahora.
        while self.pending.front().is_some_and(|(due, _)| *due <= self.t) {
            let (_, sound) = self.pending.pop_front().unwrap();
            self.mouth_target = sound;
            self.audio_t = self.t;
            // El halo y el polen también laten con la voz real.
            self.level = self.level.max(sound.level);
        }
        // La boca: sube rápido (40 ms) y baja más despacio (120 ms); sin audio nuevo en 0,2 s, se cierra.
        if self.t - self.audio_t > 0.2 {
            self.mouth_target = Sound::default();
        }
        let k = |tau: f32| 1.0 - (-dt / tau).exp();
        let up = self.mouth_target.level > self.mouth.level;
        self.mouth.level +=
            (self.mouth_target.level - self.mouth.level) * k(if up { 0.04 } else { 0.12 });
        self.mouth.bright += (self.mouth_target.bright - self.mouth.bright) * k(0.08);
    }

    /// El gesto de la cara: la mandíbula y los visemas siguen la voz de BIA, sonríe en reposo y parpadea.
    pub fn face(&self) -> Face {
        let phase = self.t % 4.2;
        let blink = if phase < 0.14 {
            (phase / 0.14 * std::f32::consts::PI).sin()
        } else {
            0.0
        };
        // Con audio real la boca lo sigue; sin él (solo texto) queda la aproximación por palabras.
        if self.has_audio {
            return Face {
                blink,
                ..lipsync::face(self.mouth, 0.15)
            };
        }
        let (t, l) = (self.t, self.level);
        let wave = |f: f32, ph: f32| 0.5 + 0.5 * (t * f + ph).sin();
        Face {
            jaw: (l * 0.9).min(1.0),
            aa: l * 0.55 * wave(9.0, 0.0),
            o: l * 0.3 * wave(6.0, 1.0),
            e: l * 0.25 * wave(7.5, 2.0),
            u: 0.0,
            smile: 0.15 * (1.0 - l),
            blink,
        }
    }

    /// Gira el modelo por un arrastre de `dx` puntos, con tope de medio giro a cada lado.
    pub fn drag_yaw(&mut self, dx: f32) {
        self.yaw = (self.yaw + dx * 0.01).clamp(-1.2, 1.2);
    }

    /// Desplaza la cámara por un arrastre de `(dx, dy)` puntos: el contenido sigue al puntero (como el botón izquierdo de la web),
    /// con los mismos topes que `SceneControls`. A más zoom, menos mundo por punto.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        let h = self.viewport.1.max(1.0);
        let per_point = 2.0 * self.dist * (16.0f32.to_radians()).tan() / h;
        self.pan[0] = (self.pan[0] - dx * per_point).clamp(-4.5, 4.5);
        self.pan[1] = (self.pan[1] + dy * per_point).clamp(-2.8, 3.2);
    }

    /// Acerca (`steps` > 0) o aleja la cámara, entre 2,4 y 13 como la web.
    pub fn zoom_by(&mut self, steps: f32) {
        self.dist = (self.dist * (-steps * 0.12).exp()).clamp(
            crate::live::bia_gpu::DIST_MIN,
            crate::live::bia_gpu::DIST_MAX,
        );
    }

    /// Doble clic (o el botón): vuelve al encuadre de partida.
    pub fn reset_view(&mut self) {
        self.pan = [0.0; 2];
        self.dist = crate::live::bia_gpu::EYE[2];
        self.yaw = 0.0;
    }

    pub fn elapsed_label(&self, now: Instant) -> String {
        match self.started {
            None => "—".into(),
            Some(t) => {
                let s = now.duration_since(t).as_secs();
                format!("{:02}:{:02}", s / 60, s % 60)
            }
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Costo de la sesión en dólares, con los precios por millón de tokens de la web.
    pub fn cost_usd(&self) -> f64 {
        self.input_tokens as f64 * self.input_price.unwrap_or(0.0) / 1e6
            + self.output_tokens as f64 * self.output_price.unwrap_or(0.0) / 1e6
    }
    pub fn has_pricing(&self) -> bool {
        self.input_price.is_some() || self.output_price.is_some()
    }
}

/// `gemini-3.8-live` → `3.8-live`: el nombre corto que muestra la telemetría.
pub fn short_model(model: &str) -> String {
    model.strip_prefix("gemini-").unwrap_or(model).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn the_defaults_are_the_webs_first_voice_and_language() {
        let l = Live::default();
        assert_eq!((l.voice_id(), l.lang_id()), ("Kore", "es-CO"));
        assert_eq!(VOICES.len(), 10);
        assert_eq!(LANGS.len(), 7);
    }

    #[test]
    fn the_state_follows_what_arrives() {
        let (mut l, t0) = (Live::default(), Instant::now());
        assert_eq!(l.bia(false, false, t0), Bia::Idle);
        assert_eq!(
            l.bia(true, false, t0),
            Bia::Connecting,
            "llamada pedida, sin sesión aún"
        );
        l.on_event(
            &REvent::Ready {
                model: "gemini-3.8-live".into(),
            },
            t0,
        );
        assert_eq!(l.bia(true, true, at(t0, 10)), Bia::Listening);
        l.on_event(
            &REvent::Transcript {
                assistant: true,
                text: "hola".into(),
            },
            at(t0, 100),
        );
        assert_eq!(l.bia(true, true, at(t0, 200)), Bia::Speaking);
        assert_eq!(
            l.bia(true, true, at(t0, 2000)),
            Bia::Listening,
            "calla pasado un instante"
        );
        l.on_event(
            &REvent::Narration {
                kind: "delegated".into(),
                label: "Buscar".into(),
                detail: None,
                agent: "web".into(),
            },
            at(t0, 2100),
        );
        assert_eq!(l.bia(true, true, at(t0, 2200)), Bia::Working);
        l.on_event(
            &REvent::Narration {
                kind: "group_ready".into(),
                label: "Listo".into(),
                detail: None,
                agent: "".into(),
            },
            at(t0, 3000),
        );
        assert_eq!(
            l.bia(true, true, at(t0, 3100)),
            Bia::Listening,
            "al terminar la colmena vuelve a escuchar"
        );
        l.on_event(&REvent::Error("x".into()), at(t0, 3200));
        assert_eq!(l.bia(true, true, at(t0, 3300)), Bia::Error);
        assert_eq!(
            l.bia(true, true, at(t0, 9000)),
            Bia::Listening,
            "el error se pasa solo"
        );
    }

    #[test]
    fn latency_is_the_gap_between_the_persons_last_words_and_the_first_reply() {
        let (mut l, t0) = (Live::default(), Instant::now());
        l.on_event(
            &REvent::Transcript {
                assistant: false,
                text: "hola ".into(),
            },
            t0,
        );
        l.on_event(
            &REvent::Transcript {
                assistant: false,
                text: "equipo".into(),
            },
            at(t0, 400),
        );
        l.on_event(
            &REvent::Transcript {
                assistant: true,
                text: "¡Hola!".into(),
            },
            at(t0, 1100),
        );
        assert_eq!(
            l.latency_ms,
            Some(700),
            "desde el último fragmento de la persona"
        );
        l.on_event(
            &REvent::Transcript {
                assistant: true,
                text: " ¿Qué tal?".into(),
            },
            at(t0, 1500),
        );
        assert_eq!(
            l.latency_ms,
            Some(700),
            "los fragmentos siguientes de la misma respuesta no la cambian"
        );
    }

    #[test]
    fn tokens_and_cost_use_the_catalog_prices_instead_of_constants() {
        let mut l = Live::default();
        l.on_event(
            &REvent::Pricing {
                input: Some(2.0),
                output: Some(4.0),
            },
            Instant::now(),
        );
        l.on_event(
            &REvent::Usage {
                input: 1_000_000,
                output: 500_000,
            },
            Instant::now(),
        );
        assert_eq!(l.total_tokens(), 1_500_000);
        assert!((l.cost_usd() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn milestones_are_capped_and_specialists_counted_once() {
        let (mut l, t0) = (Live::default(), Instant::now());
        for i in 0..55 {
            l.on_event(
                &REvent::Narration {
                    kind: "tool_call".into(),
                    label: format!("paso {i}"),
                    detail: None,
                    agent: if i % 2 == 0 {
                        "web".into()
                    } else {
                        "codigo".into()
                    },
                },
                t0,
            );
        }
        assert_eq!(l.milestones.len(), 40);
        assert_eq!(l.milestones.last().unwrap().label, "paso 54");
        assert_eq!(l.specialists.len(), 2);
    }

    #[test]
    fn a_new_call_clears_telemetry_but_keeps_the_preferences() {
        let (mut l, t0) = (Live::default(), Instant::now());
        l.voice = 3;
        l.lang = 5;
        l.scene_loading = true;
        l.scene_retry = 7;
        l.on_event(&REvent::Ready { model: "m".into() }, t0);
        l.on_event(
            &REvent::Usage {
                input: 5,
                output: 5,
            },
            t0,
        );
        l.reset();
        assert_eq!(
            (l.voice, l.lang, l.total_tokens(), l.started.is_none()),
            (3, 5, 0, true)
        );
        assert!(l.scene_loading);
        assert_eq!(
            l.scene_retry, 7,
            "colgar no reinicia la preparación gráfica"
        );
    }

    #[test]
    fn labels_and_time_read_like_the_webs() {
        let (mut l, t0) = (Live::default(), Instant::now());
        assert_eq!(Bia::Speaking.label("Pepito"), "Pepito habla");
        assert_eq!(Bia::Idle.label("Pepito"), "En reposo");
        assert_eq!(l.elapsed_label(t0), "—");
        l.on_event(&REvent::Ready { model: "m".into() }, t0);
        assert_eq!(l.elapsed_label(at(t0, 75_000)), "01:15");
        assert_eq!(short_model("gemini-3.8-live"), "3.8-live");
    }

    #[test]
    fn the_voice_level_rises_with_speech_and_falls_by_itself() {
        let (mut l, t0) = (Live::default(), Instant::now());
        l.on_event(
            &REvent::Transcript {
                assistant: true,
                text: "x".into(),
            },
            t0,
        );
        assert_eq!(l.level, 1.0);
        for _ in 0..60 {
            l.step(1.0 / 60.0);
        }
        assert!(l.level < 0.5 && l.level >= 0.0);
    }

    #[test]
    fn the_face_follows_the_voice_blinks_and_smiles_at_rest() {
        let mut l = Live::default();
        let rest = l.face();
        assert_eq!((rest.jaw, rest.aa), (0.0, 0.0), "callada, la boca cerrada");
        assert!(rest.smile > 0.1, "sonríe en reposo");
        l.level = 1.0;
        let talking = l.face();
        assert!(
            talking.jaw > 0.8 && talking.smile < 0.01,
            "al hablar abre la boca y deja de sonreír"
        );
        l.t = 0.07;
        assert!(
            l.face().blink > 0.9,
            "en mitad del parpadeo los ojos están casi cerrados"
        );
        l.t = 2.0;
        assert_eq!(l.face().blink, 0.0, "el resto del tiempo, abiertos");
    }

    #[test]
    fn the_view_turns_pans_and_zooms_within_the_webs_limits_and_resets() {
        let mut l = Live {
            viewport: (800.0, 600.0),
            ..Default::default()
        };
        assert!(l.avatar_model, "el modelo 3D es el avatar por defecto");
        l.drag_yaw(60.0);
        assert!((l.yaw - 0.6).abs() < 1e-5);
        l.drag_yaw(10_000.0);
        assert_eq!(l.yaw, 1.2, "tope a cada lado");
        for _ in 0..480 {
            l.step(1.0 / 60.0);
        }
        assert_eq!(
            l.yaw, 1.2,
            "ya no vuelve solo: lo deja donde lo pone la persona"
        );
        // Arrastrar a la derecha mueve la cámara a la izquierda (el contenido sigue al puntero); y con tope.
        l.pan_by(100.0, 0.0);
        assert!(l.pan[0] < 0.0 && l.pan[0] > -4.5, "{:?}", l.pan);
        l.pan_by(1.0e6, -1.0e6);
        assert_eq!(l.pan, [-4.5, -2.8], "topes de la web");
        // Rueda: acerca hasta 2,4 y aleja hasta 13.
        for _ in 0..100 {
            l.zoom_by(3.0);
        }
        assert_eq!(l.dist, 2.4);
        for _ in 0..100 {
            l.zoom_by(-3.0);
        }
        assert_eq!(l.dist, 13.0);
        l.reset_view();
        assert_eq!((l.pan, l.dist, l.yaw), ([0.0; 2], 6.2, 0.0));
    }

    #[test]
    fn with_real_audio_the_mouth_follows_it_and_closes_when_it_stops() {
        let mut l = Live::default();
        let t0 = Instant::now();
        // Sin audio todavía: la aproximación por palabras (la boca no se mueve si nadie habla).
        assert_eq!(l.face().jaw, 0.0);
        l.on_event(
            &REvent::Mouth {
                level: 0.8,
                bright: 0.02,
                delay_ms: 0,
            },
            t0,
        );
        for _ in 0..12 {
            l.step(1.0 / 60.0);
        }
        let open = l.face();
        assert!(
            open.jaw > 0.55 && open.o > open.e,
            "abre con la voz grave: {open:?}"
        );
        // Deja de llegar audio: en unos instantes la boca se cierra.
        for _ in 0..60 {
            l.step(1.0 / 60.0);
        }
        assert!(l.face().jaw < 0.05, "cerrada sin audio: {}", l.face().jaw);
        // Y una nueva llamada empieza de cero.
        l.reset();
        assert!(!l.has_audio && l.mouth == Sound::default());
    }

    #[test]
    fn the_mouth_opens_fast_and_closes_slower() {
        let mut up = Live::default();
        up.on_event(
            &REvent::Mouth {
                level: 1.0,
                bright: 0.0,
                delay_ms: 0,
            },
            Instant::now(),
        );
        up.step(0.0); // hora de que suene
        up.step(0.04);
        let rise = up.mouth.level;
        let mut down = Live {
            mouth: Sound {
                level: 1.0,
                bright: 0.0,
            },
            ..Default::default()
        };
        down.step(0.04);
        let fall = 1.0 - down.mouth.level;
        assert!(rise > 0.55, "ataque rápido: {rise}");
        assert!(
            fall < rise,
            "la caída es más lenta que la subida ({fall} < {rise})"
        );
    }

    #[test]
    fn the_mouth_waits_for_the_audio_that_is_still_queued_in_the_speaker() {
        let mut l = Live::default();
        l.on_event(
            &REvent::Mouth {
                level: 0.9,
                bright: 0.0,
                delay_ms: 300,
            },
            Instant::now(),
        );
        for _ in 0..12 {
            l.step(1.0 / 60.0); // 200 ms: aún no suena
        }
        assert!(
            l.face().jaw < 0.05,
            "no se abre antes de que suene: {}",
            l.face().jaw
        );
        for _ in 0..12 {
            l.step(1.0 / 60.0); // 400 ms en total: ya suena
        }
        assert!(
            l.face().jaw > 0.4,
            "se abre cuando le toca: {}",
            l.face().jaw
        );
    }
}
