//! Renderer de la oficina con wgpu: dibuja la escena en un objetivo HDR con profundidad, le aplica bloom
//! (cadena de mips) y viñeta, y deja el resultado en una textura sRGB que el hueco de Rune pinta como un quad.
//!
//! El pase de Rune no tiene profundidad, por eso la escena va a su propia textura y no directamente al hueco.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use rune::compose::DeviceKey;
use rune::render::EffectSite;
use wgpu::util::DeviceExt;

use super::camera::Camera;
use super::scene::{lin, Draw, Material, SceneFrame, Shape};
use super::shaders;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Capacidad de la reserva de mallas (vértices e índices).
const MAX_VERTS: usize = 200_000;
const MAX_INDICES: usize = 600_000;

/// Parámetros del posproceso de la UI web.
pub const BLOOM_THRESHOLD: f32 = 0.38;
pub const BLOOM_SMOOTHING: f32 = 0.22;
pub const BLOOM_RADIUS: f32 = 0.85;
pub const VIGNETTE_OFFSET: f32 = 0.34;
pub const VIGNETTE_DARKNESS: f32 = 0.38;

/// El fondo de la escena según el tema: un azul noche algo más claro que el negro de antes, o un azul día de tono medio
/// (los brillos de la oficina se suman a lo que hay detrás, así que un fondo casi blanco los borraría).
pub fn backdrop() -> u32 {
    if crate::ui::theme::is_light() {
        0xaebbd0
    } else {
        0x263a62
    }
}

