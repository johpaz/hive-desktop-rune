//! Renderizador del escenario de HiveLive: panal, halo, retrato de BIA y polen, en una textura propia que
//! se pinta dentro del hueco de la interfaz (igual que la oficina 3D).

use std::collections::HashMap;

use super::model::Face;
use bytemuck::{Pod, Zeroable};
use rune::compose::DeviceKey;
use rune::render::EffectSite;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SHADER: &str = include_str!("live.wgsl");
const BLIT: &str = include_str!("../office/shaders/blit.wgsl");
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const PORTRAIT: &[u8] = include_bytes!("../../assets/bia.webp");
/// Partículas de polen en el aire.
pub const POLLEN: u32 = 140;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    res: [f32; 2],
    t: f32,
    level: f32,
    tint: [f32; 4],
    stage: [f32; 4],
}

/// Lo que cambia de un fotograma a otro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub t: f32,
    /// Nivel de la voz, de 0 a 1: hace respirar el halo y el polen.
    pub level: f32,
    /// Color del estado de BIA (sRGB 0–1).
    pub tint: [f32; 3],
    /// Densidad de píxeles de la textura respecto a los puntos de la interfaz.
    pub dpr: f32,
    /// Desplazamiento horizontal del retrato, en píxeles (el giro con el ratón).
    pub sway: f32,
    /// Dibujar el modelo 3D en vez del retrato.
    pub model: bool,
    /// Giro del modelo sobre su eje vertical (radianes) y su gesto.
    pub yaw: f32,
    /// La cámara del modelo 3D como en la web: el punto al que mira (desplazamiento) y la distancia (zoom).
    pub pan: [f32; 2],
    pub dist: f32,
    pub face: Face,
    /// El giro de cada hueso del esqueleto (ver `body`).
    pub pose: super::body::Pose,
}

struct Target {
    size: (u32, u32),
    #[cfg_attr(not(test), allow(dead_code))]
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
    blit_bind: wgpu::BindGroup,
}

