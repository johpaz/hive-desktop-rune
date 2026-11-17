//! Sincronía labial a partir del audio real de la voz de BIA (PCM16 mono a 24 kHz).
//!
//! No es el reconocimiento de visemas de HeadAudio que usa la web: de cada trozo de audio sale cuánto suena
//! (el nivel, que abre la mandíbula) y qué tan «brillante» es (la proporción de energía en agudos, que separa las
//! vocales abiertas y redondas, graves, de los sonidos de boca estirada como la «e», la «i» o la «s»).

use super::model::Face;

/// Frecuencia de muestreo del audio del modelo.
pub const RATE: f32 = 24_000.0;

/// Lo que se sabe de un trozo de audio.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sound {
    /// Nivel de 0 a 1 (el RMS escalado: la voz normal ronda 0,05–0,2).
    pub level: f32,
    /// Proporción de energía en agudos, de 0 (todo graves) a 1 (todo agudos).
    pub bright: f32,
}

/// Analiza un trozo de PCM16 little-endian. Un trozo vacío o de un solo byte es silencio.
pub fn analyze(pcm: &[u8]) -> Sound {
    let samples: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0)
        .collect();
    if samples.len() < 2 {
        return Sound::default();
    }
    // Graves: un filtro de un polo a ~1 kHz; agudos: lo que queda (la señal menos esos graves), que equivale a un
    // paso alto suave. Es burdo a propósito: solo hace falta distinguir «ooo» de «sss».
    let a = 1.0 - (-2.0 * std::f32::consts::PI * 1_000.0 / RATE).exp();
    let (mut lp, mut total, mut low, mut high) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for &x in &samples {
        lp += a * (x - lp);
        let h = x - lp;
        total += x * x;
        low += lp * lp;
        high += h * h;
    }
    let n = samples.len() as f32;
    let rms = (total / n).sqrt();
    let bright = if low + high > 1e-12 {
        high / (low + high)
    } else {
        0.0
    };
    Sound {
        level: (rms * 5.0).clamp(0.0, 1.0),
        bright,
    }
}

/// El gesto de la cara para un sonido: la mandíbula sigue el nivel y la vocal sale del timbre.
/// `smile` es la sonrisa de reposo, que se aparta al hablar.
pub fn face(sound: Sound, smile: f32) -> Face {
    let l = sound.level;
    // Graves → boca redonda (o, u) y muy abierta (a); agudos → boca estirada (e).
    let dark = (1.0 - sound.bright * 2.0).clamp(0.0, 1.0);
    let wide = (sound.bright * 2.0 - 0.3).clamp(0.0, 1.0);
    Face {
        jaw: (l * 0.95).min(1.0),
        aa: l * 0.6 * (1.0 - wide) * (0.4 + 0.6 * dark),
        o: l * 0.45 * dark,
        e: l * 0.5 * wide,
        u: l * 0.3 * dark * (1.0 - l),
        smile: smile * (1.0 - l),
        blink: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, amp: f32, n: usize) -> Vec<u8> {
        (0..n)
            .flat_map(|i| {
                (((i as f32 / RATE * hz * std::f32::consts::TAU).sin() * amp * 32767.0) as i16)
                    .to_le_bytes()
            })
            .collect()
    }

    /// Ruido determinista y blanco: sus agudos pesan mucho más que los de un tono grave.
    fn hiss(amp: f32, n: usize) -> Vec<u8> {
        let mut x: u32 = 12345;
        (0..n)
            .flat_map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                let v = ((x >> 16) as f32 / 32768.0 - 1.0) * amp;
                ((v * 32767.0) as i16).to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn silence_and_scraps_are_quiet() {
        assert_eq!(analyze(&[]), Sound::default());
        assert_eq!(analyze(&[1]), Sound::default());
        assert_eq!(analyze(&vec![0u8; 960]), Sound::default());
        assert!(
            analyze(&[1, 0, 2]).level >= 0.0,
            "un byte suelto al final se ignora"
        );
    }

    #[test]
    fn louder_audio_gives_a_higher_level_and_it_saturates_at_one() {
        let quiet = analyze(&tone(200.0, 0.02, 960)).level;
        let speech = analyze(&tone(200.0, 0.15, 960)).level;
        let shout = analyze(&tone(200.0, 0.9, 960)).level;
        assert!(
            quiet < speech && speech < shout,
            "{quiet} < {speech} < {shout}"
        );
        assert!(quiet < 0.1 && shout <= 1.0 && shout > 0.9);
    }

    #[test]
    fn a_low_voice_is_dark_and_a_hiss_is_bright() {
        let low = analyze(&tone(150.0, 0.3, 2400));
        let mid = analyze(&tone(700.0, 0.3, 2400));
        let hiss = analyze(&hiss(0.3, 2400));
        assert!(
            low.bright < 0.1,
            "un tono grave casi no tiene agudos: {}",
            low.bright
        );
        assert!(
            hiss.bright > 0.5,
            "el siseo es casi todo agudos: {}",
            hiss.bright
        );
        assert!(
            low.bright < mid.bright && mid.bright < hiss.bright,
            "{} < {} < {}",
            low.bright,
            mid.bright,
            hiss.bright
        );
    }

    #[test]
    fn the_mouth_opens_with_the_level_and_picks_round_or_wide_by_timbre() {
        let silent = face(
            Sound {
                level: 0.0,
                bright: 0.0,
            },
            0.15,
        );
        assert_eq!(
            (silent.jaw, silent.aa, silent.o, silent.e),
            (0.0, 0.0, 0.0, 0.0)
        );
        assert!((silent.smile - 0.15).abs() < 1e-6, "callada, sonríe");
        let ooo = face(
            Sound {
                level: 0.7,
                bright: 0.02,
            },
            0.15,
        );
        let eee = face(
            Sound {
                level: 0.7,
                bright: 0.6,
            },
            0.15,
        );
        assert!(ooo.jaw > 0.6 && eee.jaw > 0.6, "las dos abren la mandíbula");
        assert!(
            ooo.o > ooo.e && eee.e > eee.o,
            "graves → redonda, agudos → estirada"
        );
        assert!(ooo.smile < 0.06, "al hablar deja de sonreír");
        let wild = face(
            Sound {
                level: 1.0,
                bright: 1.0,
            },
            1.0,
        );
        for w in [wild.jaw, wild.aa, wild.o, wild.e, wild.u, wild.smile] {
            assert!(
                (0.0..=1.0).contains(&w),
                "los pesos se quedan entre 0 y 1: {w}"
            );
        }
    }
}
