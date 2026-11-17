//! Audio nativo para la voz: micrófono y altavoz con `hive-media`, sin el puente `invoke`.
//! Sustituye a `media_mic_*` y `media_player_*` de `apps/hive-desktop/src-tauri/src/media.rs`:
//! aquí no hay IPC ni JSON, los bloques PCM pasan directo del hilo de audio a quien los use.

use hive_media::{Capture, Direction, EchoCanceller, MediaError, Player};

pub const OUTPUT_RATE: u32 = 24_000;

/// Micrófono + altavoz que comparten cancelador: el eco se quita con lo que suena, así que
/// las dos mitades tienen que hablar con el mismo procesador.
#[derive(Default)]
pub struct Voice {
    echo: EchoCanceller,
    capture: Option<Capture>,
    player: Option<Player>,
}

pub fn devices(direction: Direction) -> Result<Vec<hive_media::AudioDevice>, MediaError> {
    hive_media::list_devices(direction)
}

impl Voice {
    /// Abre el micrófono (`None` = el del sistema); `on_frame` recibe PCM16 mono a 16 kHz cada
    /// ~40 ms, en el hilo de audio: que no bloquee. Abrir de nuevo cambia de micrófono.
    pub fn start_mic(
        &mut self,
        device_id: Option<&str>,
        on_frame: impl FnMut(Vec<u8>) + Send + 'static,
    ) -> Result<(), MediaError> {
        // Antes de abrir el nuevo: dos flujos del mismo aparato dan problemas.
        drop(self.capture.take());
        self.capture = Some(Capture::start(
            device_id,
            Some(self.echo.clone()),
            on_frame,
        )?);
        Ok(())
    }

    pub fn stop_mic(&mut self) {
        drop(self.capture.take());
    }

    pub fn mic_level(&self) -> f32 {
        self.capture.as_ref().map_or(0.0, Capture::level)
    }

    pub fn start_player(&mut self, device_id: Option<&str>) -> Result<(), MediaError> {
        drop(self.player.take());
        self.player = Some(Player::start(device_id, Some(self.echo.clone()))?);
        Ok(())
    }

    /// Encola PCM16 mono a 24 kHz.
    pub fn enqueue(&self, pcm: &[u8]) {
        if let Some(p) = &self.player {
            p.enqueue(pcm);
        }
    }

    /// Barge-in: descarta lo pendiente.
    pub fn interrupt(&self) {
        if let Some(p) = &self.player {
            p.interrupt();
        }
    }

    pub fn queued_ms(&self) -> u64 {
        self.player.as_ref().map_or(0, Player::queued_ms)
    }

    pub fn stop_player(&mut self) {
        drop(self.player.take());
    }
}

/// Dos notas breves (660 y 990 Hz) como PCM16 a 24 kHz, las mismas de la prueba de la UI web.
pub fn test_tone() -> Vec<u8> {
    let notes = [660.0f32, 990.0];
    let per_note = (OUTPUT_RATE as f32 * 0.17) as usize;
    let gap = (OUTPUT_RATE as f32 * 0.01) as usize;
    let mut pcm = vec![0i16; notes.len() * (per_note + gap)];
    for (n, hz) in notes.iter().enumerate() {
        for i in 0..per_note {
            // Rampa en los extremos: un tono que arranca o corta en seco chasquea.
            let envelope = (i as f32 / 480.0)
                .min((per_note - i) as f32 / 480.0)
                .min(1.0);
            let s = (2.0 * std::f32::consts::PI * hz * i as f32 / OUTPUT_RATE as f32).sin()
                * 0.16
                * envelope;
            pcm[n * (per_note + gap) + i] = (s * 0x7fff as f32).round() as i16;
        }
    }
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_has_two_notes_of_pcm16() {
        let bytes = test_tone();
        assert_eq!(bytes.len() % 2, 0);
        let samples = bytes.len() / 2;
        assert_eq!(
            samples,
            2 * ((OUTPUT_RATE as f32 * 0.17) as usize + (OUTPUT_RATE as f32 * 0.01) as usize)
        );
        // Arranca y termina en silencio (rampa), y no se sale de rango.
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 0);
        assert!(
            bytes
                .chunks(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
                .max()
                .unwrap()
                < 6_000
        );
    }
}
