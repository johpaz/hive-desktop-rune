//! Spike 3D: ¿se puede meter una escena 3D propia (con profundidad) dentro de un elemento de Rune?
//!
//! El pase donde Rune pinta un `effect` slot no tiene búfer de profundidad, así que la escena se
//! renderiza antes, a una textura propia con `Depth32Float`, y el hueco solo la pinta como un quad.
//! `cargo run --bin spike3d` abre la ventana; `-- --snap out.png` la renderiza sin ventana.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use rune::render::EffectSite;
use rune::tok::type_;
use rune::{col, effect, text, AppSpec, Compositor, Cx, El, Frame};
use wgpu::util::DeviceExt;

const MARK: u32 = 7;
const W: u32 = 960;
const H: u32 = 600;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

const SCENE_WGSL: &str = r#"
struct U { vp: mat4x4<f32>, t: f32, _p0: f32, _p1: f32, _p2: f32 };
@group(0) @binding(0) var<uniform> u: U;
struct VIn { @location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>, @location(2) off: vec3<f32>, @location(3) phase: f32 };
struct VOut { @builtin(position) clip: vec4<f32>, @location(0) nrm: vec3<f32>, @location(1) h: f32, @location(2) y: f32 };
@vertex fn vs(v: VIn) -> VOut {
  let h = 0.25 + 1.6 * (0.5 + 0.5 * sin(u.t * 1.6 - v.phase * 0.9));
  let p = vec3<f32>(v.pos.x + v.off.x, v.pos.y * h, v.pos.z + v.off.z);
  var o: VOut;
  o.clip = u.vp * vec4<f32>(p, 1.0);
  o.nrm = v.nrm; o.h = h; o.y = v.pos.y;
  return o;
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let l = normalize(vec3<f32>(0.4, 1.0, 0.3));
  let d = max(dot(normalize(i.nrm), l), 0.0);
  let gold = mix(vec3<f32>(0.35, 0.22, 0.04), vec3<f32>(1.0, 0.72, 0.18), clamp(i.h / 1.85, 0.0, 1.0));
  let top = select(0.0, 0.35, i.nrm.y > 0.5);
  return vec4<f32>(gold * (0.25 + 0.75 * d) + vec3<f32>(top, top * 0.6, 0.0), 1.0);
}
"#;

const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
struct O { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> O {
  let x = f32((i << 1u) & 2u); let y = f32(i & 2u);
  var o: O; o.p = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0); o.uv = vec2<f32>(x, y); return o;
}
@fragment fn fs(i: O) -> @location(0) vec4<f32> { return textureSample(tex, smp, i.uv); }
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    vp: [f32; 16],
    t: f32,
    _p: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vert {
    pos: [f32; 3],
    nrm: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    off: [f32; 3],
    phase: f32,
}

/// Prisma hexagonal de radio 0.5 y altura 1 (y de 0 a 1): 6 caras laterales y la tapa.
fn hex_prism() -> Vec<Vert> {
    let r = 0.5f32;
    let c = |i: usize| {
        let a = std::f32::consts::TAU * i as f32 / 6.0;
        (r * a.cos(), r * a.sin())
    };
    let mut v = Vec::new();
    for i in 0..6 {
        let ((x0, z0), (x1, z1)) = (c(i), c(i + 1));
        let a = std::f32::consts::TAU * (i as f32 + 0.5) / 6.0;
        let n = [a.cos(), 0.0, a.sin()];
        for (x, y, z) in [
            (x0, 0.0, z0),
            (x1, 0.0, z1),
            (x1, 1.0, z1),
            (x0, 0.0, z0),
            (x1, 1.0, z1),
            (x0, 1.0, z0),
        ] {
            v.push(Vert {
                pos: [x, y, z],
                nrm: n,
            });
        }
        v.push(Vert {
            pos: [0.0, 1.0, 0.0],
            nrm: [0.0, 1.0, 0.0],
        });
        v.push(Vert {
            pos: [x1, 1.0, z1],
            nrm: [0.0, 1.0, 0.0],
        });
        v.push(Vert {
            pos: [x0, 1.0, z0],
            nrm: [0.0, 1.0, 0.0],
        });
    }
    v
}

/// Malla hexagonal de `rings` anillos alrededor del origen.
fn grid(rings: i32) -> Vec<Inst> {
    let mut out = Vec::new();
    for q in -rings..=rings {
        for r in (-rings).max(-q - rings)..=rings.min(-q + rings) {
            let x = 1.0 * (q as f32 + r as f32 / 2.0) * 0.8660254 * 2.0 / 1.7320508 * 1.732;
            let z = 1.5 * r as f32 * 0.5773503 * 1.1547;
            out.push(Inst {
                off: [x * 0.58, 0.0, z * 0.9],
                phase: (x * x + z * z).sqrt(),
            });
        }
    }
    out
}

