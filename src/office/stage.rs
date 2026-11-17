//! Compositor de Rune para la oficina: cuando la pantalla activa es la oficina, avanza la simulación,
//! dibuja la escena 3D en su propia textura y la pinta dentro del hueco de la interfaz; si no, el
//! fotograma normal de Rune.

use rune::{Compositor, Frame};

use super::gpu::OfficeGpu;
use super::scene;
use crate::{Screen, State};

/// Marca del hueco (`El::effect`) y clave del elemento que lo contiene.
pub const OFFICE_MARK: u32 = 11;
pub const SLOT_KEY: &str = "office3d";

#[derive(Default)]
pub struct OfficeStage {
    gpu: Option<OfficeGpu>,
    /// Si el último fotograma mostraba la oficina: mantiene el reloj de Rune andando mientras se mueve.
    visible: bool,
}

/// Tamaño en píxeles de la textura de la escena para un hueco de `rect` puntos.
pub fn target_size(rect: [f32; 4], scale: f32, high: bool) -> (u32, u32) {
    // En calidad alta se limita a 1,75 píxeles por punto; en baja, a uno (la UI web hace lo mismo).
    let dpr = if high { scale.clamp(1.0, 1.75) } else { 1.0 };
    (
        ((rect[2] * dpr).round() as u32).max(1),
        ((rect[3] * dpr).round() as u32).max(1),
    )
}

impl OfficeStage {
    /// La oficina deja de animar cuando otra pantalla con escenario toma el relevo.
    pub fn set_visible(&mut self, v: bool) {
        self.visible = v;
    }
}

impl Compositor<State> for OfficeStage {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        self.visible = f.rt.state.screen == Screen::Office;
        if !self.visible {
            f.draw_ui();
            return;
        }

        let rect = f.rt.presented(SLOT_KEY).map(|(r, _)| r).unwrap_or([0.0; 4]);
        let device = f.device;
        let key = f.device_key();
        if self.gpu.as_ref().and_then(|g| g.key) != Some(key) {
            self.gpu = Some(OfficeGpu::new(device, Some(key)));
        }

        if rect[2] >= 1.0 && rect[3] >= 1.0 {
            f.rt.state.office.viewport = (rect[2], rect[3]);
            // Un fotograma lento no debe lanzar a las abejas: se limita el paso.
            f.rt.state.office.step((f.dt as f32).min(0.1));
            f.rt.state.office.play_pending_sounds();
            let o = &f.rt.state.office;
            let frame = scene::build(o);
            let size = target_size(rect, f.rt.env.scale, o.high_quality);
            let dpr = size.0 as f32 / rect[2].max(1.0);
            self.gpu.as_mut().unwrap().render(
                device,
                f.queue,
                f.encoder,
                &frame,
                &o.camera,
                o.t,
                size,
                dpr,
                o.high_quality,
            );
        }

        let clear = f.rt.clear_color();
        f.ui.prepare(
            device,
            f.queue,
            &mut f.rt.text.atlas,
            &f.rt.instances,
            f.size,
        );
        let gpu = self.gpu.as_mut().unwrap();
        f.ui.draw_with(f.encoder, f.target, clear, &mut |pass, site| {
            gpu.blit(device, pass, site, rect)
        });
    }

    fn animating(&self) -> bool {
        self.visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_texture_follows_the_slot_and_the_quality_cap() {
        assert_eq!(target_size([0.0, 0.0, 800.0, 500.0], 1.0, true), (800, 500));
        assert_eq!(
            target_size([0.0, 0.0, 800.0, 500.0], 2.0, true),
            (1400, 875),
            "tope de 1,75 px por punto"
        );
        assert_eq!(
            target_size([0.0, 0.0, 800.0, 500.0], 2.0, false),
            (800, 500),
            "en calidad baja, uno"
        );
        assert_eq!(
            target_size([0.0, 0.0, 0.0, 0.0], 1.0, true),
            (1, 1),
            "nunca cero"
        );
    }
}
