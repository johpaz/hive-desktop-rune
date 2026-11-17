//! Compositor de HiveLive: dibuja el escenario en su textura y lo pinta dentro del hueco de la interfaz.

use rune::{Compositor, Frame};

use super::gpu::{LiveGpu, Params};
use super::loading::Background;
use crate::{Screen, State};

/// Marca del hueco (`El::effect`) y clave del elemento que lo contiene.
pub const LIVE_MARK: u32 = 12;
pub const SLOT_KEY: &str = "live-stage";

#[derive(Default)]
pub struct LiveStage {
    gpu: Option<LiveGpu>,
    pending: Option<(rune::compose::DeviceKey, Background<LiveGpu>)>,
    failed: Option<rune::compose::DeviceKey>,
    retry: u64,
}

impl LiveStage {
    pub fn render(&mut self, f: &mut Frame<'_, State>) {
        debug_assert_eq!(f.rt.state.screen, Screen::Live);
        let rect = f.rt.presented(SLOT_KEY).map(|(r, _)| r).unwrap_or([0.0; 4]);
        let device = f.device;
        let key = f.device_key();
        if self.retry != f.rt.state.live.scene_retry {
            self.retry = f.rt.state.live.scene_retry;
            self.failed = None;
        }
        if self.gpu.as_ref().and_then(|g| g.key) != Some(key) {
            self.gpu = None;
            if self
                .pending
                .as_ref()
                .is_some_and(|(pending, _)| *pending != key)
            {
                self.pending = None;
            }
            if self.pending.is_none() && self.failed != Some(key) {
                // Single-frame exports must wait for the complete scene. Interactive navigation
                // paints the controls first and prepares all graphics on a worker.
                let export = crate::is_headless(&std::env::args().collect::<Vec<_>>());
                if export {
                    self.gpu = Some(LiveGpu::new(device, f.queue, Some(key)));
                } else {
                    let device = device.clone();
                    let queue = f.queue.clone();
                    self.pending = Some((
                        key,
                        Background::start(move || {
                            let start = std::time::Instant::now();
                            let gpu = LiveGpu::new(&device, &queue, Some(key));
                            eprintln!(
                                "[HiveLive] Escenario gráfico preparado en {} ms (segundo plano)",
                                start.elapsed().as_millis()
                            );
                            gpu
                        }),
                    ));
                }
            }
        }
        if let Some(result) = self.pending.as_ref().and_then(|(_, job)| job.poll()) {
            self.pending = None;
            match result {
                Ok(gpu) => {
                    self.gpu = Some(gpu);
                    f.rt.state.live.scene_error = None;
                }
                Err(error) => {
                    self.failed = Some(key);
                    f.rt.state.live.scene_error = Some(error);
                }
            }
            f.rt.invalidate();
        }
        let loading = self.pending.is_some();
        if f.rt.state.live.scene_loading != loading {
            f.rt.state.live.scene_loading = loading;
            f.rt.invalidate();
        }
        if self.gpu.is_none() {
            let clear = f.rt.clear_color();
            f.ui.prepare(
                device,
                f.queue,
                &mut f.rt.text.atlas,
                &f.rt.instances,
                f.size,
            );
            f.ui.draw(f.encoder, f.target, clear);
            return;
        }
        if rect[2] >= 1.0 && rect[3] >= 1.0 {
            let now = std::time::Instant::now();
            let (voice_on, voice_live) = (f.rt.state.voice_on, f.rt.state.voice_live);
            let live = &mut f.rt.state.live;
            live.viewport = (rect[2], rect[3]);
            live.step((f.dt as f32).min(0.1));
            let bia = live.bia(voice_on, voice_live, now);
            let scale = f.rt.env.scale.clamp(1.0, 1.75);
            let size = (
                ((rect[2] * scale).round() as u32).max(1),
                ((rect[3] * scale).round() as u32).max(1),
            );
            let params = Params {
                t: live.t,
                level: live.level,
                tint: bia.tint(),
                dpr: scale,
                sway: 0.0,
                model: live.avatar_model,
                yaw: live.yaw,
                pan: live.pan,
                dist: live.dist,
                face: live.face(),
            };
            self.gpu
                .as_mut()
                .unwrap()
                .render(device, f.queue, f.encoder, size, params);
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
}

/// Elige el compositor según la pantalla: la oficina y HiveLive tienen escenario propio; el resto, la interfaz normal.
#[derive(Default)]
pub struct Stages {
    pub office: crate::office::stage::OfficeStage,
    pub live: LiveStage,
}

impl Compositor<State> for Stages {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        // Anota el tamaño de la ventana (en puntos) para que la interfaz se adapte; si cambia, se vuelve a construir la vista.
        let sc = f.rt.env.scale.max(0.5);
        let win = (f.size.0 as f32 / sc, f.size.1 as f32 / sc);
        if (win.0 - f.rt.state.win.0).abs() >= 1.0 || (win.1 - f.rt.state.win.1).abs() >= 1.0 {
            f.rt.state.win = win;
            f.rt.invalidate();
        }
        let live = f.rt.state.screen == Screen::Live;
        LIVE_ACTIVE.store(live, std::sync::atomic::Ordering::Relaxed);
        if live {
            self.office.set_visible(false);
            self.live.render(f);
        } else {
            self.office.render(f);
        }
    }

    fn animating(&self) -> bool {
        self.office.animating() || self.live_visible()
    }
}

impl Stages {
    fn live_visible(&self) -> bool {
        (self.live.gpu.is_some() || self.live.pending.is_some())
            && LIVE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Si el último fotograma era HiveLive (mantiene el reloj de Rune andando para el polen).
pub static LIVE_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    #[ignore = "real GPU timing probe"]
    fn live_navigation_first_frame_is_nonblocking_and_reuses_graphics() {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let format = rune::render::OFFSCREEN_FORMAT;
        let mut renderer = rune::render::Renderer::new(&device, format);
        let mut state = crate::State::offline();
        crate::demo_live(&mut state);
        state.win = (800.0, 600.0);
        state.live.viewport = state.stage_size();
        let mut rt = rune::Runtime::new(
            state,
            crate::view,
            (800.0, 600.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.reduced_motion = true;
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live navigation probe"),
            size: wgpu::Extent3d {
                width: 800,
                height: 600,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut stage = LiveStage::default();
        let draw = |stage: &mut LiveStage,
                    rt: &mut rune::Runtime<State>,
                    renderer: &mut rune::render::Renderer| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let now = Instant::now();
            stage.render(&mut Frame {
                device: &device,
                queue: &queue,
                encoder: &mut encoder,
                target: &view,
                format,
                size: (800, 600),
                dt: 1.0 / 60.0,
                update_ms: 0.0,
                rt,
                ui: renderer,
            });
            queue.submit([encoder.finish()]);
            now.elapsed()
        };
        let first = draw(&mut stage, &mut rt, &mut renderer);
        eprintln!(
            "HiveLive primer frame de navegación (incluye envío GPU): {:.2} ms",
            first.as_secs_f64() * 1000.0
        );
        assert!(
            first < Duration::from_millis(500),
            "La escena bloqueó el primer frame: {first:?}"
        );
        assert!(stage.pending.is_some());
        assert!(rt.state.live.scene_loading);
        assert!(rt.find("voice").is_some());
        let deadline = Instant::now() + Duration::from_secs(30);
        while stage.gpu.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            rt.frame(1.0 / 60.0);
            draw(&mut stage, &mut rt, &mut renderer);
        }
        assert!(
            stage.gpu.is_some(),
            "No se entregaron los gráficos: {:?}",
            rt.state.live.scene_error
        );
        assert!(!rt.state.live.scene_loading);
        let again = draw(&mut stage, &mut rt, &mut renderer);
        assert!(
            stage.pending.is_none(),
            "Volver a la vista no debe reconstruirla"
        );
        eprintln!(
            "HiveLive frame con gráficos reutilizados: {} ms",
            again.as_millis()
        );
    }
}
