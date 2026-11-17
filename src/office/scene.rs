//! Lista de dibujos de la oficina: de la simulación sale, cada fotograma, qué mallas pintar, con qué
//! material y dónde. Es lógica pura (sin GPU) para poder probarla. Cifras de `scene/*.tsx` de la UI web.

use std::f32::consts::{FRAC_PI_2, PI};

use super::camera::{Mat4, Vec3};
use super::effects::{self, Sprite, ORACLE_POSITION};
use super::graph::{Interaction, InteractionKind};
use super::mesh::Mesh;
use super::model::{self, tool_kind, DeskState, ToolKind};
use super::state::{tune, OfficeState};
use super::xform::{
    align_y, chain, euler, look_along, rot_x, rot_y, rot_z, scale, translate, IDENTITY,
};

/// Posición fija del coordinador como origen o destino de rayos y paquetes.
pub const COORDINATOR_POINT: Vec3 = [0.0, 4.4, 0.0];

fn milli(v: f32) -> u32 {
    (v * 1000.0).round() as u32
}

/// Malla paramétrica; las medidas van en milésimas para poder usarla como clave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Disc {
        r: u32,
        seg: u16,
    },
    Sphere {
        w: u16,
        h: u16,
    },
    Torus {
        r: u32,
        tube: u32,
        radial: u16,
        tubular: u16,
        arc: u32,
    },
    Cylinder {
        rt: u32,
        rb: u32,
        h: u32,
        radial: u16,
        open: bool,
    },
    Cone {
        r: u32,
        h: u32,
        radial: u16,
    },
    Ring {
        inner: u32,
        outer: u32,
        seg: u16,
    },
    Cuboid {
        x: u32,
        y: u32,
        z: u32,
    },
    Capsule {
        r: u32,
        len: u32,
        seg: u16,
    },
    Octa {
        r: u32,
    },
    Ico {
        r: u32,
    },
    /// Cuadrado unidad para sprites.
    Quad,
}

impl Shape {
    pub fn sphere(w: u16, h: u16) -> Shape {
        Shape::Sphere { w, h }
    }
    pub fn torus(r: f32, tube: f32, radial: u16, tubular: u16) -> Shape {
        Shape::Torus {
            r: milli(r),
            tube: milli(tube),
            radial,
            tubular,
            arc: milli(std::f32::consts::TAU),
        }
    }
    pub fn cylinder(rt: f32, rb: f32, h: f32, radial: u16) -> Shape {
        Shape::Cylinder {
            rt: milli(rt),
            rb: milli(rb),
            h: milli(h),
            radial,
            open: false,
        }
    }
    pub fn cone(r: f32, h: f32, radial: u16) -> Shape {
        Shape::Cone {
            r: milli(r),
            h: milli(h),
            radial,
        }
    }
    pub fn ring(inner: f32, outer: f32, seg: u16) -> Shape {
        Shape::Ring {
            inner: milli(inner),
            outer: milli(outer),
            seg,
        }
    }

