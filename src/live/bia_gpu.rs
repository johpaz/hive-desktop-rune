//! BIA en la GPU: sube el modelo y sus tres texturas (con mipmaps), mueve la cara cada fotograma y lo dibuja
//! con la cámara y las luces de la web (`VoiceStage.tsx`/`BiaModel.tsx`).

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::gpu::Params;
use super::model::{Bia, Image, Vertex};
use crate::office::camera::{mul, perspective, Mat4};
use crate::office::xform::{chain, rot_y, scale, translate};

const SHADER: &str = include_str!("bia.wgsl");

/// La cámara de la web: campo de visión de 32°, en (0; 0,15; 6,2) mirando hacia −z.
pub const EYE: [f32; 3] = [0.0, 0.15, 6.2];
const FOV: f32 = 32.0 * std::f32::consts::PI / 180.0;
/// El modelo: escala 1,3 y bajado 1,02 (`<group position={[0,-1.02,0]} scale={1.3}>`).
const SCALE: f32 = 1.3;
const BASE_Y: f32 = -1.02;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    vp: [f32; 16],
    model: [f32; 16],
    eye: [f32; 4],
    l1d: [f32; 4],
    l1c: [f32; 4],
    l2d: [f32; 4],
    l2c: [f32; 4],
    pl_pos: [f32; 4],
    pl_col: [f32; 4],
    ambient: [f32; 4],
    flat_col: [f32; 4],
    flags: [f32; 4],
}

struct Draw {
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

struct PartGpu {
    draw: Draw,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    count: u32,
    offset: [f32; 3],
    joint: Option<usize>,
    color: [f32; 4],
    roughness: f32,
}

pub struct BiaGpu {
    model: Bia,
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    scratch: Vec<Vertex>,
    face: Draw,
    parts: Vec<PartGpu>,
}

/// sRGB (0–1) a luz lineal.
fn lin(c: [f32; 3]) -> [f32; 3] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2)]
}

