//! Sonido de los eventos de la oficina: dos tonos senoidales por fase. Síntesis pura (PCM16 mono a 24 kHz,
//! lo que espera el reproductor nativo) y seguimiento de los eventos nuevos.
//! Frecuencias y envolvente de `state/sound.ts` de la UI web.

use std::collections::HashSet;

use super::graph::Graph;

pub const RATE: u32 = 24_000;
/// Segundos que dura el sonido completo.
pub const TOTAL_SECONDS: f32 = 0.34;
const FLOOR: f32 = 0.0001;
const PEAK: f32 = 0.045;
const ATTACK: f32 = 0.018;
/// Cada tono dura 0,2 s; el segundo empieza 0,12 s después del primero.
const TONE_SECONDS: f32 = 0.2;
const SECOND_AT: f32 = 0.12;

/// Frecuencias (Hz) de los dos tonos de cada fase. Las fases desconocidas no suenan.
pub fn tones(phase: &str) -> Option<(f32, f32)> {
    Some(match phase {
        "delegated" => (520.0, 680.0),
        "review_passed" => (720.0, 960.0),
        "review_failed" => (260.0, 190.0),
        "completed" => (660.0, 880.0),
        "failed" => (220.0, 150.0),
        "aborted" => (280.0, 210.0),
        "blocked" => (240.0, 180.0),
        _ => return None,
    })
}

/// Ganancia en el instante `t`: sube de 0,0001 a 0,045 en 18 ms y baja a 0,0001 a los 0,34 s, ambas rampas exponenciales.
pub fn gain(t: f32) -> f32 {
    if t < 0.0 || t >= TOTAL_SECONDS {
        return 0.0;
    }
    let (from, to, k) = if t < ATTACK {
        (FLOOR, PEAK, t / ATTACK)
    } else {
        (PEAK, FLOOR, (t - ATTACK) / (TOTAL_SECONDS - ATTACK))
    };
    from * (to / from).powf(k)
}

/// El sonido de una fase como PCM16 little-endian mono a 24 kHz, o `None` si la fase no suena.
pub fn render(phase: &str) -> Option<Vec<u8>> {
    let (f1, f2) = tones(phase)?;
    let n = (TOTAL_SECONDS * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let mut s = 0.0;
        if t < TONE_SECONDS {
            s += (std::f32::consts::TAU * f1 * t).sin();
        }
        if (SECOND_AT..SECOND_AT + TONE_SECONDS).contains(&t) {
            s += (std::f32::consts::TAU * f2 * (t - SECOND_AT)).sin();
        }
        out.extend_from_slice(
            &(((s * gain(t)).clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes(),
        );
    }
    Some(out)
}

/// Fases de los eventos nuevos. Los que ya existían al abrir son historia y no suenan.
#[derive(Debug, Default)]
pub struct SoundCues {
    seen: HashSet<String>,
    primed: bool,
}

impl SoundCues {
    pub fn update(&mut self, graph: &Graph) -> Vec<String> {
        if !self.primed {
            self.seen
                .extend(graph.work_events.iter().map(|e| e.event_id.clone()));
            self.primed = true;
            return Vec::new();
        }
        graph
            .work_events
            .iter()
            .filter(|e| self.seen.insert(e.event_id.clone()))
            .map(|e| e.phase.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn samples(pcm: &[u8]) -> Vec<i16> {
        pcm.chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    /// Frecuencia estimada contando cruces por cero ascendentes en un tramo.
    fn freq(s: &[i16], from: f32, to: f32) -> f32 {
        let (a, b) = ((from * RATE as f32) as usize, (to * RATE as f32) as usize);
        let crossings = s[a..b].windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        crossings as f32 / (to - from)
    }

    #[test]
    fn every_known_phase_has_two_tones_and_unknown_ones_are_silent() {
        for p in [
            "delegated",
            "review_passed",
            "review_failed",
            "completed",
            "failed",
            "aborted",
            "blocked",
        ] {
            assert!(tones(p).is_some() && render(p).is_some(), "{p}");
        }
        assert_eq!(tones("delegated"), Some((520.0, 680.0)));
        assert_eq!(tones("failed"), Some((220.0, 150.0)));
        assert_eq!(tones("raro"), None);
        assert!(render("raro").is_none());
    }

    #[test]
    fn good_news_rises_and_bad_news_falls() {
        for p in ["delegated", "review_passed", "completed"] {
            let (a, b) = tones(p).unwrap();
            assert!(b > a, "{p} sube");
        }
        for p in ["review_failed", "failed", "aborted", "blocked"] {
            let (a, b) = tones(p).unwrap();
            assert!(b < a, "{p} baja");
        }
    }

    #[test]
    fn the_envelope_attacks_in_18_ms_then_decays_to_silence_by_340_ms() {
        assert_eq!(gain(-0.1), 0.0);
        assert!((gain(0.0) - 0.0001).abs() < 1e-6);
        assert!((gain(0.018) - 0.045).abs() < 1e-5, "pico a los 18 ms");
        assert!(
            gain(0.1) < gain(0.05) && gain(0.05) < gain(0.018),
            "luego baja"
        );
        assert!(gain(0.339) < 0.0002, "casi silencio al final");
        assert_eq!(gain(0.34), 0.0);
        assert!(
            gain(0.009) > gain(0.0) && gain(0.009) < gain(0.018),
            "la subida es continua"
        );
    }

    #[test]
    fn the_pcm_has_the_right_length_and_never_clips() {
        let pcm = render("completed").unwrap();
        assert_eq!(pcm.len(), (0.34 * 24_000.0) as usize * 2);
        let s = samples(&pcm);
        let peak = s.iter().map(|v| v.unsigned_abs()).max().unwrap();
        // Dos tonos solapados suman como mucho 2 × 0,045 de la escala completa.
        assert!(
            peak > 600 && peak < (0.092 * i16::MAX as f32) as u16,
            "pico moderado: {peak}"
        );
        assert!(s[s.len() - 1].unsigned_abs() < 5, "termina en silencio");
    }

    #[test]
    fn the_first_tone_comes_first_and_the_second_follows_at_its_own_pitch() {
        let s = samples(&render("completed").unwrap()); // 660 y 880 Hz
        let early = freq(&s, 0.02, 0.1);
        assert!((early - 660.0).abs() < 60.0, "primer tono: {early}");
        // Entre 0,2 y 0,32 s solo suena el segundo tono.
        let late = freq(&s, 0.21, 0.32);
        assert!((late - 880.0).abs() < 90.0, "segundo tono: {late}");
        let alert = samples(&render("failed").unwrap()); // 220 y 150 Hz
        assert!(
            (freq(&alert, 0.02, 0.1) - 220.0).abs() < 40.0
                && (freq(&alert, 0.21, 0.32) - 150.0).abs() < 40.0
        );
    }

    #[test]
    fn only_new_events_make_a_sound() {
        let mut g = Graph::default();
        let mut c = SoundCues::default();
        g.apply(
            "canvas:work_event",
            &json!({"eventId": "old", "phase": "completed", "actorId": "a"}),
            None,
            0.0,
        );
        assert!(c.update(&g).is_empty(), "la historia al abrir no suena");
        g.apply(
            "canvas:work_event",
            &json!({"eventId": "n1", "phase": "delegated", "actorId": "a"}),
            None,
            0.0,
        );
        g.apply(
            "canvas:work_event",
            &json!({"eventId": "n2", "phase": "failed", "actorId": "a"}),
            None,
            0.0,
        );
        assert_eq!(c.update(&g), ["delegated", "failed"]);
        assert!(c.update(&g).is_empty(), "una vez cada uno");
    }
}