pub fn bloom_intensity(high: bool) -> f32 {
    if high {
        0.52
    } else {
        0.32
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    vp: [f32; 16],
    eye: [f32; 4],
    fog: [f32; 4],
    fog_color: [f32; 4],
    ambient: [f32; 4],
    dir_to_light: [f32; 4],
    dir_color: [f32; 4],
    pl_pos: [f32; 4],
    pl_color: [f32; 4],
    ol_pos: [f32; 4],
    ol_color: [f32; 4],
    view: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    model: [f32; 16],
    color: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GVert {
    pos: [f32; 3],
    nrm: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostParams {
    texel: [f32; 2],
    threshold: f32,
    smoothing: f32,
    intensity: f32,
    vig_offset: f32,
    vig_dark: f32,
    radius: f32,
}

#[derive(Clone, Copy)]
struct MeshRef {
    id: u32,
    base_vertex: i32,
    first_index: u32,
    count: u32,
    /// Índices de aristas (para el alambre), a continuación de los triángulos.
    wire_first: u32,
    wire_count: u32,
}

/// Orden de dibujo: primero lo opaco (que escribe profundidad) y luego lo translúcido.
fn material_order(m: Material) -> Option<u32> {
    Some(match m {
        Material::Solid => 0,
        Material::Basic => 1,
        Material::Floor => 2,
        Material::SolidAlpha => 3,
        Material::Hologram => 4,
        Material::BasicAdd => 5,
        Material::Beam => 6,
        Material::Sprite => 7,
        Material::WireAdd => 8,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PassKind {
    Threshold,
    Down,
    Up,
    Composite,
}

struct Pass {
    kind: PassKind,
    target: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

struct Targets {
    size: (u32, u32),
    hdr: wgpu::TextureView,
    depth: wgpu::TextureView,
    _out: wgpu::TextureView,
    /// La textura de salida: la lee `read_pixels` (pruebas); en la app basta con mantenerla viva.
    #[cfg_attr(not(test), allow(dead_code))]
    out_tex: wgpu::Texture,
    passes: Vec<Pass>,
    comp_params: wgpu::Buffer,
    blit_bind: wgpu::BindGroup,
}

struct Post {
    bgl: wgpu::BindGroupLayout,
    thr: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    comp: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,
}

pub struct OfficeGpu {
    /// Dispositivo para el que se creó (`None` fuera de Rune, en las pruebas).
    pub key: Option<DeviceKey>,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    pipes: HashMap<Material, wgpu::RenderPipeline>,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    vcount: usize,
    icount: usize,
    meshes: HashMap<Shape, MeshRef>,
    inst: wgpu::Buffer,
    inst_cap: usize,
    post: Post,
    targets: Option<Targets>,
    blit_module: wgpu::ShaderModule,
    blit_layout: wgpu::PipelineLayout,
    blit_bgl: wgpu::BindGroupLayout,
    blit_pipes: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

fn blend_add() -> wgpu::BlendState {
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: add,
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn blend_alpha() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn texture_with(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    extra: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | extra,
        view_formats: &[],
    })
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    texture_with(device, label, size, format, wgpu::TextureUsages::empty())
        .create_view(&Default::default())
}

impl OfficeGpu {
    pub fn new(device: &wgpu::Device, key: Option<DeviceKey>) -> OfficeGpu {
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("office globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("office globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("office globals"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("office scene"),
            bind_group_layouts: &[Some(&globals_bgl)],
            immediate_size: 0,
        });

        let attr = |loc, off, fmt| wgpu::VertexAttribute {
            shader_location: loc,
            offset: off,
            format: fmt,
        };
        let mesh_attrs = [
            attr(0, 0, wgpu::VertexFormat::Float32x3),
            attr(1, 12, wgpu::VertexFormat::Float32x3),
        ];
        let f4 = wgpu::VertexFormat::Float32x4;
        let inst_attrs = [
            attr(2, 0, f4),
            attr(3, 16, f4),
            attr(4, 32, f4),
            attr(5, 48, f4),
            attr(6, 64, f4),
            attr(7, 80, f4),
            attr(8, 96, f4),
        ];

        let mut pipes = HashMap::new();
        let mut make = |m: Material,
                        body: &str,
                        fs: &str,
                        blend: Option<wgpu::BlendState>,
                        depth_write: bool,
                        topology: wgpu::PrimitiveTopology| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("office scene"),
                source: wgpu::ShaderSource::Wgsl(shaders::scene(body).into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&format!("office {m:?}")),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 24,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &mesh_attrs,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Inst>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &inst_attrs,
                        }),
                    ],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            pipes.insert(m, pipeline);
        };
        make(
            Material::Solid,
            shaders::SOLID,
            "fs_opaque",
            None,
            true,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::SolidAlpha,
            shaders::SOLID,
            "fs_alpha",
            Some(blend_alpha()),
            false,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::Basic,
            shaders::BASIC,
            "fs_opaque",
            None,
            true,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::BasicAdd,
            shaders::BASIC,
            "fs_add",
            Some(blend_add()),
            false,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::WireAdd,
            shaders::BASIC,
            "fs_add",
            Some(blend_add()),
            false,
            wgpu::PrimitiveTopology::LineList,
        );
        make(
            Material::Floor,
            shaders::FLOOR,
            "fs",
            Some(blend_add()),
            false,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::Hologram,
            shaders::HOLOGRAM,
            "fs",
            Some(blend_add()),
            false,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::Beam,
            shaders::BEAM,
            "fs",
            Some(blend_add()),
            false,
            wgpu::PrimitiveTopology::TriangleList,
        );
        make(
            Material::Sprite,
            shaders::SPRITE,
            "fs",
            Some(blend_add()),
            false,
            wgpu::PrimitiveTopology::TriangleList,
        );

        let vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("office verts"),
            size: (MAX_VERTS * std::mem::size_of::<GVert>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("office indices"),
            size: (MAX_INDICES * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let inst_cap = 1024;
        let inst = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("office instances"),
            size: (inst_cap * std::mem::size_of::<Inst>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Posproceso.
        let post_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("office post"),
            source: wgpu::ShaderSource::Wgsl(shaders::post().into()),
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("office post"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("office post"),
            bind_group_layouts: &[Some(&post_bgl)],
            immediate_size: 0,
        });
        let post_pipe = |fs: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&post_layout),
                vertex: wgpu::VertexState {
                    module: &post_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &post_module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let post = Post {
            bgl: post_bgl,
            thr: post_pipe("fs_thr", HDR),
            down: post_pipe("fs_down", HDR),
            up: post_pipe("fs_up", HDR),
            comp: post_pipe("fs_comp", OUT),
            sampler,
            dummy: texture(device, "office dummy", (1, 1), HDR),
        };

        // Volcado del resultado en el hueco de Rune.
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("office blit"),
            source: wgpu::ShaderSource::Wgsl(shaders::BLIT.into()),
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("office blit"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("office blit"),
            bind_group_layouts: &[Some(&blit_bgl)],
            immediate_size: 0,
        });

        OfficeGpu {
            key,
            globals,
            globals_bind,
            pipes,
            vbuf,
            ibuf,
            vcount: 0,
            icount: 0,
            meshes: HashMap::new(),
            inst,
            inst_cap,
            post,
            targets: None,
            blit_module,
            blit_layout,
            blit_bgl,
            blit_pipes: HashMap::new(),
        }
    }

    /// Crea la malla la primera vez que se pide y la añade a la reserva; `None` si ya no cabe.
    fn mesh(&mut self, queue: &wgpu::Queue, shape: Shape) -> Option<MeshRef> {
        if let Some(m) = self.meshes.get(&shape) {
            return Some(*m);
        }
        let mesh = shape.build();
        let wire = mesh.wire_indices();
        if self.vcount + mesh.verts.len() > MAX_VERTS
            || self.icount + mesh.idx.len() + wire.len() > MAX_INDICES
        {
            return None;
        }
        let verts: Vec<GVert> = mesh
            .verts
            .iter()
            .map(|v| GVert {
                pos: v.pos,
                nrm: v.nrm,
            })
            .collect();
        queue.write_buffer(
            &self.vbuf,
            (self.vcount * std::mem::size_of::<GVert>()) as u64,
            bytemuck::cast_slice(&verts),
        );
        queue.write_buffer(
            &self.ibuf,
            (self.icount * 4) as u64,
            bytemuck::cast_slice(&mesh.idx),
        );
        queue.write_buffer(
            &self.ibuf,
            ((self.icount + mesh.idx.len()) * 4) as u64,
            bytemuck::cast_slice(&wire),
        );
        let r = MeshRef {
            id: self.meshes.len() as u32,
            base_vertex: self.vcount as i32,
            first_index: self.icount as u32,
            count: mesh.idx.len() as u32,
            wire_first: (self.icount + mesh.idx.len()) as u32,
            wire_count: wire.len() as u32,
        };
        self.vcount += mesh.verts.len();
        self.icount += mesh.idx.len() + wire.len();
        self.meshes.insert(shape, r);
        Some(r)
    }

    /// Cuántos niveles tiene la cadena del bloom para este tamaño.
    pub fn bloom_levels(size: (u32, u32)) -> usize {
        ((size.0.min(size.1).max(1) as f32).log2().floor() as i32 - 3).clamp(1, 8) as usize
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let hdr = texture(device, "office hdr", size, HDR);
        let depth = texture(device, "office depth", size, DEPTH);
        let out_tex = texture_with(
            device,
            "office out",
            size,
            OUT,
            wgpu::TextureUsages::COPY_SRC,
        );
        let out = out_tex.create_view(&Default::default());

        let levels = Self::bloom_levels(size);
        let mip_size = |i: usize| ((size.0 >> (i + 1)).max(1), (size.1 >> (i + 1)).max(1));
        let downs: Vec<wgpu::TextureView> = (0..levels)
            .map(|i| texture(device, "office down", mip_size(i), HDR))
            .collect();
        let ups: Vec<wgpu::TextureView> = (0..levels.saturating_sub(1))
            .map(|i| texture(device, "office up", mip_size(i), HDR))
            .collect();

        let params = |texel: (u32, u32), intensity: f32| PostParams {
            texel: [1.0 / texel.0.max(1) as f32, 1.0 / texel.1.max(1) as f32],
            threshold: BLOOM_THRESHOLD,
            smoothing: BLOOM_SMOOTHING,
            intensity,
            vig_offset: VIGNETTE_OFFSET,
            vig_dark: if crate::ui::theme::is_light() {
                VIGNETTE_DARKNESS * 0.4
            } else {
                VIGNETTE_DARKNESS
            },
            radius: BLOOM_RADIUS,
        };
        let bind = |a: &wgpu::TextureView, b: &wgpu::TextureView, buf: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("office post"),
                layout: &self.post.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(a),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(b),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.post.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: buf.as_entire_binding(),
                    },
                ],
            })
        };
        let uniform = |p: PostParams| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("office post params"),
                contents: bytemuck::bytes_of(&p),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        };

        let mut passes = Vec::new();
        // Umbral + primera reducción: del HDR al primer nivel.
        let b = uniform(params(size, 0.0));
        passes.push(Pass {
            kind: PassKind::Threshold,
            target: downs[0].clone(),
            bind: bind(&hdr, &self.post.dummy, &b),
        });
        // Reducciones sucesivas.
        for i in 1..levels {
            let b = uniform(params(mip_size(i - 1), 0.0));
            passes.push(Pass {
                kind: PassKind::Down,
                target: downs[i].clone(),
                bind: bind(&downs[i - 1], &self.post.dummy, &b),
            });
        }
        // Ampliaciones: de lo más pequeño a lo más grande, mezclando con el nivel de reducción.
        let mut prev: Option<&wgpu::TextureView> = None;
        for i in (0..levels.saturating_sub(1)).rev() {
            let small = prev.unwrap_or(&downs[i + 1]);
            let b = uniform(params(mip_size(i + 1), 0.0));
            passes.push(Pass {
                kind: PassKind::Up,
                target: ups[i].clone(),
                bind: bind(small, &downs[i], &b),
            });
            prev = Some(&ups[i]);
        }
        let bloom = ups.first().unwrap_or(&downs[0]);
        let comp_params = uniform(params(size, bloom_intensity(true)));
        passes.push(Pass {
            kind: PassKind::Composite,
            target: out.clone(),
            bind: bind(&hdr, bloom, &comp_params),
        });

        let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("office blit"),
            layout: &self.blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&out),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.post.sampler),
                },
            ],
        });
        self.targets = Some(Targets {
            size,
            hdr,
            depth,
            _out: out,
            out_tex,
            passes,
            comp_params,
            blit_bind,
        });
    }

    fn globals(frame: &SceneFrame, camera: &Camera, size: (u32, u32), dpr: f32, t: f32) -> Globals {
        let aspect = size.0 as f32 / size.1 as f32;
        let eye = camera.eye();
        let fog = lin(backdrop());
        let dir = lin(0x9db8ff);
        let amber = lin(0xf59e0b);
        let l = {
            let v = [12.0f32, 24.0, 8.0];
            let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / n, v[1] / n, v[2] / n]
        };
        let (lp, li) = frame.queen_light;
        Globals {
            vp: camera.view_proj(aspect),
            eye: [eye[0], eye[1], eye[2], t],
            fog: [34.0, 100.0, 0.0, 0.0],
            fog_color: [fog[0], fog[1], fog[2], 1.0],
            ambient: {
                let a = if crate::ui::theme::is_light() {
                    0.62
                } else {
                    0.5
                };
                [a, a, a, 0.0]
            },
            dir_to_light: [l[0], l[1], l[2], 0.0],
            dir_color: [dir[0] * 0.35, dir[1] * 0.35, dir[2] * 0.35, 0.0],
            pl_pos: [lp[0], lp[1], lp[2], 18.0],
            pl_color: [amber[0] * li, amber[1] * li, amber[2] * li, 1.8],
            // Luz del oráculo (como la `pointLight` de la web: distancia 12, decaimiento 1,8); apagada si no hay Jev.
            ol_pos: frame
                .oracle_light
                .map_or([0.0; 4], |(p, _, _)| [p[0], p[1], p[2], 12.0]),
            ol_color: frame
                .oracle_light
                .map_or([0.0; 4], |(_, c, i)| [c[0] * i, c[1] * i, c[2] * i, 1.8]),
            view: [size.0 as f32, size.1 as f32, dpr, 0.0],
        }
    }

    /// Dibuja la escena en `size` píxeles y deja el resultado listo para `blit`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &SceneFrame,
        camera: &Camera,
        t: f32,
        size: (u32, u32),
        dpr: f32,
        high: bool,
    ) {
        let size = (size.0.clamp(32, 3840), size.1.clamp(32, 2160));
        self.ensure_targets(device, size);
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Self::globals(frame, camera, size, dpr, t)),
        );
        if let Some(t) = &self.targets {
            let p = PostParams {
                texel: [1.0 / size.0 as f32, 1.0 / size.1 as f32],
                threshold: BLOOM_THRESHOLD,
                smoothing: BLOOM_SMOOTHING,
                intensity: bloom_intensity(high),
                vig_offset: VIGNETTE_OFFSET,
                vig_dark: if crate::ui::theme::is_light() {
                    VIGNETTE_DARKNESS * 0.4
                } else {
                    VIGNETTE_DARKNESS
                },
                radius: BLOOM_RADIUS,
            };
            queue.write_buffer(&t.comp_params, 0, bytemuck::bytes_of(&p));
        }

        // Agrupa por material y malla: cada grupo es una llamada instanciada.
        let mut items: Vec<(u32, MeshRef, &Draw)> = Vec::with_capacity(frame.draws.len());
        for d in &frame.draws {
            let Some(order) = material_order(d.material) else {
                continue;
            };
            let Some(m) = self.mesh(queue, d.shape) else {
                continue;
            };
            items.push((order, m, d));
        }
        items.sort_by_key(|(o, m, _)| (*o, m.id));
        if items.len() > self.inst_cap {
            self.inst_cap = items.len().next_power_of_two();
            self.inst = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("office instances"),
                size: (self.inst_cap * std::mem::size_of::<Inst>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let data: Vec<Inst> = items
            .iter()
            .map(|(_, _, d)| Inst {
                model: d.model,
                color: d.color,
                p0: d.p0,
                p1: d.p1,
            })
            .collect();
        if !data.is_empty() {
            queue.write_buffer(&self.inst, 0, bytemuck::cast_slice(&data));
        }

        let targets = self.targets.as_ref().expect("objetivos creados");
        let clear = lin(backdrop());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("office scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.hdr,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.set_vertex_buffer(1, self.inst.slice(..));
            pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            let mut i = 0;
            while i < items.len() {
                let (order, mesh, draw) = (items[i].0, items[i].1, items[i].2);
                let mut j = i;
                while j < items.len() && items[j].0 == order && items[j].1.id == mesh.id {
                    j += 1;
                }
                if let Some(p) = self.pipes.get(&draw.material) {
                    pass.set_pipeline(p);
                    let (first, count) = if draw.material == Material::WireAdd {
                        (mesh.wire_first, mesh.wire_count)
                    } else {
                        (mesh.first_index, mesh.count)
                    };
                    pass.draw_indexed(first..first + count, mesh.base_vertex, i as u32..j as u32);
                }
                i = j;
            }
        }

        // Bloom y composición final.
        for p in &targets.passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("office post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &p.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(match p.kind {
                PassKind::Threshold => &self.post.thr,
                PassKind::Down => &self.post.down,
                PassKind::Up => &self.post.up,
                PassKind::Composite => &self.post.comp,
            });
            pass.set_bind_group(0, &p.bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Pinta el resultado en el hueco: el viewport lo recorta a la caja del elemento (en puntos).
    pub fn blit(
        &mut self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        site: &EffectSite<'_>,
        rect: [f32; 4],
    ) {
        let Some(targets) = &self.targets else { return };
        let pipe = self.blit_pipes.entry(site.format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("office blit"),
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
        pass.set_bind_group(0, &targets.blit_bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Tamaño con el que se dibujó por última vez (para pruebas).
    #[cfg(test)]
    pub fn size(&self) -> Option<(u32, u32)> {
        self.targets.as_ref().map(|t| t.size)
    }

    /// Lee de vuelta la imagen final (RGBA8, filas sin relleno). Solo para pruebas con GPU real.
    #[cfg(test)]
    pub fn read_out(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.targets.as_ref()?;
        let (w, h) = t.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("office readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.out_tex,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bloom_chain_depth_depends_on_the_size_and_stays_bounded() {
        assert_eq!(OfficeGpu::bloom_levels((960, 600)), 6);
        assert_eq!(OfficeGpu::bloom_levels((32, 32)), 2);
        assert_eq!(
            OfficeGpu::bloom_levels((1, 1)),
            1,
            "nunca menos de un nivel"
        );
        assert_eq!(OfficeGpu::bloom_levels((8000, 8000)), 8, "ni más de ocho");
    }

    #[test]
    fn bloom_is_stronger_in_high_quality() {
        assert_eq!(bloom_intensity(true), 0.52);
        assert_eq!(bloom_intensity(false), 0.32);
    }

    #[test]
    fn the_gpu_structs_have_the_sizes_the_shaders_expect() {
        assert_eq!(std::mem::size_of::<Inst>(), 112, "16 + 4 + 4 + 4 floats");
        assert_eq!(std::mem::size_of::<GVert>(), 24);
        assert_eq!(
            std::mem::size_of::<Globals>(),
            64 + 11 * 16,
            "matriz + 11 vectores (con la luz del oráculo)"
        );
        assert_eq!(std::mem::size_of::<PostParams>(), 32);
    }

    #[test]
    fn opaque_materials_are_drawn_before_translucent_ones() {
        let o = |m| material_order(m).unwrap();
        assert!(o(Material::Solid) < o(Material::Floor) && o(Material::Basic) < o(Material::Floor));
        assert!(
            o(Material::Floor) < o(Material::Hologram) && o(Material::Hologram) < o(Material::Beam)
        );
        assert!(
            o(Material::Beam) < o(Material::WireAdd),
            "el alambre se pinta al final"
        );
    }

    // ── con GPU real: se omiten (con aviso) si no hay adaptador gráfico ──

    fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn busy_office() -> crate::office::state::OfficeState {
        use crate::office::state::OfficeState;
        use serde_json::json;
        let agent = |id: &str, role: &str| crate::settings::api::Agent {
            id: id.into(),
            name: id.into(),
            role: role.into(),
            status: "idle".into(),
            enabled: true,
            ..Default::default()
        };
        let mut o = OfficeState {
            agents: vec![
                agent("boss", "coordinator"),
                agent("a1", "worker"),
                agent("a2", "worker"),
                agent("a3", "worker"),
                agent("a4", "worker"),
            ],
            ..Default::default()
        };
        let node = |id: &str, status: &str, data: serde_json::Value| json!({"id": id, "type": "agent", "label": id, "status": status, "data": data});
        o.graph.apply(
            "canvas:node_add",
            &node("a1", "tool_call", json!({"currentTask": "t"})),
            None,
            0.0,
        );
        o.graph.apply(
            "canvas:node_add",
            &node("a2", "thinking", json!({"currentTask": "t"})),
            None,
            0.0,
        );
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "e", "source": "boss", "target": "a1", "edgeType": "delegates"}),
            None,
            0.0,
        );
        for _ in 0..600 {
            o.step(1.0 / 60.0);
        }
        o
    }

    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut OfficeGpu,
        o: &crate::office::state::OfficeState,
        size: (u32, u32),
        high: bool,
    ) -> (u32, u32, Vec<u8>) {
        let frame = crate::office::scene::build(o);
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(
            device, queue, &mut enc, &frame, &o.camera, o.t, size, 1.0, high,
        );
        queue.submit([enc.finish()]);
        gpu.read_out(device, queue).expect("lectura")
    }

    fn luminance_sum(px: &[u8]) -> u64 {
        px.chunks(4)
            .map(|p| (p[0] as u64 * 2126 + p[1] as u64 * 7152 + p[2] as u64 * 722) / 10000)
            .sum()
    }

    #[test]
    fn the_office_renders_visible_content_with_the_right_colors() {
        let Some((device, queue)) = headless() else {
            eprintln!("(sin adaptador gráfico: prueba omitida)");
            return;
        };
        let mut gpu = OfficeGpu::new(&device, None);
        let (w, h, px) = render(&device, &queue, &mut gpu, &busy_office(), (640, 400), true);
        assert_eq!((w, h, px.len()), (640, 400, 640 * 400 * 4));
        let count =
            |f: &dyn Fn(u8, u8, u8) -> bool| px.chunks(4).filter(|p| f(p[0], p[1], p[2])).count();
        let lit = count(&|r, g, b| r.max(g).max(b) > 120);
        assert!(
            lit > 640 * 400 / 50,
            "hay contenido, no un lienzo vacío: {lit} píxeles encendidos"
        );
        assert!(
            count(&|r, g, b| r > 150 && r > g && g > b) > 50,
            "ámbar de la reina y su pedestal"
        );
        assert!(
            count(&|r, g, b| b > 140 && g > 120 && r < 140) > 20,
            "cian de las abejas y los rayos"
        );
        // Fuera de la oficina el suelo sigue el color de fondo: un azul noche (más claro que el negro de antes, que se perdía).
        let corner = &px[0..4];
        assert!(
            corner[0] < 45 && corner[1] < 65 && corner[2] < 100 && corner[2] > corner[0],
            "esquina azul noche: {corner:?}"
        );
        assert!(px.chunks(4).all(|p| p[3] == 255), "imagen opaca");
    }

    #[test]
    fn the_oracle_light_brightens_the_scene_around_it() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut gpu = OfficeGpu::new(&device, None);
        let size = (800, 500);
        let mut o = busy_office();
        o.graph.apply(
            "canvas:jev_status",
            &serde_json::json!({"state": "ready", "backend": "jev"}),
            None,
            0.0,
        );
        o.step(0.016);
        // Mismo fotograma con y sin la luz: la geometría del oráculo es idéntica, solo cambia la iluminación.
        let mut frame = crate::office::scene::build(&o);
        let draw = |gpu: &mut OfficeGpu, frame: &crate::office::scene::SceneFrame| {
            let mut enc = device.create_command_encoder(&Default::default());
            gpu.render(
                &device, &queue, &mut enc, frame, &o.camera, o.t, size, 1.0, true,
            );
            queue.submit([enc.finish()]);
            gpu.read_out(&device, &queue).expect("lectura").2
        };
        let lit = draw(&mut gpu, &frame);
        frame.oracle_light = None;
        let dark = draw(&mut gpu, &frame);
        let (l, d) = (luminance_sum(&lit), luminance_sum(&dark));
        assert!(
            l > d,
            "con la luz del oráculo la escena es más clara: {l} frente a {d}"
        );
        assert!(lit != dark);
    }

    #[test]
    fn rendering_the_same_state_twice_gives_the_same_image() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let o = busy_office();
        let mut gpu = OfficeGpu::new(&device, None);
        let a = render(&device, &queue, &mut gpu, &o, (480, 300), true).2;
        let b = render(&device, &queue, &mut gpu, &o, (480, 300), true).2;
        assert!(a == b, "el renderer es determinista para un mismo estado");
    }

    #[test]
    fn bloom_adds_light_and_high_quality_adds_more() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let o = busy_office();
        let mut gpu = OfficeGpu::new(&device, None);
        let high = luminance_sum(&render(&device, &queue, &mut gpu, &o, (480, 300), true).2);
        let low = luminance_sum(&render(&device, &queue, &mut gpu, &o, (480, 300), false).2);
        assert!(
            high > low,
            "más intensidad de bloom, más luz: {high} frente a {low}"
        );
    }

    #[test]
    fn the_vignette_darkens_the_edges() {
        let Some((device, queue)) = headless() else {
            return;
        };
        // Sin agentes la escena es el suelo y la reina: se compara una franja del borde con otra más adentro.
        let o = crate::office::state::OfficeState::default();
        let mut gpu = OfficeGpu::new(&device, None);
        let (w, h, px) = render(&device, &queue, &mut gpu, &o, (640, 400), true);
        let block = |x0: u32, y0: u32| -> u64 {
            (y0..y0 + 8)
                .flat_map(|y| (x0..x0 + 8).map(move |x| (x, y)))
                .map(|(x, y)| px[((y * w + x) * 4) as usize + 2] as u64)
                .sum()
        };
        let corner = block(0, 0);
        let inner = block(w / 4, 8);
        assert!(
            corner <= inner,
            "la viñeta oscurece la esquina: {corner} frente a {inner} ({h})"
        );
    }

    #[test]
    fn changing_the_size_recreates_the_targets_and_the_readback_matches() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let o = busy_office();
        let mut gpu = OfficeGpu::new(&device, None);
        assert_eq!(
            render(&device, &queue, &mut gpu, &o, (640, 400), true).0,
            640
        );
        assert_eq!(gpu.size(), Some((640, 400)));
        let (w, h, px) = render(&device, &queue, &mut gpu, &o, (800, 500), true);
        assert_eq!(
            (w, h, px.len(), gpu.size()),
            (800, 500, 800 * 500 * 4, Some((800, 500)))
        );
        // Un tamaño absurdo se limita en vez de reventar la memoria.
        let (w, h, _) = render(&device, &queue, &mut gpu, &o, (10, 10), true);
        assert_eq!((w, h), (32, 32));
    }

    #[test]
    fn a_selected_bee_and_the_camera_orbit_change_the_picture() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut o = busy_office();
        let mut gpu = OfficeGpu::new(&device, None);
        let base = render(&device, &queue, &mut gpu, &o, (480, 300), true).2;
        o.camera.orbit(200.0, 0.0);
        let orbited = render(&device, &queue, &mut gpu, &o, (480, 300), true).2;
        assert!(base != orbited, "orbitar cambia la imagen");
        o.camera.orbit(-200.0, 0.0);
        o.selected = Some("a1".into());
        let selected = render(&device, &queue, &mut gpu, &o, (480, 300), true).2;
        assert!(base != selected, "seleccionar una abeja la hace más grande");
    }

    #[test]
    fn the_oracle_the_wireframes_and_a_flying_packet_show_up_on_the_real_gpu() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut gpu = OfficeGpu::new(&device, None);
        let size = (800, 500);
        let mut o = busy_office();
        let base = render(&device, &queue, &mut gpu, &o, size, true).2;

        // Oráculo de Jev: violeta (#a78bfa) por encima de la reina.
        o.graph.apply(
            "canvas:jev_status",
            &serde_json::json!({"state": "ready", "backend": "jev"}),
            None,
            0.0,
        );
        o.step(0.016);
        let with_oracle = render(&device, &queue, &mut gpu, &o, size, true).2;
        let violet = |px: &[u8]| {
            px.chunks(4)
                .filter(|p| p[2] > 150 && p[0] > 90 && p[1] < p[2] - 40 && p[0] < p[2])
                .count()
        };
        assert!(
            violet(&with_oracle) > violet(&base) + 30,
            "aparecen píxeles violeta: {} frente a {}",
            violet(&with_oracle),
            violet(&base)
        );

        // Holograma de herramienta (alambre): cambia la imagen respecto a una abeja sin él.
        let mut tool = busy_office();
        tool.graph
            .nodes
            .iter_mut()
            .find(|n| n.id == "a1")
            .unwrap()
            .data = serde_json::json!({"currentTask": "t", "currentTool": "web_search"});
        let wired = render(&device, &queue, &mut gpu, &tool, size, true).2;
        assert!(wired != base, "el alambre del holograma se dibuja");

        // Paquete de trabajo en vuelo.
        let mut flying = busy_office();
        flying.graph.apply("canvas:work_event", &serde_json::json!({"eventId": "f", "phase": "failed", "actorId": "boss", "targetId": "a2"}), None, 0.0);
        flying.step(0.016);
        flying.step(0.5);
        let packet = render(&device, &queue, &mut gpu, &flying, size, true).2;
        let rose = |px: &[u8]| {
            px.chunks(4)
                .filter(|p| p[0] > 200 && p[1] > 70 && p[1] < 190 && p[2] > 90 && p[0] > p[1] + 50)
                .count()
        };
        assert!(
            rose(&packet) > rose(&base) + 20,
            "el paquete de una alerta es rosado: {} frente a {}",
            rose(&packet),
            rose(&base)
        );
    }

    #[test]
    fn sprites_and_trails_are_painted_on_the_real_gpu() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut gpu = OfficeGpu::new(&device, None);
        let size = (800, 500);
        let mut o = busy_office();
        o.step(1.0 / 60.0);
        let frame = crate::office::scene::build(&o);
        assert!(
            frame
                .draws
                .iter()
                .filter(|d| d.material == crate::office::scene::Material::Sprite)
                .count()
                > 600,
            "hay partículas que pintar"
        );
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(
            &device, &queue, &mut enc, &frame, &o.camera, o.t, size, 1.0, true,
        );
        queue.submit([enc.finish()]);
        let with = gpu.read_out(&device, &queue).unwrap().2;

        let mut bare = frame.clone();
        bare.draws
            .retain(|d| d.material != crate::office::scene::Material::Sprite);
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(
            &device, &queue, &mut enc, &bare, &o.camera, o.t, size, 1.0, true,
        );
        queue.submit([enc.finish()]);
        let without = gpu.read_out(&device, &queue).unwrap().2;
        assert!(with != without, "los sprites cambian la imagen");
        assert!(
            luminance_sum(&with) > luminance_sum(&without),
            "y suman luz (son aditivos)"
        );

        // El tamaño en píxeles escala con la densidad de pantalla: a 2× los discos son más grandes y suman más luz.
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(
            &device, &queue, &mut enc, &frame, &o.camera, o.t, size, 2.0, true,
        );
        queue.submit([enc.finish()]);
        let dense = gpu.read_out(&device, &queue).unwrap().2;
        assert!(
            luminance_sum(&dense) > luminance_sum(&with),
            "a doble densidad, sprites más grandes"
        );
    }

    /// Medida (no es una prueba de corrección): `cargo test -- --ignored --nocapture frame_time`.
    #[test]
    #[ignore]
    fn frame_time_of_a_busy_office() {
        let Some((device, queue)) = headless() else {
            return;
        };
        let mut o = busy_office();
        let mut gpu = OfficeGpu::new(&device, None);
        for size in [(960, 600), (1920, 1080)] {
            let warm = crate::office::scene::build(&o);
            let mut enc = device.create_command_encoder(&Default::default());
            gpu.render(
                &device, &queue, &mut enc, &warm, &o.camera, o.t, size, 1.0, true,
            );
            queue.submit([enc.finish()]);
            device.poll(wgpu::PollType::wait_indefinitely()).ok();

            let frames = 120;
            let (mut cpu, start) = (std::time::Duration::ZERO, std::time::Instant::now());
            for _ in 0..frames {
                let t0 = std::time::Instant::now();
                o.step(1.0 / 60.0);
                let frame = crate::office::scene::build(&o);
                let mut enc = device.create_command_encoder(&Default::default());
                gpu.render(
                    &device, &queue, &mut enc, &frame, &o.camera, o.t, size, 1.0, true,
                );
                queue.submit([enc.finish()]);
                cpu += t0.elapsed();
                device.poll(wgpu::PollType::wait_indefinitely()).ok();
            }
            let total = start.elapsed();
            eprintln!(
                "{}x{}: {:.2} ms por fotograma (CPU preparando: {:.2} ms), {} dibujos",
                size.0,
                size.1,
                total.as_secs_f64() * 1000.0 / frames as f64,
                cpu.as_secs_f64() * 1000.0 / frames as f64,
                crate::office::scene::build(&o).draws.len()
            );
        }
    }
}