fn hex(h: u32) -> [f32; 3] {
    [
        (h >> 16 & 0xff) as f32 / 255.0,
        (h >> 8 & 0xff) as f32 / 255.0,
        (h & 0xff) as f32 / 255.0,
    ]
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Dónde está la cámara: la de la web, desplazada por el pan y alejada o acercada por el zoom (siempre mirando hacia −z).
pub fn camera_eye(p: &Params) -> [f32; 3] {
    [
        EYE[0] + p.pan[0],
        EYE[1] + p.pan[1],
        p.dist.clamp(DIST_MIN, DIST_MAX),
    ]
}

/// Los topes de distancia de la web (`DIST_MIN`/`DIST_MAX` de `SceneControls`).
pub const DIST_MIN: f32 = 2.4;
pub const DIST_MAX: f32 = 13.0;

/// Un punto del espacio del modelo al mundo.
fn transform(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// La matriz del modelo: respira, se balancea apenas y gira con el ratón.
pub fn model_matrix(p: &Params) -> Mat4 {
    let breathe = 0.012 * (p.t * 1.1).sin();
    let sway = 0.02 * (p.t * 0.5).sin();
    chain(&[
        translate([sway, BASE_Y + breathe, 0.0]),
        rot_y(p.yaw),
        scale([SCALE; 3]),
    ])
}

fn uniforms(p: &Params, aspect: f32, model: Mat4, flat: [f32; 4], textured: bool) -> Uniforms {
    let eye = camera_eye(p);
    let vp = mul(
        &perspective(FOV, aspect, 0.1, 60.0),
        &translate([-eye[0], -eye[1], -eye[2]]),
    );
    let m0 = model_matrix(p);
    let l1 = lin(hex(0xffe0b0));
    let l2 = lin(hex(0x8fd0ff));
    let tint = lin(p.tint);
    // La luz puntual de BIA: intensidad 2 + voz × 6, sobre el pecho (en el espacio del modelo).
    let pl = transform(&m0, [0.0, 1.35, 1.4]);
    let power = 2.0 + p.level * 6.0;
    Uniforms {
        vp,
        model,
        eye: [eye[0], eye[1], eye[2], 0.0],
        l1d: {
            let d = norm([3.0, 5.0, 6.0]);
            [d[0], d[1], d[2], 0.0]
        },
        l1c: [l1[0] * 0.7, l1[1] * 0.7, l1[2] * 0.7, 0.0],
        l2d: {
            let d = norm([-4.0, 2.0, 3.0]);
            [d[0], d[1], d[2], 0.0]
        },
        l2c: [l2[0] * 0.35, l2[1] * 0.35, l2[2] * 0.35, 0.0],
        pl_pos: [pl[0], pl[1], pl[2], 7.0],
        pl_col: [tint[0] * power, tint[1] * power, tint[2] * power, 1.7],
        ambient: [0.85, 0.85, 0.85, 0.0],
        flat_col: flat,
        flags: [
            if textured { 1.0 } else { 0.0 },
            0.3 + p.level * 0.5,
            0.0,
            0.0,
        ],
    }
}

/// Número de niveles de la cadena de mipmaps de una imagen.
pub fn mip_levels(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// Sube una imagen con su cadena de mipmaps (la reducción es un filtro triangular en la CPU).
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    img: &Image,
    srgb: bool,
    label: &str,
) -> wgpu::TextureView {
    let levels = mip_levels(img.w, img.h);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: img.w,
            height: img.h,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut cur = image::RgbaImage::from_raw(img.w, img.h, img.rgba.clone())
        .expect("tamaño de imagen coherente");
    for level in 0..levels {
        let (w, h) = (cur.width(), cur.height());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &cur,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        if level + 1 < levels {
            cur = image::imageops::resize(
                &cur,
                (w / 2).max(1),
                (h / 2).max(1),
                image::imageops::FilterType::Triangle,
            );
        }
    }
    tex.create_view(&Default::default())
}

impl BiaGpu {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> Option<BiaGpu> {
        let model = super::model::take()?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bia"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bia"),
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
                tex(1),
                tex(2),
                tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bia"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bia"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            // Doble cara: la malla del pelo y las alas se ven por los dos lados.
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        let (base, normal, mr) = (
            upload(device, queue, &model.color, true, "bia base"),
            upload(device, queue, &model.normal, false, "bia normal"),
            upload(device, queue, &model.metal_rough, false, "bia mr"),
        );
        let draw = |label: &str| {
            let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&base),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&normal),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&mr),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            Draw { ubuf, bind }
        };

        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bia vertices"),
            size: (model.vertex_count() * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bia indices"),
            contents: bytemuck::cast_slice(&model.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let parts = model
            .parts
            .iter()
            .map(|p| PartGpu {
                draw: draw(&p.name),
                vbuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&p.name),
                    contents: bytemuck::cast_slice(&p.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                ibuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&p.name),
                    contents: bytemuck::cast_slice(&p.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                count: p.indices.len() as u32,
                offset: p.offset,
                joint: p.joint,
                color: p.color,
                roughness: p.roughness,
            })
            .collect();
        let face = draw("bia face");
        let index_count = model.indices.len() as u32;
        Some(BiaGpu {
            model,
            pipeline,
            vbuf,
            ibuf,
            index_count,
            scratch: Vec::new(),
            face,
            parts,
        })
    }

    /// Mueve la cara y escribe las matrices y luces de este fotograma.
    pub fn update(&mut self, queue: &wgpu::Queue, p: &Params, size: (u32, u32)) {
        let aspect = size.0 as f32 / size.1 as f32;
        self.model.morphed(&p.face, &mut self.scratch);
        let skin = self.model.skin_matrices(&p.pose);
        self.model.skinned(&skin, &mut self.scratch);
        queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.scratch));
        let m = model_matrix(p);
        queue.write_buffer(
            &self.face.ubuf,
            0,
            bytemuck::bytes_of(&uniforms(p, aspect, m, [0.0; 4], true)),
        );
        for part in &self.parts {
            // La boca va atada a la cabeza: sigue su cabeceo y su ladeo.
            let head = Bia::joint_matrix(&skin, part.joint);
            let pm = mul(&mul(&m, &head), &translate(part.offset));
            let flat = [part.color[0], part.color[1], part.color[2], part.roughness];
            queue.write_buffer(
                &part.draw.ubuf,
                0,
                bytemuck::bytes_of(&uniforms(p, aspect, pm, flat, false)),
            );
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        // La boca por dentro primero (cavidad, dientes, lengua) y la cara encima, con su abertura.
        for part in &self.parts {
            pass.set_bind_group(0, &part.draw.bind, &[]);
            pass.set_vertex_buffer(0, part.vbuf.slice(..));
            pass.set_index_buffer(part.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..part.count, 0, 0..1);
        }
        pass.set_bind_group(0, &self.face.bind, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
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
        assert_eq!(std::mem::size_of::<Uniforms>(), 64 + 64 + 10 * 16);
    }

    #[test]
    fn mip_chains_reach_one_pixel() {
        assert_eq!(mip_levels(2048, 2048), 12);
        assert_eq!(mip_levels(1024, 512), 11);
        assert_eq!(mip_levels(1, 1), 1);
    }

    #[test]
    fn the_model_matrix_follows_the_webs_placement_and_the_drag() {
        let p = Params {
            t: 0.0,
            level: 0.0,
            tint: [1.0; 3],
            dpr: 1.0,
            sway: 0.0,
            model: true,
            yaw: 0.0,
            pan: [0.0; 2],
            dist: EYE[2],
            face: Default::default(),
            pose: Default::default(),
        };
        let m = model_matrix(&p);
        // El origen del modelo cae en y = −1,02 y la cabeza (y = 1) en 1,3 − 1,02 = 0,28.
        assert!((transform(&m, [0.0, 0.0, 0.0])[1] + 1.02).abs() < 1e-5);
        assert!((transform(&m, [0.0, 1.0, 0.0])[1] - 0.28).abs() < 1e-5);
        // Girar el modelo mueve un punto de su costado alrededor del eje vertical.
        let turned = model_matrix(&Params {
            yaw: std::f32::consts::FRAC_PI_2,
            ..p
        });
        let side = transform(&turned, [1.0, 0.0, 0.0]);
        assert!(
            side[0].abs() < 1e-4 && side[2].abs() > 1.2,
            "un cuarto de vuelta lleva x a z: {side:?}"
        );
    }
}