pub struct LiveGpu {
    pub key: Option<DeviceKey>,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    backdrop: wgpu::RenderPipeline,
    portrait: wgpu::RenderPipeline,
    pollen: wgpu::RenderPipeline,
    aspect: f32,
    sampler: wgpu::Sampler,
    /// El modelo 3D de BIA (si el GLB se pudo leer).
    bia: Option<super::bia_gpu::BiaGpu>,
    target: Option<Target>,
    blit_module: wgpu::ShaderModule,
    blit_layout: wgpu::PipelineLayout,
    blit_bgl: wgpu::BindGroupLayout,
    blit_pipes: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl LiveGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, key: Option<DeviceKey>) -> LiveGpu {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        // El retrato: se decodifica una vez y se sube como textura sRGB.
        let (pw, ph, rgba) = decode_portrait();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bia"),
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pw * 4),
                rows_per_image: Some(ph),
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = tex.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        blend: Option<wgpu::BlendState>,
                        topology: wgpu::PrimitiveTopology| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // «Screen»: lo negro del retrato no tapa nada y sus luces se suman al fondo sin pasarse.
        let screen = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrc,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let backdrop = pipeline(
            "live backdrop",
            "vs_full",
            "fs_backdrop",
            None,
            wgpu::PrimitiveTopology::TriangleList,
        );
        let portrait = pipeline(
            "live portrait",
            "vs_full",
            "fs_portrait",
            Some(screen),
            wgpu::PrimitiveTopology::TriangleList,
        );
        let pollen = pipeline(
            "live pollen",
            "vs_pollen",
            "fs_pollen",
            Some(additive),
            wgpu::PrimitiveTopology::TriangleStrip,
        );

        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live blit"),
            bind_group_layouts: &[Some(&blit_bgl)],
            immediate_size: 0,
        });

        let bia = super::bia_gpu::BiaGpu::new(device, queue, FORMAT, DEPTH);
        LiveGpu {
            key,
            uniforms,
            bind,
            backdrop,
            portrait,
            pollen,
            aspect: pw as f32 / ph as f32,
            sampler,
            bia,
            target: None,
            blit_module,
            blit_layout,
            blit_bgl,
            blit_pipes: HashMap::new(),
        }
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live out"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live blit"),
            layout: &self.blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("live depth"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        self.target = Some(Target {
            size,
            tex,
            view,
            depth,
            blit_bind,
        });
    }

    /// Dibuja el escenario en `size` píxeles.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
        p: Params,
    ) {
        let size = (size.0.clamp(32, 3840), size.1.clamp(32, 2160));
        self.ensure_target(device, size);
        let u = Uniforms {
            res: [size.0 as f32, size.1 as f32],
            t: p.t,
            level: p.level.clamp(0.0, 1.0),
            tint: [
                p.tint[0],
                p.tint[1],
                p.tint[2],
                if crate::ui::theme::is_light() {
                    1.0
                } else {
                    0.0
                },
            ],
            // El retrato ocupa casi todo el alto del escenario.
            stage: [self.aspect, 0.98, p.dpr, p.sway],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        let use_model = p.model && self.bia.is_some();
        if let (true, Some(bia)) = (use_model, self.bia.as_mut()) {
            bia.update(queue, &p, size);
        }
        let target = self.target.as_ref().expect("textura de salida");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("live"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.backdrop);
        pass.draw(0..3, 0..1);
        if use_model {
            if let Some(bia) = &self.bia {
                bia.draw(&mut pass);
            }
        } else {
            pass.set_pipeline(&self.portrait);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.pollen);
        pass.draw(0..4, 0..POLLEN);
    }

    /// Pinta el resultado en el hueco de la interfaz.
    pub fn blit(
        &mut self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        site: &EffectSite<'_>,
        rect: [f32; 4],
    ) {
        let Some(target) = &self.target else { return };
        let pipe = self.blit_pipes.entry(site.format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("live blit"),
                layout: Some(&self.blit_layout),
                vertex: wgpu::VertexState {
                    module: &self.blit_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.blit_module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: site.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let s = site.scale;
        pass.set_viewport(
            rect[0] * s - site.origin[0],
            rect[1] * s - site.origin[1],
            rect[2] * s,
            rect[3] * s,
            0.0,
            1.0,
        );
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &target.blit_bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Lee de vuelta la imagen final (RGBA8, filas sin relleno). Solo para pruebas con GPU real.
    #[cfg(test)]
    pub fn read_out(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.target.as_ref()?;
        let (w, h) = t.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            out.extend_from_slice(&data[(y * row) as usize..(y * row + w * 4) as usize]);
        }
        Some((w, h, out))
    }
}

/// El retrato de BIA (el de la web) a RGBA8; si no se puede decodificar, un píxel negro (no tapa nada).
fn decode_portrait() -> (u32, u32, Vec<u8>) {
    match image::load_from_memory(PORTRAIT) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            (rgba.width(), rgba.height(), rgba.into_raw())
        }
        Err(_) => (1, 1, vec![0, 0, 0, 255]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_is_valid_wgsl() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("WGSL válido");
        let mut v = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        v.validate(&module).expect("el módulo valida");
    }

    #[test]
    fn the_uniform_block_matches_the_shader() {
        assert_eq!(std::mem::size_of::<Uniforms>(), 48);
    }

    #[test]
    fn the_portrait_decodes_with_the_webs_proportions() {
        let (w, h, px) = decode_portrait();
        assert_eq!((w, h), (768, 1152));
        assert_eq!(px.len(), (w * h * 4) as usize);
    }

    fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut LiveGpu,
        size: (u32, u32),
        p: Params,
    ) -> Vec<u8> {
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(device, queue, &mut enc, size, p);
        queue.submit([enc.finish()]);
        gpu.read_out(device, queue).expect("lectura").2
    }

    fn params(level: f32) -> Params {
        Params {
            t: 1.0,
            level,
            tint: [0.26, 0.83, 0.99],
            dpr: 1.0,
            sway: 0.0,
            model: false,
            yaw: 0.0,
            pan: [0.0; 2],
            dist: crate::live::bia_gpu::EYE[2],
            face: Default::default(),
            pose: Default::default(),
        }
    }

    fn lum(px: &[u8]) -> u64 {
        px.chunks(4)
            .map(|p| (p[0] as u64 * 2126 + p[1] as u64 * 7152 + p[2] as u64 * 722) / 10000)
            .sum()
    }

    #[test]
    fn the_stage_draws_the_honeycomb_bia_and_pollen_on_the_real_gpu() {
        let Some((device, queue)) = headless() else {
            eprintln!("(sin adaptador gráfico: prueba omitida)");
            return;
        };
        let mut gpu = LiveGpu::new(&device, &queue, None);
        let (w, h) = (900u32, 600u32);
        let px = render(&device, &queue, &mut gpu, (w, h), params(0.0));
        assert_eq!(px.len(), (w * h * 4) as usize);
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        // Todo es opaco y no es una imagen vacía.
        assert!(px.chunks(4).all(|p| p[3] == 255));
        assert!(
            lum(&px) > (w * h) as u64 * 6,
            "el escenario no está apagado"
        );
        // El panal es ámbar: hay píxeles cálidos (rojo > azul) repartidos por el fondo, no solo en la figura.
        let warm = |x0: u32, x1: u32| {
            (0..h)
                .step_by(3)
                .flat_map(|y| (x0..x1).step_by(3).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let p = at(x, y);
                    p[0] > 40 && p[0] as i32 > p[2] as i32 + 15
                })
                .count()
        };
        assert!(
            warm(0, 150) > 200,
            "el panal llega al borde izquierdo: {}",
            warm(0, 150)
        );
        // BIA está en el centro: la franja central es más luminosa que los márgenes (el panal solo no lo explica).
        let band = |x0: u32, x1: u32| {
            (0..h)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let p = at(x, y);
                    p[0] as u64 + p[1] as u64 + p[2] as u64
                })
                .sum::<u64>()
                / ((x1 - x0) as u64 * h as u64)
        };
        assert!(
            band(340, 560) > band(0, 120) + 6,
            "la figura se ve en el centro: {} frente a {}",
            band(340, 560),
            band(0, 120)
        );
    }

    #[test]
    fn the_voice_level_brightens_the_stage_and_time_moves_the_pollen() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut gpu = LiveGpu::new(&device, &queue, None);
        let size = (700, 450);
        let quiet = render(&device, &queue, &mut gpu, size, params(0.0));
        let loud = render(&device, &queue, &mut gpu, size, params(1.0));
        assert!(
            lum(&loud) > lum(&quiet),
            "al hablar, el halo y la figura brillan más: {} frente a {}",
            lum(&loud),
            lum(&quiet)
        );
        let later = render(
            &device,
            &queue,
            &mut gpu,
            size,
            Params {
                t: 6.0,
                ..params(0.0)
            },
        );
        assert!(later != quiet, "con el tiempo el polen se mueve");
        let again = render(&device, &queue, &mut gpu, size, params(0.0));
        assert!(again == quiet, "el mismo instante da la misma imagen");
    }

    fn model_params(level: f32) -> Params {
        let mut p = params(level);
        p.model = true;
        p.face = crate::live::model::Face {
            jaw: level,
            ..Default::default()
        };
        p
    }

    #[test]
    fn the_3d_model_draws_a_lit_figure_that_turns_and_opens_its_mouth() {
        let Some((device, queue)) = headless() else {
            eprintln!("(sin adaptador gráfico: prueba omitida)");
            return;
        };
        let mut gpu = LiveGpu::new(&device, &queue, None);
        assert!(gpu.bia.is_some(), "el modelo se subió a la GPU");
        let size = (900u32, 700u32);
        let backdrop_only = render(
            &device,
            &queue,
            &mut gpu,
            size,
            Params {
                model: false,
                ..params(0.0)
            },
        );
        let figure = render(&device, &queue, &mut gpu, size, model_params(0.0));
        let diff = |a: &[u8], b: &[u8]| {
            a.chunks(4)
                .zip(b.chunks(4))
                .filter(|(x, y)| (0..3).any(|k| (x[k] as i32 - y[k] as i32).abs() > 12))
                .count()
        };
        assert!(
            diff(&figure, &backdrop_only) > 8_000,
            "la figura cubre una buena parte del escenario: {}",
            diff(&figure, &backdrop_only)
        );
        // El rostro (zona alta del centro) está iluminado: hay píxeles de color piel (rojo > azul, claros).
        let at = |px: &[u8], x: u32, y: u32| px[((y * size.0 + x) * 4) as usize..][..3].to_vec();
        let skin = (150..300)
            .flat_map(|y| (380..520).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = at(&figure, x, y);
                p[0] > 90 && p[0] as i32 > p[2] as i32 + 15 && p[1] > 50
            })
            .count();
        assert!(
            skin > 300,
            "se ve la piel de la cara iluminada: {skin} píxeles"
        );
        // Abrir la mandíbula cambia la zona de la boca y casi nada más.
        // Solo cambia el gesto: con `level` también subirían la luz del pecho y el halo, que no es lo que se mide aquí.
        let open = render(
            &device,
            &queue,
            &mut gpu,
            size,
            Params {
                face: crate::live::model::Face {
                    jaw: 1.0,
                    ..Default::default()
                },
                ..model_params(0.0)
            },
        );
        let changed = diff(&figure, &open);
        assert!(changed > 20, "abrir la boca cambia la imagen: {changed}");
        assert!(
            changed < 6_000,
            "pero solo la cara, no toda la figura: {changed}"
        );
        // Girar el modelo con el ratón mueve la figura.
        let turned = render(
            &device,
            &queue,
            &mut gpu,
            size,
            Params {
                yaw: 0.7,
                ..model_params(0.0)
            },
        );
        assert!(
            diff(&figure, &turned) > 5_000,
            "con el modelo girado la imagen es distinta"
        );
        // Y el mismo instante da la misma imagen.
        assert!(render(&device, &queue, &mut gpu, size, model_params(0.0)) == figure);
    }

    /// Cuánto cuesta un fotograma del escenario con el modelo. `cargo test -- --ignored --nocapture live_frame_time`
    #[test]
    #[ignore = "mide tiempos con tu GPU"]
    fn live_frame_time_with_the_model() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut gpu = LiveGpu::new(&device, &queue, None);
        let size = (1152, 733);
        let mut face = crate::live::model::Face::default();
        render(&device, &queue, &mut gpu, size, model_params(0.0)); // calentamiento
        let start = std::time::Instant::now();
        let n = 60;
        for i in 0..n {
            face.jaw = (i as f32 * 0.3).sin().abs();
            let mut p = model_params(face.jaw);
            p.t = i as f32 / 60.0;
            p.face = face;
            let mut enc = device.create_command_encoder(&Default::default());
            gpu.render(&device, &queue, &mut enc, size, p);
            queue.submit([enc.finish()]);
        }
        device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!(
            "HiveLive con el modelo 3D: {ms:.2} ms por fotograma a {}x{}",
            size.0, size.1
        );
        assert!(ms < 33.0, "debe caber en 30 fps: {ms:.1} ms");
    }

    /// Cuánto tarda la primera vez: leer el GLB y crear la GPU. `cargo test -- --ignored --nocapture live_first_use`
    #[test]
    #[ignore = "mide tiempos"]
    fn live_first_use_cost() {
        let t = std::time::Instant::now();
        let bia = crate::live::model::Bia::load().expect("modelo");
        eprintln!(
            "leer el GLB (mallas, 3 texturas decodificadas): {:?} ({} vértices)",
            t.elapsed(),
            bia.vertex_count()
        );
        let Some((device, queue)) = headless() else {
            return;
        };
        let t = std::time::Instant::now();
        let _gpu = LiveGpu::new(&device, &queue, None);
        eprintln!(
            "LiveGpu::new completo (incluye leer el GLB otra vez, mipmaps y subida): {:?}",
            t.elapsed()
        );
    }
}