    pub fn build(&self) -> Mesh {
        let f = |v: u32| v as f32 / 1000.0;
        match *self {
            Shape::Disc { r, seg } => Mesh::disc(f(r), seg as usize),
            Shape::Sphere { w, h } => Mesh::sphere(1.0, w as usize, h as usize),
            Shape::Torus {
                r,
                tube,
                radial,
                tubular,
                arc,
            } => Mesh::torus(f(r), f(tube), radial as usize, tubular as usize, f(arc)),
            Shape::Cylinder {
                rt,
                rb,
                h,
                radial,
                open,
            } => Mesh::cylinder(f(rt), f(rb), f(h), radial as usize, open),
            Shape::Cone { r, h, radial } => Mesh::cone(f(r), f(h), radial as usize),
            Shape::Ring { inner, outer, seg } => Mesh::ring(f(inner), f(outer), seg as usize),
            Shape::Cuboid { x, y, z } => Mesh::cuboid([f(x), f(y), f(z)]),
            Shape::Capsule { r, len, seg } => Mesh::capsule(f(r), f(len), seg as usize),
            Shape::Octa { r } => Mesh::octahedron(f(r)),
            Shape::Ico { r } => Mesh::icosahedron(f(r)),
            Shape::Quad => Mesh::quad(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    /// Suelo hexagonal: aditivo, sin escribir profundidad.
    Floor,
    /// Iluminado y opaco.
    Solid,
    /// Iluminado y translúcido (alas), de doble cara.
    SolidAlpha,
    /// Sin iluminación y opaco.
    Basic,
    /// Sin iluminación y aditivo.
    BasicAdd,
    Hologram,
    Beam,
    /// Cuadrado que mira a la cámara con un disco suave (partículas): `p0.x` tamaño en píxeles, `p0.y` atenuación.
    Sprite,
    /// Aristas, aditivo.
    WireAdd,
}

/// Un dibujo: malla, material, transformación y parámetros que cada material interpreta a su manera.
///
/// - `Solid`/`SolidAlpha`: `color.rgb` base, `color.a` opacidad; `p0` = emisivo rgb + intensidad; `p1.x` metal, `p1.y` rugosidad.
/// - `Hologram`: `color.a` opacidad; `p0.x` intensidad, `p0.y` glitch.
/// - `Basic`/`BasicAdd`/`WireAdd`: `color` con su opacidad.
/// - `Beam`: `p0.x` velocidad, `p0.y` intensidad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub shape: Shape,
    pub material: Material,
    pub model: Mat4,
    pub color: [f32; 4],
    pub p0: [f32; 4],
    pub p1: [f32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct SceneFrame {
    pub draws: Vec<Draw>,
    /// Luz puntual de la reina: posición e intensidad (candelas).
    pub queen_light: (Vec3, f32),
    /// Luz puntual del oráculo de Jev: posición, color lineal e intensidad. `None` sin Jev.
    pub oracle_light: Option<(Vec3, [f32; 3], f32)>,
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Color `#rrggbb` en sRGB a RGB lineal.
pub fn lin(hex: u32) -> [f32; 3] {
    let c = model::hex(hex);
    [
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
    ]
}

fn lin_rgb(c: [f32; 3]) -> [f32; 3] {
    [
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
    ]
}

#[derive(Default)]
struct Builder {
    out: Vec<Draw>,
}

impl Builder {
    fn push(
        &mut self,
        shape: Shape,
        material: Material,
        model: Mat4,
        color: [f32; 4],
        p0: [f32; 4],
        p1: [f32; 4],
    ) {
        self.out.push(Draw {
            shape,
            material,
            model,
            color,
            p0,
            p1,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn solid(
        &mut self,
        shape: Shape,
        model: Mat4,
        color: u32,
        emissive: u32,
        e: f32,
        metal: f32,
        rough: f32,
    ) {
        let (c, em) = (lin(color), lin(emissive));
        self.push(
            shape,
            Material::Solid,
            model,
            [c[0], c[1], c[2], 1.0],
            [em[0], em[1], em[2], e],
            [metal, rough, 0.0, 0.0],
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn solid_alpha(
        &mut self,
        shape: Shape,
        model: Mat4,
        color: u32,
        emissive: u32,
        e: f32,
        opacity: f32,
    ) {
        let (c, em) = (lin(color), lin(emissive));
        self.push(
            shape,
            Material::SolidAlpha,
            model,
            [c[0], c[1], c[2], opacity],
            [em[0], em[1], em[2], e],
            [0.0, 0.6, 0.0, 0.0],
        );
    }

    fn basic(&mut self, shape: Shape, model: Mat4, color: u32) {
        let c = lin(color);
        self.push(
            shape,
            Material::Basic,
            model,
            [c[0], c[1], c[2], 1.0],
            [0.0; 4],
            [0.0; 4],
        );
    }

    fn basic_add(&mut self, shape: Shape, model: Mat4, color: [f32; 3], opacity: f32) {
        self.push(
            shape,
            Material::BasicAdd,
            model,
            [color[0], color[1], color[2], opacity],
            [0.0; 4],
            [0.0; 4],
        );
    }

    fn holo(
        &mut self,
        shape: Shape,
        model: Mat4,
        color: [f32; 3],
        intensity: f32,
        glitch: f32,
        opacity: f32,
    ) {
        self.push(
            shape,
            Material::Hologram,
            model,
            [color[0], color[1], color[2], opacity],
            [intensity, glitch, 0.0, 0.0],
            [0.0; 4],
        );
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

const AMBER: u32 = 0xf59e0b;

/// La reina y su pedestal. Devuelve la posición de su luz puntual.
fn queen(b: &mut Builder, o: &OfficeState, processing: bool, active: bool) -> Vec3 {
    let sel = o.selected.is_some()
        && o.selected.as_deref() == o.desks().0.as_ref().map(|c| c.id.as_str());
    let pick = |normal: f32, selected: f32| if sel { selected } else { normal };
    let sphere = Shape::sphere(24, 18);

    // Pedestal hexagonal y su anillo, en el suelo.
    b.solid(
        Shape::cylinder(2.5, 2.8, 0.45, 6),
        translate([0.0, 0.3, 0.0]),
        0x0b0f1a,
        AMBER,
        pick(0.18, 0.5),
        0.7,
        0.35,
    );
    let ring_op = pick(0.48, 0.9);
    b.basic_add(
        Shape::ring(1.85, 2.22, 6),
        chain(&[translate([0.0, 0.54, 0.0]), rot_x(-FRAC_PI_2)]),
        lin(AMBER),
        ring_op,
    );

    // Cuerpo: un grupo que respira, gira un poco y puede crecer.
    let breathe = if o.calm && (active || processing) {
        (2.1 * o.t).sin() * 0.04
    } else {
        0.0
    };
    let g = chain(&[
        translate([0.0, 1.05 + breathe, 0.0]),
        rot_y(o.queen.yaw),
        scale([o.queen.scale; 3]),
    ]);
    let at = |m: Mat4| chain(&[g, m]);

    // Abdomen, rayas y aguijón.
    b.solid(
        sphere,
        at(chain(&[
            translate([0.0, 1.2, 0.0]),
            scale([0.72 * 0.74, 0.72 * 1.28, 0.72 * 0.68]),
        ])),
        0xe6a519,
        AMBER,
        pick(0.42, 0.8),
        0.34,
        0.36,
    );
    for (i, y) in [0.58f32, 1.02, 1.46].iter().enumerate() {
        b.solid(
            Shape::torus(0.5 - i as f32 * 0.035, 0.075, 8, 28),
            at(chain(&[translate([0.0, *y, 0.0]), rot_x(FRAC_PI_2)])),
            0x251807,
            0x000000,
            0.0,
            0.52,
            0.48,
        );
    }
    b.solid(
        Shape::cone(0.14, 0.52, 8),
        at(chain(&[translate([0.0, -0.02, 0.0]), rot_z(PI)])),
        0xffcf6a,
        AMBER,
        0.5,
        0.3,
        0.4,
    );

    // Tórax, cabeza y ojos.
    b.solid(
        sphere,
        at(chain(&[
            translate([0.0, 2.25, 0.0]),
            scale([0.7, 0.7 * 0.9, 0.7 * 0.82]),
        ])),
        0x593c12,
        AMBER,
        pick(0.24, 0.55),
        0.48,
        0.42,
    );
    b.solid(
        sphere,
        at(chain(&[translate([0.0, 3.15, 0.0]), scale([0.54; 3])])),
        0xd69113,
        AMBER,
        pick(0.48, 0.95),
        0.3,
        0.4,
    );
    for s in [-1.0f32, 1.0] {
        b.basic(
            sphere,
            at(chain(&[
                translate([0.34 * s, 3.2, 0.4]),
                scale([0.15, 0.15 * 1.18, 0.15 * 0.42]),
            ])),
            0xfff1a8,
        );
    }

    // Antenas.
    for s in [-1.0f32, 1.0] {
        // Las antenas vibran cuando la reina interviene en el evento actual.
        let wobble = if o.queen.involved && o.calm {
            (7.0 * o.t).sin() * 0.08
        } else {
            0.0
        };
        let ant = chain(&[translate([0.0, 3.55, 0.0]), rot_z(-0.36 * s + wobble)]);
        b.basic(
            Shape::Capsule {
                r: 35,
                len: 420,
                seg: 6,
            },
            at(chain(&[ant, translate([0.22 * s, 0.28, 0.0])])),
            0xffd465,
        );
        b.basic(
            sphere,
            at(chain(&[
                ant,
                translate([0.32 * s, 0.53, 0.0]),
                scale([0.07; 3]),
            ])),
            0xfff0a6,
        );
    }

    // Alas: más rápidas y amplias cuanto más trabajo hay.
    let (freq, amp) = if o.queen.involved {
        (7.0, 0.34)
    } else if active || processing {
        (4.2, 0.24)
    } else {
        (1.4, 0.08)
    };
    let flap = if o.calm {
        (o.t * freq).sin() * amp
    } else {
        0.0
    };
    for s in [-1.0f32, 1.0] {
        let wing = chain(&[translate([0.48 * s, 2.45, 0.0]), rot_z(s * (0.28 + flap))]);
        b.solid_alpha(
            sphere,
            at(chain(&[
                wing,
                translate([0.62 * s, 0.18, 0.0]),
                euler(0.08, 0.15 * s, -0.18 * s),
                scale([0.48 * 1.65, 0.48 * 0.12, 0.48 * 0.72]),
            ])),
            0xe8f8ff,
            0x8cdcff,
            0.58,
            0.48,
        );
        b.solid_alpha(
            sphere,
            at(chain(&[
                wing,
                translate([0.46 * s, -0.34, 0.05]),
                euler(0.08, 0.1 * s, 0.2 * s),
                scale([0.4 * 1.25, 0.4 * 0.1, 0.4 * 0.58]),
            ])),
            0xdff7ff,
            0x77c8ff,
            0.42,
            0.38,
        );
    }

    // Corona que gira.
    let crown = chain(&[translate([0.0, 4.08, 0.0]), rot_y(o.queen.crown)]);
    b.basic_add(
        Shape::torus(0.48, 0.055, 6, 6),
        at(chain(&[crown, rot_x(FRAC_PI_2)])),
        lin(0xffe38a),
        1.0,
    );
    for (x, y, h) in [
        (-0.4f32, 0.14f32, 0.34f32),
        (0.0, 0.25, 0.48),
        (0.4, 0.14, 0.34),
    ] {
        b.solid(
            Shape::cone(0.11, h, 5),
            at(chain(&[crown, translate([x, y, 0.0])])),
            0xffd465,
            0xffd465,
            0.9,
            0.6,
            0.2,
        );
    }

    // Su luz puntual, dentro del grupo.
    super::xform::apply(&g, [0.0, 3.0, 0.0])
}

/// Una abeja: cuerpo holográfico, rayas, alas que baten y, según su estado, un aro.
fn bee(b: &mut Builder, o: &OfficeState, id: &str, desk: &model::Desk) {
    let Some(sim) = o.bees.get(id) else { return };
    let selected = o.selected.as_deref() == Some(id);
    let s = if selected { 0.85 } else { 0.62 };
    let base = chain(&[translate(sim.bee.pos), look_along(sim.dir), scale([s; 3])]);
    // El gesto (cabeceo y escala) afecta al cuerpo; los aros y el holograma se quedan en el grupo de la abeja.
    let body = chain(&[base, rot_x(sim.pitch), scale([sim.gscale; 3])]);
    let color = lin_rgb(desk.color);
    let vis = sim.vis;
    let sphere = Shape::sphere(14, 14);

    b.holo(
        sphere,
        chain(&[
            body,
            translate([0.0, 0.0, 0.0]),
            scale([0.32 * 0.85, 0.32 * 0.75, 0.32 * 1.35]),
        ]),
        color,
        vis.intensity,
        vis.glitch,
        vis.opacity,
    );
    b.solid(
        Shape::torus(0.24, 0.045, 8, 20),
        chain(&[body, translate([0.0, 0.0, 0.10])]),
        0x241804,
        0x000000,
        0.0,
        0.4,
        0.6,
    );
    b.solid(
        Shape::torus(0.22, 0.04, 8, 20),
        chain(&[body, translate([0.0, 0.0, -0.14])]),
        0x241804,
        0x000000,
        0.0,
        0.4,
        0.6,
    );
    b.holo(
        Shape::sphere(12, 12),
        chain(&[body, translate([0.0, 0.03, 0.5]), scale([0.18; 3])]),
        color,
        vis.intensity,
        vis.glitch,
        vis.opacity,
    );
    b.holo(
        Shape::cone(0.07, 0.22, 8),
        chain(&[body, translate([0.0, 0.0, -0.52]), rot_x(-FRAC_PI_2)]),
        color,
        vis.intensity,
        vis.glitch,
        vis.opacity,
    );

    // Alas: baten solo si hay movimiento y la abeja está activa.
    let flap = tune(desk.state).flap;
    let angle = if o.calm && flap > 0.0 {
        (o.t * flap).sin() * 0.45 + 0.2
    } else {
        0.15
    };
    let wing_color = lerp3(color, [1.0, 1.0, 1.0], 0.55);
    for side in [-1.0f32, 1.0] {
        b.holo(
            Shape::sphere(10, 10),
            chain(&[
                body,
                translate([0.26 * side, 0.24, 0.05]),
                euler(0.35, 0.0, side * angle),
                scale([0.3, 0.3 * 0.08, 0.3 * 0.55]),
            ]),
            wing_color,
            0.9,
            vis.glitch,
            vis.wing_opacity,
        );
    }

    // Holograma de la herramienta en uso: gira sobre la abeja, en alambre y con el color del agente.
    if let (DeskState::ToolCall, Some(tool)) = (desk.state, desk.current_tool.as_deref()) {
        tool_hologram(b, o, base, color, tool);
    }

    // Decoración por estado.
    match desk.state {
        DeskState::Thinking => b.basic_add(
            Shape::ring(0.42, 0.52, 6),
            chain(&[base, translate([0.0, 0.95, 0.0]), rot_x(-FRAC_PI_2)]),
            color,
            0.42,
        ),
        DeskState::Stuck => b.basic_add(
            Shape::ring(0.52, 0.68, 6),
            chain(&[base, translate([0.0, 1.05, 0.0]), rot_x(-FRAC_PI_2)]),
            lin(0xfb7185),
            0.86,
        ),
        _ => {}
    }
}

fn sprite(b: &mut Builder, sp: &Sprite) {
    b.push(
        Shape::Quad,
        Material::Sprite,
        translate(sp.pos),
        [sp.color[0], sp.color[1], sp.color[2], sp.alpha],
        [sp.size, sp.falloff, 0.0, 0.0],
        [0.0; 4],
    );
}

fn wire(b: &mut Builder, shape: Shape, model: Mat4, color: [f32; 3], opacity: f32) {
    b.push(
        shape,
        Material::WireAdd,
        model,
        [color[0], color[1], color[2], opacity],
        [0.0; 4],
        [0.0; 4],
    );
}

/// Figura de alambre según la categoría de la herramienta; flota y gira a 0,85 rad/s dentro del grupo de la abeja.
fn tool_hologram(b: &mut Builder, o: &OfficeState, base: Mat4, color: [f32; 3], tool: &str) {
    let g = chain(&[
        base,
        translate([0.0, 1.05 + (2.4 * o.t).sin() * 0.08, 0.0]),
        rot_y(o.t * 0.85),
        scale([0.7; 3]),
    ]);
    let op = 0.72;
    let flat_ring = |r: f32, tube: f32, radial: u16, tubular: u16| {
        (Shape::torus(r, tube, radial, tubular), rot_x(FRAC_PI_2))
    };
    match tool_kind(tool) {
        ToolKind::Browser => {
            wire(
                b,
                Shape::sphere(10, 8),
                chain(&[g, scale([0.42; 3])]),
                color,
                op,
            );
            let (t, r) = flat_ring(0.53, 0.035, 6, 24);
            wire(b, t, chain(&[g, r]), color, op);
        }
        ToolKind::Code => {
            wire(
                b,
                Shape::Cuboid {
                    x: 750,
                    y: 550,
                    z: 120,
                },
                g,
                color,
                op,
            );
            for s in [-1.0f32, 1.0] {
                wire(
                    b,
                    Shape::Cuboid {
                        x: 60,
                        y: 320,
                        z: 50,
                    },
                    chain(&[g, translate([0.16 * s, 0.0, 0.1]), rot_z(0.55 * s)]),
                    color,
                    op,
                );
            }
        }
        ToolKind::Knowledge => {
            wire(b, Shape::Octa { r: 460 }, g, color, op);
            let (t, r) = flat_ring(0.58, 0.025, 6, 6);
            wire(b, t, chain(&[g, r]), color, op);
        }
        ToolKind::Communication => {
            for r in [0.25f32, 0.42, 0.59] {
                let half = Shape::Torus {
                    r: milli(r),
                    tube: 25,
                    radial: 5,
                    tubular: 20,
                    arc: milli(PI),
                };
                wire(b, half, chain(&[g, rot_y(FRAC_PI_2)]), color, op);
            }
        }
        ToolKind::Generic => {
            wire(b, Shape::Ico { r: 400 }, g, color, op);
            let (t, r) = flat_ring(0.56, 0.025, 5, 24);
            wire(b, t, chain(&[g, r]), color, op);
        }
    }
}

/// Dónde está un agente a efectos de rayos y paquetes: el coordinador, en su punto fijo; los demás, su abeja.
pub fn point_of(o: &OfficeState, id: &str, coordinator_id: Option<&str>) -> Option<Vec3> {
    if Some(id) == coordinator_id {
        Some(COORDINATOR_POINT)
    } else {
        o.bee_pos(id)
    }
}

/// Paquetes de trabajo en vuelo, con los anillos de pulso del origen.
fn transfers(
    b: &mut Builder,
    o: &OfficeState,
    coordinator_id: Option<&str>,
    desks: &[model::Desk],
) {
    for f in &o.transfers.flights {
        let e = &f.event;
        let (Some(from), Some(to)) = (
            point_of(o, &e.actor_id, coordinator_id),
            point_of(
                o,
                e.target_id.as_deref().or(coordinator_id).unwrap_or(""),
                coordinator_id,
            ),
        ) else {
            continue;
        };
        let desk_color = |id: &str| desks.iter().find(|d| d.id == id).map(|d| d.color);
        let color = lin_rgb(effects::packet_color(
            &e.phase,
            e.target_id.as_deref().and_then(desk_color),
            desk_color(&e.actor_id),
        ));
        let elapsed = o.t - f.started;
        let progress = effects::progress(o.t, f.started);
        let (pos, _) = effects::packet_pose(from, to, progress);
        b.basic_add(
            Shape::Octa { r: 300 },
            chain(&[translate(pos), rot_x(elapsed * 2.4), rot_y(elapsed * 3.2)]),
            color,
            0.98,
        );
        if effects::rings_visible(progress) {
            let k = effects::ring_scale(progress);
            let ring = chain(&[
                translate(from),
                rot_y(elapsed * 0.8),
                scale([k; 3]),
                rot_x(FRAC_PI_2),
            ]);
            b.basic_add(Shape::torus(0.72, 0.055, 6, 28), ring, color, 0.72);
            b.basic_add(Shape::torus(1.08, 0.035, 6, 28), ring, color, 0.42);
        }
    }
}

/// La luz del oráculo: violeta (gris en fallback), intensidad 3 más el pulso de cada decisión.
fn oracle_light(o: &OfficeState) -> Option<(Vec3, [f32; 3], f32)> {
    let st = o.graph.jev_status.as_ref().filter(|j| j.state != "off")?;
    let fallback = st.state == "fallback";
    Some((
        ORACLE_POSITION,
        lin(if fallback { 0x94a3b8 } else { 0xa78bfa }),
        effects::oracle_light(fallback, o.jev_fx.pulse),
    ))
}

/// El oráculo de Jev sobre la reina y los rayos de sus decisiones. Solo si Jev está activo.
fn oracle(b: &mut Builder, o: &OfficeState, coordinator_id: Option<&str>) {
    let Some(st) = o.graph.jev_status.as_ref().filter(|j| j.state != "off") else {
        return;
    };
    let fallback = st.state == "fallback";
    let hex = if fallback { 0x94a3b8 } else { 0xa78bfa };
    let c = lin(hex);
    let fx = &o.jev_fx;
    let at = translate(ORACLE_POSITION);
    wire(
        b,
        Shape::Ico { r: 950 },
        chain(&[
            at,
            rot_x(fx.shell.0),
            rot_y(fx.shell.1),
            scale([1.0 + fx.pulse * 0.22; 3]),
        ]),
        c,
        0.55,
    );
    b.solid(
        Shape::Octa { r: 450 },
        chain(&[at, rot_y(fx.core)]),
        0x1e1535,
        hex,
        effects::core_emissive(fallback, o.t, fx.pulse),
        0.4,
        0.25,
    );
    b.basic_add(
        Shape::torus(1.45, 0.025, 6, 48),
        chain(&[at, rot_x(FRAC_PI_2), rot_z(fx.ring)]),
        c,
        0.45,
    );

    for beam in &fx.beams {
        let progress = ((o.t - beam.started) / effects::JEV_BEAM_SECONDS).clamp(0.0, 1.0);
        let opacity = effects::beam_opacity(progress);
        let agent = beam.decision.agent_id.as_str();
        let end = if Some(agent) == coordinator_id {
            COORDINATOR_POINT
        } else {
            o.bee_pos(agent).unwrap_or(COORDINATOR_POINT)
        };
        // La curva se pinta como una cadena de tramos finos.
        const SEGMENTS: usize = 16;
        let mut prev = effects::bezier(ORACLE_POSITION, end, 0.0);
        for i in 1..=SEGMENTS {
            let next = effects::bezier(ORACLE_POSITION, end, i as f32 / SEGMENTS as f32);
            segment(b, prev, next, 0.04, c, opacity * 0.85);
            prev = next;
        }
        let spark = effects::bezier(ORACLE_POSITION, end, effects::spark_param(progress));
        b.basic_add(
            Shape::sphere(10, 8),
            chain(&[translate(spark), scale([0.16; 3])]),
            c,
            opacity,
        );
    }
}

/// Estelas de las abejas: tramos finos entre muestras sucesivas, que se afinan y se apagan con la edad.
fn trails(b: &mut Builder, o: &OfficeState, desks: &[model::Desk]) {
    for d in desks {
        let Some(trail) = o.trails.get(&d.id) else {
            continue;
        };
        let color = lin_rgb(d.color);
        for pair in trail.points.windows(2) {
            let ((from, t0), (to, _)) = (pair[0], pair[1]);
            let w = effects::trail_width(o.t - t0);
            if w > 0.0 {
                segment(b, from, to, 0.12 * w.max(0.05), color, 0.55 * w);
            }
        }
    }
}

/// Partículas ambientales y chispas de las ráfagas. Solo con movimiento y calidad alta.
fn ambient(b: &mut Builder, o: &OfficeState) {
    let color = lin(0x6aa9ff);
    for p in &o.field {
        let sp = effects::field_sprite(p, o.t, color);
        if sp.alpha > 0.001 {
            sprite(b, &sp);
        }
    }
    for burst in &o.bursts.active {
        for sp in effects::burst_sprites(burst, o.t) {
            if sp.alpha > 0.001 && sp.size > 0.05 {
                sprite(
                    b,
                    &Sprite {
                        color: [
                            lin_rgb_one(sp.color[0]),
                            lin_rgb_one(sp.color[1]),
                            lin_rgb_one(sp.color[2]),
                        ],
                        ..sp
                    },
                );
            }
        }
    }
}

/// Un canal sRGB → lineal (las chispas heredan el color sRGB del agente).
fn lin_rgb_one(c: f32) -> f32 {
    srgb_to_linear(c.min(1.0)) + (c - c.min(1.0)).max(0.0)
}

/// Un tramo recto fino entre dos puntos, aditivo.
fn segment(b: &mut Builder, from: Vec3, to: Vec3, radius: f32, color: [f32; 3], opacity: f32) {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 1e-4 {
        return;
    }
    let mid = [
        (from[0] + to[0]) / 2.0,
        (from[1] + to[1]) / 2.0,
        (from[2] + to[2]) / 2.0,
    ];
    b.basic_add(
        Shape::Cylinder {
            rt: 1000,
            rb: 1000,
            h: 1000,
            radial: 6,
            open: true,
        },
        chain(&[translate(mid), align_y(d), scale([radius, len, radius])]),
        color,
        opacity,
    );
}

/// Un rayo entre dos puntos: cilindro hexagonal estirado a lo largo de la línea.
fn beam(b: &mut Builder, from: Vec3, to: Vec3, color: [f32; 3]) {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len <= 0.5 {
        return;
    }
    let mid = [
        (from[0] + to[0]) / 2.0,
        (from[1] + to[1]) / 2.0,
        (from[2] + to[2]) / 2.0,
    ];
    b.push(
        Shape::Cylinder {
            rt: 1000,
            rb: 1000,
            h: 1000,
            radial: 6,
            open: true,
        },
        Material::Beam,
        chain(&[translate(mid), align_y(d), scale([0.055, len, 0.055])]),
        [color[0], color[1], color[2], 1.0],
        [0.52, 0.82, 0.0, 0.0],
        [0.0; 4],
    );
}

fn delegation_beams(
    b: &mut Builder,
    o: &OfficeState,
    coordinator_id: Option<&str>,
    desks: &[model::Desk],
) {
    let interactions: Vec<Interaction> = o.graph.interactions();
    for i in interactions {
        let from = if Some(i.source_id.as_str()) == coordinator_id {
            Some(COORDINATOR_POINT)
        } else {
            o.bee_pos(&i.source_id)
        };
        let to = o.bee_pos(&i.target_id);
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let color = desks
            .iter()
            .find(|d| d.id == i.target_id)
            .map_or(lin(AMBER), |d| lin_rgb(d.color));
        let _ = i.kind == InteractionKind::Reviews;
        beam(b, from, to, color);
    }
}

pub fn build(o: &OfficeState) -> SceneFrame {
    let (coordinator, desks) = o.desks();
    let mut b = Builder::default();

    // Suelo: el disco más grande, apenas por encima de y = 0.
    let c = lin(0x4f8ff7);
    b.push(
        Shape::Disc { r: 48_000, seg: 72 },
        Material::Floor,
        chain(&[translate([0.0, 0.01, 0.0]), rot_x(-FRAC_PI_2)]),
        [c[0], c[1], c[2], 1.0],
        [0.0; 4],
        [0.0; 4],
    );

    let processing = coordinator
        .as_ref()
        .is_some_and(|c| matches!(c.state, DeskState::Thinking | DeskState::ToolCall));
    let active = processing
        || desks.iter().any(|d| {
            matches!(
                d.state,
                DeskState::Thinking | DeskState::ToolCall | DeskState::Stuck
            )
        });
    let light_pos = queen(&mut b, o, processing, active);

    for d in &desks {
        bee(&mut b, o, &d.id, d);
    }
    delegation_beams(
        &mut b,
        o,
        coordinator.as_ref().map(|c| c.id.as_str()),
        &desks,
    );
    trails(&mut b, o, &desks);
    if o.calm && o.high_quality {
        ambient(&mut b, o);
    }
    transfers(
        &mut b,
        o,
        coordinator.as_ref().map(|c| c.id.as_str()),
        &desks,
    );
    oracle(&mut b, o, coordinator.as_ref().map(|c| c.id.as_str()));
    let _ = IDENTITY;
    SceneFrame {
        draws: b.out,
        queen_light: (light_pos, o.queen.light),
        oracle_light: oracle_light(o),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::graph::Node;
    use crate::settings::api::Agent;
    use serde_json::json;

    fn agent(id: &str, role: &str) -> Agent {
        Agent {
            id: id.into(),
            name: id.into(),
            role: role.into(),
            status: "idle".into(),
            enabled: true,
            ..Default::default()
        }
    }

    fn office(workers: usize) -> OfficeState {
        let mut o = OfficeState::default();
        o.agents.push(agent("boss", "coordinator"));
        for i in 0..workers {
            o.agents.push(agent(&format!("w{i}"), "worker"));
        }
        o.step(0.0);
        o
    }

    fn node(id: &str, status: &str) -> Node {
        Node {
            id: id.into(),
            kind: "agent".into(),
            name: id.into(),
            status: status.into(),
            data: json!({}),
        }
    }

    fn count(f: &SceneFrame, m: Material) -> usize {
        f.draws.iter().filter(|d| d.material == m).count()
    }

    fn pos(d: &Draw) -> Vec3 {
        [d.model[12], d.model[13], d.model[14]]
    }

    #[test]
    fn an_empty_office_is_the_floor_and_the_queen() {
        let f = build(&office(0));
        assert_eq!(
            f.draws[0].material,
            Material::Floor,
            "el suelo se pinta primero"
        );
        let solid = f
            .draws
            .iter()
            .filter(|d| d.material != Material::Sprite)
            .count();
        assert_eq!(
            solid,
            1 + 23,
            "suelo + 23 piezas de la reina y su pedestal (sin contar las partículas)"
        );
        assert_eq!(
            count(&f, Material::SolidAlpha),
            4,
            "cuatro alas translúcidas"
        );
        assert_eq!(count(&f, Material::Hologram), 0);
    }

    #[test]
    fn an_idle_bee_is_seven_pieces_and_a_thinking_one_adds_a_ring() {
        let mut o = office(2);
        o.graph.nodes.push(node("w1", "thinking"));
        o.step(0.016);
        let f = build(&o);
        assert_eq!(
            count(&f, Material::Hologram),
            2 * 5,
            "abdomen, cabeza, aguijón y dos alas por abeja"
        );
        let rings = f
            .draws
            .iter()
            .filter(|d| {
                d.material == Material::BasicAdd
                    && matches!(d.shape, Shape::Ring { inner: 420, .. })
            })
            .count();
        assert_eq!(rings, 1, "solo la abeja que piensa lleva aro");
    }

    #[test]
    fn a_stuck_bee_gets_the_red_ring() {
        let mut o = office(1);
        o.graph.nodes.push(node("w0", "error"));
        o.step(0.016);
        let f = build(&o);
        let ring = f
            .draws
            .iter()
            .find(|d| matches!(d.shape, Shape::Ring { inner: 520, .. }))
            .expect("aro de atasco");
        let red = lin(0xfb7185);
        assert!((ring.color[0] - red[0]).abs() < 1e-6 && ring.color[3] == 0.86);
    }

    #[test]
    fn colors_are_converted_to_linear_space() {
        let l = lin(0x4f8ff7);
        assert!(
            (l[0] - 0.0782).abs() < 1e-3 && (l[2] - 0.9302).abs() < 1e-3,
            "{l:?}"
        );
        assert_eq!(lin(0xffffff), [1.0, 1.0, 1.0]);
        assert_eq!(lin(0x000000), [0.0, 0.0, 0.0]);
        assert!((srgb_to_linear(0.5) - 0.2140).abs() < 1e-3);
    }

    #[test]
    fn the_floor_is_a_big_flat_disc_just_above_the_ground() {
        let f = build(&office(0));
        let floor = f.draws[0];
        assert_eq!(floor.shape, Shape::Disc { r: 48_000, seg: 72 });
        assert!((floor.model[13] - 0.01).abs() < 1e-6);
        let up = super::super::xform::apply(&floor.model, [0.0, 0.0, 1.0]);
        assert!(up[1] > 0.99, "queda tendido: su cara mira hacia arriba");
    }

    #[test]
    fn a_selected_bee_is_bigger_and_a_selected_queen_glows_more() {
        let mut o = office(1);
        let head = |f: &SceneFrame| {
            f.draws
                .iter()
                .find(|d| matches!(d.shape, Shape::Sphere { w: 12, h: 12 }))
                .copied()
                .unwrap()
        };
        let bee_pos = o.bee_pos("w0").unwrap();
        let d = |f: &SceneFrame| {
            let p = pos(&head(f));
            ((p[0] - bee_pos[0]).powi(2)
                + (p[1] - bee_pos[1]).powi(2)
                + (p[2] - bee_pos[2]).powi(2))
            .sqrt()
        };
        let normal = d(&build(&o));
        o.selected = Some("w0".into());
        let big = d(&build(&o));
        assert!((big / normal - 0.85 / 0.62).abs() < 0.01, "{normal} {big}");

        let pedestal = |f: &SceneFrame| {
            f.draws
                .iter()
                .find(|d| matches!(d.shape, Shape::Cylinder { rt: 2500, .. }))
                .unwrap()
                .p0[3]
        };
        o.selected = None;
        assert_eq!(pedestal(&build(&o)), 0.18);
        o.selected = Some("boss".into());
        assert_eq!(pedestal(&build(&o)), 0.5);
    }

    #[test]
    fn the_bee_head_leads_in_the_direction_of_flight() {
        let mut o = office(1);
        o.graph.nodes.push(node("w0", "thinking"));
        for _ in 0..600 {
            o.step(1.0 / 60.0);
        }
        let f = build(&o);
        let b = &o.bees["w0"];
        let head = f
            .draws
            .iter()
            .find(|d| matches!(d.shape, Shape::Sphere { w: 12, h: 12 }))
            .unwrap();
        let to_head = [
            pos(head)[0] - b.bee.pos[0],
            pos(head)[1] - b.bee.pos[1],
            pos(head)[2] - b.bee.pos[2],
        ];
        assert!(
            to_head[0] * b.dir[0] + to_head[1] * b.dir[1] + to_head[2] * b.dir[2] > 0.0,
            "la cabeza va por delante"
        );
    }

    #[test]
    fn wings_use_the_smoothed_opacity_and_flap_only_when_active_and_calm() {
        let mut o = office(1);
        let wing = |f: &SceneFrame| {
            f.draws
                .iter()
                .filter(|d| matches!(d.shape, Shape::Sphere { w: 10, h: 10 }))
                .map(|d| d.model)
                .collect::<Vec<_>>()
        };
        let rest = wing(&build(&o));
        o.t = 1.3;
        assert_eq!(
            rest,
            wing(&build(&o)),
            "una abeja ociosa mantiene las alas quietas"
        );
        o.graph.nodes.push(node("w0", "tool_call"));
        o.step(0.016);
        let a = wing(&build(&o));
        o.t += 0.1;
        assert_ne!(
            a,
            wing(&build(&o)),
            "una abeja que ejecuta una herramienta bate las alas"
        );
        o.calm = false;
        let c = wing(&build(&o));
        o.t += 0.1;
        assert_eq!(c.len(), 2);
        let alpha = build(&o)
            .draws
            .iter()
            .find(|d| matches!(d.shape, Shape::Sphere { w: 10, h: 10 }))
            .unwrap()
            .color[3];
        assert!(
            (alpha - 0.4).abs() < 0.05,
            "opacidad de ala suavizada: {alpha}"
        );
    }

    #[test]
    fn delegation_beams_connect_the_coordinator_or_a_bee_to_its_target() {
        let mut o = office(2);
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "e1", "source": "boss", "target": "w0", "edgeType": "delegates"}),
            None,
            0.0,
        );
        let f = build(&o);
        assert_eq!(count(&f, Material::Beam), 1);
        let beam = f
            .draws
            .iter()
            .find(|d| d.material == Material::Beam)
            .unwrap();
        let w0 = o.bee_pos("w0").unwrap();
        let mid = [
            (COORDINATOR_POINT[0] + w0[0]) / 2.0,
            (COORDINATOR_POINT[1] + w0[1]) / 2.0,
            (COORDINATOR_POINT[2] + w0[2]) / 2.0,
        ];
        assert!(
            pos(beam)
                .iter()
                .zip(mid.iter())
                .all(|(a, b)| (a - b).abs() < 1e-4),
            "centrado entre origen y destino"
        );
        // Estirado a lo largo de la línea: el eje Y local mide su longitud.
        let len = ((w0[0] - COORDINATOR_POINT[0]).powi(2)
            + (w0[1] - COORDINATOR_POINT[1]).powi(2)
            + (w0[2] - COORDINATOR_POINT[2]).powi(2))
        .sqrt();
        let ylen = (beam.model[4].powi(2) + beam.model[5].powi(2) + beam.model[6].powi(2)).sqrt();
        assert!((ylen - len).abs() < 1e-3);
        assert_eq!(
            beam.color[..3],
            lin_rgb(model::agent_color("w0")),
            "color del destino"
        );
    }

    #[test]
    fn a_bee_to_bee_review_beam_uses_both_live_positions() {
        let mut o = office(2);
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "e", "source": "w0", "target": "w1", "edgeType": "reviews"}),
            None,
            0.0,
        );
        assert_eq!(count(&build(&o), Material::Beam), 1);
    }

    #[test]
    fn beams_with_a_missing_end_or_a_degenerate_length_are_skipped() {
        let mut o = office(1);
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "a", "source": "boss", "target": "ghost", "edgeType": "delegates"}),
            None,
            0.0,
        );
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "b", "source": "ghost", "target": "w0", "edgeType": "delegates"}),
            None,
            0.0,
        );
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "c", "source": "w0", "target": "w0", "edgeType": "reviews"}),
            None,
            0.0,
        );
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "d", "source": "w0", "target": "boss", "edgeType": "depends"}),
            None,
            0.0,
        );
        assert_eq!(count(&build(&o), Material::Beam), 0);
    }

    #[test]
    fn the_queen_light_sits_above_her_and_follows_the_state() {
        let mut o = office(1);
        let (p, i) = build(&o).queen_light;
        assert!(p[1] > 3.9 && p[1] < 4.3, "{p:?}");
        assert_eq!(i, 6.0);
        o.graph.nodes.push(node("w0", "thinking"));
        for _ in 0..600 {
            o.step(1.0 / 60.0);
        }
        assert!(build(&o).queen_light.1 > 11.0);
    }

    #[test]
    fn every_shape_in_a_busy_scene_builds_into_a_valid_mesh() {
        let mut o = office(4);
        o.graph.nodes.push(node("w0", "thinking"));
        o.graph.nodes.push(node("w1", "error"));
        o.graph.apply(
            "canvas:edge_add",
            &json!({"id": "e", "source": "boss", "target": "w2", "edgeType": "delegates"}),
            None,
            0.0,
        );
        o.step(0.016);
        let mut shapes: Vec<Shape> = build(&o).draws.iter().map(|d| d.shape).collect();
        shapes.sort_by_key(|s| format!("{s:?}"));
        shapes.dedup();
        assert!(shapes.len() >= 12, "{} formas distintas", shapes.len());
        for s in shapes {
            let m = s.build();
            assert!(
                !m.idx.is_empty() && m.idx.iter().all(|i| (*i as usize) < m.verts.len()),
                "{s:?}"
            );
        }
    }

    #[test]
    fn the_scene_is_deterministic() {
        let (a, b) = (office(3), office(3));
        assert_eq!(build(&a).draws, build(&b).draws);
    }

    // ── paquetes, oráculo y hologramas ──

    fn count_shape(f: &SceneFrame, m: Material, shape: Shape) -> usize {
        f.draws
            .iter()
            .filter(|d| d.material == m && d.shape == shape)
            .count()
    }

    fn with_flight(phase: &str, target: Option<&str>) -> OfficeState {
        let mut o = office(2);
        o.step(0.016); // prepara el director y los trackers con la historia vacía
        o.graph.apply("canvas:work_event", &json!({"eventId": "f1", "phase": phase, "actorId": "boss", "targetId": target, "taskName": "T"}), None, 0.0);
        o.step(0.016);
        o
    }

    #[test]
    fn a_new_work_event_launches_a_packet_with_two_pulse_rings() {
        let o = with_flight("delegated", Some("w0"));
        let f = build(&o);
        assert_eq!(
            count_shape(&f, Material::BasicAdd, Shape::Octa { r: 300 }),
            1,
            "un paquete"
        );
        let rings = f
            .draws
            .iter()
            .filter(|d| {
                matches!(
                    d.shape,
                    Shape::Torus { r: 720, .. } | Shape::Torus { r: 1080, .. }
                )
            })
            .count();
        assert_eq!(rings, 2, "y dos anillos de pulso en el origen");
        let packet = f
            .draws
            .iter()
            .find(|d| matches!(d.shape, Shape::Octa { r: 300 }))
            .unwrap();
        assert_eq!(packet.color[3], 0.98);
    }

    #[test]
    fn the_packet_starts_at_the_coordinator_and_ends_on_the_target_bee() {
        let mut o = with_flight("delegated", Some("w0"));
        let start = pos(f_packet(&build(&o)));
        assert!(
            (start[1] - COORDINATOR_POINT[1]).abs() < 0.2 && start[0].abs() < 0.2,
            "sale del coordinador: {start:?}"
        );
        for _ in 0..140 {
            o.step(1.0 / 60.0); // 2,3 s: ya ha aterrizado y desaparecido
        }
        assert_eq!(
            count_shape(&build(&o), Material::BasicAdd, Shape::Octa { r: 300 }),
            0,
            "tras 2,25 s no queda nada"
        );
    }

    fn f_packet(f: &SceneFrame) -> &Draw {
        f.draws
            .iter()
            .find(|d| matches!(d.shape, Shape::Octa { r: 300 }))
            .expect("hay un paquete")
    }

    #[test]
    fn packets_take_the_color_of_their_phase_and_rings_fade_after_the_first_half() {
        let alert = with_flight("failed", None);
        let red = lin_rgb(effects::packet_color("failed", None, None));
        assert_eq!(f_packet(&build(&alert)).color[..3], red);
        let mut done = with_flight("completed", Some("w0"));
        assert_eq!(
            f_packet(&build(&done)).color[..3],
            lin_rgb(effects::packet_color("completed", None, None))
        );
        for _ in 0..90 {
            done.step(1.0 / 60.0); // 1,5 s: pasado el 46 % de 2,25 s (1,035 s)
        }
        let f = build(&done);
        assert_eq!(
            f.draws
                .iter()
                .filter(|d| matches!(d.shape, Shape::Torus { r: 720, .. }))
                .count(),
            0,
            "los anillos se apagan"
        );
        assert_eq!(
            count_shape(&f, Material::BasicAdd, Shape::Octa { r: 300 }),
            1,
            "pero el paquete sigue volando"
        );
    }

    #[test]
    fn with_motion_off_nothing_flies() {
        let mut o = office(2);
        o.calm = false;
        o.step(0.016);
        o.graph.apply(
            "canvas:work_event",
            &json!({"eventId": "f1", "phase": "delegated", "actorId": "boss", "targetId": "w0"}),
            None,
            0.0,
        );
        o.step(0.016);
        assert_eq!(
            count_shape(&build(&o), Material::BasicAdd, Shape::Octa { r: 300 }),
            0
        );
    }

    #[test]
    fn an_event_with_an_unknown_actor_draws_no_packet() {
        let mut o = office(1);
        o.step(0.016);
        o.graph.apply(
            "canvas:work_event",
            &json!({"eventId": "x", "phase": "delegated", "actorId": "ghost", "targetId": "w0"}),
            None,
            0.0,
        );
        o.step(0.016);
        assert_eq!(
            count_shape(&build(&o), Material::BasicAdd, Shape::Octa { r: 300 }),
            0,
            "sin origen resoluble se omite"
        );
    }

    #[test]
    fn the_oracle_only_exists_when_jev_is_active() {
        let mut o = office(1);
        assert_eq!(
            count(&build(&o), Material::WireAdd),
            0,
            "sin Jev no hay oráculo"
        );
        o.graph
            .apply("canvas:jev_status", &json!({"state": "off"}), None, 0.0);
        assert_eq!(count(&build(&o), Material::WireAdd), 0, "apagado tampoco");
        o.graph.apply(
            "canvas:jev_status",
            &json!({"state": "ready", "backend": "jev"}),
            None,
            0.0,
        );
        let f = build(&o);
        assert_eq!(
            count_shape(&f, Material::WireAdd, Shape::Ico { r: 950 }),
            1,
            "cubierta de alambre"
        );
        assert_eq!(
            f.draws
                .iter()
                .filter(
                    |d| d.material == Material::Solid && matches!(d.shape, Shape::Octa { r: 450 })
                )
                .count(),
            1,
            "núcleo"
        );
        assert_eq!(
            f.draws
                .iter()
                .filter(|d| matches!(d.shape, Shape::Torus { r: 1450, .. }))
                .count(),
            1,
            "aro"
        );
        let shell = f
            .draws
            .iter()
            .find(|d| matches!(d.shape, Shape::Ico { r: 950 }))
            .unwrap();
        assert!(
            (pos(shell)[1] - 7.6).abs() < 1e-5,
            "a 7,6 de altura sobre la reina"
        );
    }

    #[test]
    fn the_oracle_is_grey_in_fallback_and_violet_otherwise() {
        let mut o = office(0);
        o.graph.apply(
            "canvas:jev_status",
            &json!({"state": "fallback"}),
            None,
            0.0,
        );
        let shell = |o: &OfficeState| {
            build(o)
                .draws
                .iter()
                .find(|d| matches!(d.shape, Shape::Ico { r: 950 }))
                .unwrap()
                .color
        };
        assert_eq!(shell(&o)[..3], lin(0x94a3b8));
        o.graph
            .apply("canvas:jev_status", &json!({"state": "ready"}), None, 0.0);
        assert_eq!(shell(&o)[..3], lin(0xa78bfa));
    }

    #[test]
    fn the_oracle_lights_the_scene_only_with_jev_and_a_decision_makes_it_flare() {
        let mut o = office(1);
        assert!(build(&o).oracle_light.is_none(), "sin Jev no hay luz");
        o.graph
            .apply("canvas:jev_status", &json!({"state": "off"}), None, 0.0);
        assert!(build(&o).oracle_light.is_none(), "apagado tampoco");
        o.graph.apply(
            "canvas:jev_status",
            &json!({"state": "ready", "backend": "jev"}),
            None,
            0.0,
        );
        let (p, c, i) = build(&o).oracle_light.expect("hay luz");
        assert!((p[1] - 7.6).abs() < 1e-5, "en el oráculo");
        assert_eq!(c, lin(0xa78bfa), "violeta");
        assert!(
            (i - 3.0).abs() < 1e-5,
            "intensidad 3 en reposo, como la web"
        );
        o.graph.apply(
            "canvas:jev_status",
            &json!({"state": "fallback"}),
            None,
            0.0,
        );
        let (_, c, i) = build(&o).oracle_light.unwrap();
        assert_eq!((c, i), (lin(0x94a3b8), 1.2), "gris y tenue en fallback");
        o.jev_fx.pulse = 1.0;
        assert!(
            (build(&o).oracle_light.unwrap().2 - (1.2 + 9.0)).abs() < 1e-4,
            "una decisión la hace destellar"
        );
    }

    #[test]
    fn a_pulse_swells_the_shell() {
        let mut o = office(0);
        o.graph
            .apply("canvas:jev_status", &json!({"state": "ready"}), None, 0.0);
        o.step(0.016);
        let size = |o: &OfficeState| {
            let d = *build(o)
                .draws
                .iter()
                .find(|d| matches!(d.shape, Shape::Ico { r: 950 }))
                .unwrap();
            (d.model[0].powi(2) + d.model[1].powi(2) + d.model[2].powi(2)).sqrt()
        };
        let calm = size(&o);
        o.graph.apply(
            "canvas:jev_decision",
            &json!({"eventId": "d", "agentId": "boss", "kind": "context"}),
            None,
            0.0,
        );
        o.step(0.016);
        assert!(
            size(&o) > calm * 1.15,
            "la cubierta se hincha un 22 % con el pulso: {calm} → {}",
            size(&o)
        );
    }

    #[test]
    fn a_decision_fires_a_curved_beam_of_thin_segments_and_a_spark() {
        let mut o = office(1);
        o.graph
            .apply("canvas:jev_status", &json!({"state": "ready"}), None, 0.0);
        o.step(0.016);
        o.graph.apply(
            "canvas:jev_decision",
            &json!({"eventId": "d", "agentId": "w0", "kind": "context"}),
            None,
            0.0,
        );
        o.step(0.3);
        let f = build(&o);
        let thin = f
            .draws
            .iter()
            .filter(|d| {
                d.material == Material::BasicAdd
                    && matches!(d.shape, Shape::Cylinder { open: true, .. })
            })
            .count();
        assert_eq!(thin, 16, "la curva se pinta con 16 tramos");
        assert_eq!(
            count_shape(&f, Material::BasicAdd, Shape::Sphere { w: 10, h: 8 }),
            1,
            "y una chispa"
        );
        for _ in 0..120 {
            o.step(1.0 / 60.0);
        }
        let after = build(&o)
            .draws
            .iter()
            .filter(|d| {
                matches!(d.shape, Shape::Cylinder { open: true, .. })
                    && d.material == Material::BasicAdd
            })
            .count();
        assert_eq!(after, 0, "tras 1,7 s el rayo desaparece");
    }

    #[test]
    fn only_a_bee_running_a_tool_wears_a_hologram_shaped_by_the_tool() {
        let mut o = office(2);
        o.graph.nodes.push(node("w0", "tool_call"));
        o.graph.nodes[0].data = json!({"currentTool": "web_search"});
        o.step(0.016);
        let f = build(&o);
        assert_eq!(
            count(&f, Material::WireAdd),
            2,
            "esfera y aro para una herramienta de navegador"
        );
        assert_eq!(count_shape(&f, Material::WireAdd, Shape::sphere(10, 8)), 1);

        for (tool, shapes) in [
            (
                "fs_read",
                vec![
                    Shape::Cuboid {
                        x: 750,
                        y: 550,
                        z: 120,
                    },
                    Shape::Cuboid {
                        x: 60,
                        y: 320,
                        z: 50,
                    },
                    Shape::Cuboid {
                        x: 60,
                        y: 320,
                        z: 50,
                    },
                ],
            ),
            (
                "search_knowledge",
                vec![Shape::Octa { r: 460 }, Shape::torus(0.58, 0.025, 6, 6)],
            ),
            (
                "telegram_send",
                vec![
                    Shape::Torus {
                        r: 250,
                        tube: 25,
                        radial: 5,
                        tubular: 20,
                        arc: milli(PI),
                    },
                    Shape::Torus {
                        r: 420,
                        tube: 25,
                        radial: 5,
                        tubular: 20,
                        arc: milli(PI),
                    },
                    Shape::Torus {
                        r: 590,
                        tube: 25,
                        radial: 5,
                        tubular: 20,
                        arc: milli(PI),
                    },
                ],
            ),
            (
                "calc",
                vec![Shape::Ico { r: 400 }, Shape::torus(0.56, 0.025, 5, 24)],
            ),
        ] {
            let mut o = office(1);
            o.graph.nodes.push(node("w0", "tool_call"));
            o.graph.nodes[0].data = json!({"currentTool": tool});
            o.step(0.016);
            let f = build(&o);
            let mut got: Vec<String> = f
                .draws
                .iter()
                .filter(|d| d.material == Material::WireAdd)
                .map(|d| format!("{:?}", d.shape))
                .collect();
            let mut want: Vec<String> = shapes.iter().map(|s| format!("{s:?}")).collect();
            got.sort();
            want.sort();
            assert_eq!(got, want, "{tool}");
        }
        let idle = build(&office(2));
        assert_eq!(
            count(&idle, Material::WireAdd),
            0,
            "una abeja ociosa no lleva holograma"
        );
    }

    #[test]
    fn the_hologram_floats_above_its_bee_in_the_bee_color() {
        let mut o = office(1);
        o.graph.nodes.push(node("w0", "tool_call"));
        o.graph.nodes[0].data = json!({"currentTool": "web_search"});
        o.step(0.016);
        let f = build(&o);
        let holo = f
            .draws
            .iter()
            .find(|d| {
                matches!(d.shape, Shape::Sphere { w: 10, h: 8 }) && d.material == Material::WireAdd
            })
            .unwrap();
        let bee = o.bee_pos("w0").unwrap();
        assert!(pos(holo)[1] > bee[1] + 0.4, "flota por encima de la abeja");
        assert_eq!(holo.color[..3], lin_rgb(model::agent_color("w0")));
        assert_eq!(holo.color[3], 0.72);
    }

    // ── partículas, ráfagas y estelas ──

    fn sprites(f: &SceneFrame) -> Vec<&Draw> {
        f.draws
            .iter()
            .filter(|d| d.material == Material::Sprite)
            .collect()
    }

    #[test]
    fn ambient_particles_need_motion_and_high_quality() {
        let mut o = office(0);
        let on = sprites(&build(&o)).len();
        assert!(
            on > 650 && on <= effects::FIELD_COUNT,
            "casi todas las 780 son visibles a la vez: {on}"
        );
        o.calm = false;
        assert_eq!(
            sprites(&build(&o)).len(),
            0,
            "sin movimiento no hay partículas"
        );
        o.calm = true;
        o.high_quality = false;
        assert_eq!(sprites(&build(&o)).len(), 0, "en calidad baja tampoco");
    }

    #[test]
    fn sprites_carry_their_pixel_size_falloff_and_a_faint_alpha() {
        let o = office(0);
        let f = build(&o);
        let d = sprites(&f)[0];
        assert_eq!(d.shape, Shape::Quad);
        assert!(
            d.p0[0] >= 1.4 && d.p0[0] <= 3.8 && d.p0[1] == 28.0,
            "{:?}",
            d.p0
        );
        assert!(
            d.color[3] > 0.0 && d.color[3] <= effects::FIELD_OPACITY + 1e-6,
            "tenue: {}",
            d.color[3]
        );
    }

    #[test]
    fn a_finished_tool_call_throws_sparks_that_vanish() {
        let mut o = office(1);
        o.graph.nodes.push(node("w0", "tool_call"));
        for _ in 0..30 {
            o.step(1.0 / 60.0);
        }
        let before = sprites(&build(&o))
            .iter()
            .filter(|d| d.p0[1] == 30.0)
            .count();
        assert_eq!(before, 0, "mientras usa la herramienta no hay chispas");
        o.graph.nodes[0].status = "idle".into();
        o.step(1.0 / 60.0);
        o.step(0.3);
        let sparks = sprites(&build(&o))
            .iter()
            .filter(|d| d.p0[1] == 30.0)
            .count();
        assert!(
            sparks > 50 && sparks <= effects::BURST_PARTICLES,
            "salen unas 64 chispas: {sparks}"
        );
        for _ in 0..120 {
            o.step(1.0 / 60.0);
        }
        assert_eq!(
            sprites(&build(&o))
                .iter()
                .filter(|d| d.p0[1] == 30.0)
                .count(),
            0,
            "y se apagan"
        );
    }

    #[test]
    fn no_sparks_with_motion_off_or_low_quality() {
        for (calm, high) in [(false, true), (true, false)] {
            let mut o = office(1);
            o.calm = calm;
            o.high_quality = high;
            o.graph.nodes.push(node("w0", "tool_call"));
            for _ in 0..30 {
                o.step(1.0 / 60.0);
            }
            o.graph.nodes[0].status = "idle".into();
            o.step(1.0 / 60.0);
            o.step(0.3);
            assert!(o.bees.contains_key("w0"));
            assert_eq!(sprites(&build(&o)).len(), 0, "calm={calm} high={high}");
        }
    }

    fn trail_segments(f: &SceneFrame) -> Vec<&Draw> {
        f.draws
            .iter()
            .filter(|d| {
                d.material == Material::BasicAdd
                    && matches!(d.shape, Shape::Cylinder { open: true, .. })
            })
            .collect()
    }

    #[test]
    fn a_working_bee_trails_behind_it_and_an_idle_one_does_not() {
        let mut o = office(2);
        o.graph.nodes.push(node("w0", "thinking"));
        for _ in 0..180 {
            o.step(1.0 / 60.0); // 3 s
        }
        let f = build(&o);
        let n = trail_segments(&f).len();
        assert!(
            n > 40 && n < 75,
            "unos 30 puntos por segundo durante 2,2 s: {n}"
        );
        let mut idle = office(2);
        for _ in 0..180 {
            idle.step(1.0 / 60.0);
        }
        assert_eq!(
            trail_segments(&build(&idle)).len(),
            0,
            "sin trabajo no hay estela"
        );
    }

    #[test]
    fn the_trail_fades_and_thins_toward_its_tail() {
        let mut o = office(1);
        o.graph.nodes.push(node("w0", "thinking"));
        for _ in 0..180 {
            o.step(1.0 / 60.0);
        }
        let f = build(&o);
        let segs = trail_segments(&f);
        let (tail, head) = (segs.first().unwrap(), segs.last().unwrap());
        assert!(
            head.color[3] > tail.color[3] + 0.2,
            "más opaca en la cabeza: {} frente a {}",
            head.color[3],
            tail.color[3]
        );
        let width =
            |d: &Draw| (d.model[0].powi(2) + d.model[1].powi(2) + d.model[2].powi(2)).sqrt();
        assert!(width(head) > width(tail) * 1.5, "y más gruesa");
        assert!(head.color[3] <= 0.55 + 1e-6);
    }

    #[test]
    fn a_bee_that_stops_working_loses_its_trail() {
        let mut o = office(1);
        o.graph.nodes.push(node("w0", "thinking"));
        for _ in 0..120 {
            o.step(1.0 / 60.0);
        }
        assert!(!trail_segments(&build(&o)).is_empty());
        o.graph.nodes[0].status = "idle".into();
        o.step(1.0 / 60.0);
        assert!(trail_segments(&build(&o)).is_empty());
    }
}