type M = [f32; 16];
fn mul(a: &M, b: &M) -> M {
    let mut o = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            for k in 0..4 {
                o[c * 4 + r] += a[k * 4 + r] * b[c * 4 + k];
            }
        }
    }
    o
}
fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> M {
    let f = 1.0 / (fov / 2.0).tan();
    [
        f / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        f,
        0.0,
        0.0,
        0.0,
        0.0,
        far / (near - far),
        -1.0,
        0.0,
        0.0,
        far * near / (near - far),
        0.0,
    ]
}
fn look_at(eye: [f32; 3], at: [f32; 3]) -> M {
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = norm(sub(at, eye));
    let s = norm(cross(f, [0.0, 1.0, 0.0]));
    let u = cross(s, f);
    [
        s[0],
        u[0],
        -f[0],
        0.0,
        s[1],
        u[1],
        -f[1],
        0.0,
        s[2],
        u[2],
        -f[2],
        0.0,
        -dot(s, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

struct Gpu {
    key: rune::compose::DeviceKey,
    scene: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    mesh: wgpu::Buffer,
    mesh_len: u32,
    insts: wgpu::Buffer,
    inst_len: u32,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    blit_module: wgpu::ShaderModule,
    blit_layout: wgpu::PipelineLayout,
    blit_bind: wgpu::BindGroup,
    blits: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Gpu {
    fn new(device: &wgpu::Device, key: rune::compose::DeviceKey) -> Gpu {
        let tex = |format, label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: W,
                        height: H,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let (color, depth) = (tex(COLOR, "spike color"), tex(DEPTH, "spike depth"));

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(SCENE_WGSL.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&bgl)],
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
        let inst_attrs = [
            attr(2, 0, wgpu::VertexFormat::Float32x3),
            attr(3, 12, wgpu::VertexFormat::Float32),
        ];
        let scene = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&layout),
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
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &inst_attrs,
                    }),
                ],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
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
                    format: COLOR,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let mesh_v = hex_prism();
        let inst_v = grid(7);
        let mesh = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh"),
            contents: bytemuck::cast_slice(&mesh_v),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let insts = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("inst"),
            contents: bytemuck::cast_slice(&inst_v),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
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
            label: Some("blit"),
            bind_group_layouts: &[Some(&blit_bgl)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blit_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Gpu {
            key,
            scene,
            uniform,
            bind,
            mesh,
            mesh_len: mesh_v.len() as u32,
            insts,
            inst_len: inst_v.len() as u32,
            color,
            depth,
            blit_module,
            blit_layout,
            blit_bind,
            blits: HashMap::new(),
        }
    }

    /// Dibuja la escena en su textura (antes del pase de la interfaz).
    fn render_scene(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, t: f32) {
        let a = t * 0.25;
        let eye = [a.cos() * 11.0, 8.0, a.sin() * 11.0];
        let vp = mul(
            &perspective(0.8, W as f32 / H as f32, 0.1, 100.0),
            &look_at(eye, [0.0, 0.4, 0.0]),
        );
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Uniform {
                vp,
                t,
                _p: [0.0; 3],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spike scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.01,
                        g: 0.012,
                        b: 0.02,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
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
        pass.set_pipeline(&self.scene);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(0, self.mesh.slice(..));
        pass.set_vertex_buffer(1, self.insts.slice(..));
        pass.draw(0..self.mesh_len, 0..self.inst_len);
    }

    /// Pinta la textura en el hueco: el viewport lo recorta a la caja del elemento.
    fn blit(
        &mut self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        site: &EffectSite<'_>,
        rect: [f32; 4],
    ) {
        let pipe = self.blits.entry(site.format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("blit"),
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
        pass.set_bind_group(0, &self.blit_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[derive(Default)]
struct State;

struct Stage {
    gpu: Option<Gpu>,
    t: f32,
}

impl Compositor<State> for Stage {
    fn render(&mut self, f: &mut Frame<'_, State>) {
        let key = f.device_key();
        if self.gpu.as_ref().map(|g| g.key) != Some(key) {
            self.gpu = Some(Gpu::new(f.device, key));
        }
        self.t += f.dt as f32;
        let gpu = self.gpu.as_mut().unwrap();
        gpu.render_scene(f.queue, f.encoder, 1.2 + self.t);

        let clear = f.rt.clear_color();
        let rect = f.rt.presented("scene").map(|(r, _)| r).unwrap_or([0.0; 4]);
        let device = f.device;
        f.ui.prepare(
            device,
            f.queue,
            &mut f.rt.text.atlas,
            &f.rt.instances,
            f.size,
        );
        f.ui.draw_with(f.encoder, f.target, clear, &mut |pass, site| {
            gpu.blit(device, pass, site, rect)
        });
    }

    fn animating(&self) -> bool {
        true
    }
}

fn view(_: &State, _: &mut Cx<State>) -> El {
    let t = rune::tok::theme(true);
    let slot = effect(MARK)
        .key("scene")
        .w(480.0)
        .h(300.0)
        .radius(14.0)
        .child(
            text("Oficina · 91 prismas")
                .font(type_::LABEL)
                .color(t.text)
                .absolute(14.0, 12.0),
        );
    col()
        .w_full()
        .h_full()
        .padding(20.0)
        .gap(12.0)
        .fill(t.bg)
        .child(text("Spike 3D sobre Rune").font(type_::TITLE).color(t.text))
        .child(slot)
        .child(
            text("Escena con profundidad en textura propia, compuesta en un hueco de la interfaz")
                .font(type_::LABEL)
                .color(t.text_secondary),
        )
}

fn main() {
    rune::run_with(
        AppSpec::<State> {
            scenes: &[],
            ..AppSpec::new("Spike 3D", (520.0, 420.0), State::default, view)
        },
        Stage { gpu: None, t: 0.0 },
    );
}
