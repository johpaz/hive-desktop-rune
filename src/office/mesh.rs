//! Mallas procedurales (posición y normal por vértice, índices de triángulo). Sin dependencias de GPU.

use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub verts: Vec<Vertex>,
    pub idx: Vec<u32>,
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-9 {
        [0.0, 1.0, 0.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Mesh {
    fn push(&mut self, pos: [f32; 3], nrm: [f32; 3]) -> u32 {
        self.verts.push(Vertex { pos, nrm });
        (self.verts.len() - 1) as u32
    }

    /// Rejilla paramétrica (u, v): `f` da posición y normal; une las celdas con dos triángulos.
    fn grid(nu: usize, nv: usize, f: impl Fn(f32, f32) -> ([f32; 3], [f32; 3])) -> Mesh {
        let mut m = Mesh::default();
        for j in 0..=nv {
            for i in 0..=nu {
                let (p, n) = f(i as f32 / nu as f32, j as f32 / nv as f32);
                m.push(p, n);
            }
        }
        let row = (nu + 1) as u32;
        for j in 0..nv as u32 {
            for i in 0..nu as u32 {
                let a = j * row + i;
                let (b, c, d) = (a + 1, a + row, a + row + 1);
                m.idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        m
    }

    /// Esfera UV de radio `r`.
    pub fn sphere(r: f32, w: usize, h: usize) -> Mesh {
        Mesh::grid(w, h, |u, v| {
            let (th, ph) = (u * TAU, v * PI);
            let n = [ph.sin() * th.cos(), ph.cos(), ph.sin() * th.sin()];
            ([n[0] * r, n[1] * r, n[2] * r], n)
        })
    }

    /// Toro en el plano XY: radio mayor `r`, grosor `tube`; `arc` en radianes del círculo mayor (TAU = completo).
    pub fn torus(r: f32, tube: f32, radial: usize, tubular: usize, arc: f32) -> Mesh {
        Mesh::grid(tubular, radial, |u, v| {
            let (a, b) = (u * arc, v * TAU);
            let center = [r * a.cos(), r * a.sin(), 0.0];
            let n = [b.cos() * a.cos(), b.cos() * a.sin(), b.sin()];
            (
                [
                    center[0] + tube * n[0],
                    center[1] + tube * n[1],
                    center[2] + tube * n[2],
                ],
                n,
            )
        })
    }

    /// Cilindro (o tronco de cono) a lo largo de Y, centrado en el origen, con tapas.
    pub fn cylinder(
        r_top: f32,
        r_bottom: f32,
        height: f32,
        radial: usize,
        open_ended: bool,
    ) -> Mesh {
        let slope = (r_bottom - r_top) / height;
        let mut m = Mesh::grid(radial, 1, |u, v| {
            let a = u * TAU;
            let r = r_top + (r_bottom - r_top) * v;
            let n = norm([a.cos(), slope, a.sin()]);
            ([r * a.cos(), height / 2.0 - v * height, r * a.sin()], n)
        });
        if !open_ended {
            for (y, r, ny) in [
                (height / 2.0, r_top, 1.0f32),
                (-height / 2.0, r_bottom, -1.0),
            ] {
                if r <= 0.0 {
                    continue;
                }
                let c = m.push([0.0, y, 0.0], [0.0, ny, 0.0]);
                let first = m.verts.len() as u32;
                for i in 0..=radial {
                    let a = i as f32 / radial as f32 * TAU;
                    m.push([r * a.cos(), y, r * a.sin()], [0.0, ny, 0.0]);
                }
                for i in 0..radial as u32 {
                    if ny > 0.0 {
                        m.idx.extend_from_slice(&[c, first + i + 1, first + i]);
                    } else {
                        m.idx.extend_from_slice(&[c, first + i, first + i + 1]);
                    }
                }
            }
        }
        m
    }

    /// Disco plano en el plano XY (cara hacia +Z) con `segments` triángulos en abanico.
    pub fn disc(r: f32, segments: usize) -> Mesh {
        let mut m = Mesh::default();
        let c = m.push([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        for i in 0..=segments {
            let a = i as f32 / segments as f32 * TAU;
            m.push([r * a.cos(), r * a.sin(), 0.0], [0.0, 0.0, 1.0]);
        }
        for i in 0..segments as u32 {
            m.idx.extend_from_slice(&[c, c + 1 + i, c + 2 + i]);
        }
        m
    }

    /// Cuadrado unidad centrado en el origen, en el plano XY (cara hacia +Z), para sprites.
    pub fn quad() -> Mesh {
        let mut m = Mesh::default();
        for (x, y) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            m.push([x, y, 0.0], [0.0, 0.0, 1.0]);
        }
        m.idx.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        m
    }

    /// Cono con la punta hacia +Y y la base en -Y/2.
    pub fn cone(r: f32, height: f32, radial: usize) -> Mesh {
        Mesh::cylinder(0.0, r, height, radial, false)
    }

    /// Anillo plano en el plano XY (cara hacia +Z).
    pub fn ring(inner: f32, outer: f32, segments: usize) -> Mesh {
        Mesh::grid(segments, 1, |u, v| {
            let (a, r) = (u * TAU, inner + (outer - inner) * v);
            ([r * a.cos(), r * a.sin(), 0.0], [0.0, 0.0, 1.0])
        })
    }

    /// Caja centrada de tamaño `s`.
    pub fn cuboid(s: [f32; 3]) -> Mesh {
        let mut m = Mesh::default();
        let h = [s[0] / 2.0, s[1] / 2.0, s[2] / 2.0];
        for (axis, sign) in [
            (0, 1.0f32),
            (0, -1.0),
            (1, 1.0),
            (1, -1.0),
            (2, 1.0),
            (2, -1.0),
        ] {
            let n = {
                let mut n = [0.0; 3];
                n[axis] = sign;
                n
            };
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut corner = |su: f32, sv: f32| {
                let mut p = [0.0; 3];
                p[axis] = sign * h[axis];
                p[u] = su * h[u];
                p[v] = sv * h[v];
                m.push(p, n)
            };
            let (a, b, c, d) = (
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            );
            // Orientación contraria a las agujas del reloj mirando desde fuera.
            if sign > 0.0 {
                m.idx.extend_from_slice(&[a, b, c, a, c, d]);
            } else {
                m.idx.extend_from_slice(&[a, c, b, a, d, c]);
            }
        }
        m
    }

    /// Cápsula a lo largo de Y: cilindro de largo `length` y dos semiesferas de radio `r`.
    pub fn capsule(r: f32, length: f32, segs: usize) -> Mesh {
        let rings = segs.max(2);
        Mesh::grid(segs.max(3), rings * 2, |u, v| {
            let a = u * TAU;
            let t = v * 2.0; // 0..1 hemisferio superior, 1..2 inferior
            let (ph, y_off) = if t < 1.0 {
                (t * PI / 2.0, length / 2.0)
            } else {
                (PI / 2.0 + (t - 1.0) * PI / 2.0, -length / 2.0)
            };
            let n = [ph.sin() * a.cos(), ph.cos(), ph.sin() * a.sin()];
            ([n[0] * r, n[1] * r + y_off, n[2] * r], n)
        })
    }

    fn flat_faces(points: &[[f32; 3]], faces: &[[usize; 3]]) -> Mesh {
        let mut m = Mesh::default();
        for f in faces {
            let (a, b, c) = (points[f[0]], points[f[1]], points[f[2]]);
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let mut n = norm(cross(e1, e2));
            // La normal debe apuntar hacia fuera: contra el centroide del triángulo.
            let ctr = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let flip = n[0] * ctr[0] + n[1] * ctr[1] + n[2] * ctr[2] < 0.0;
            if flip {
                n = [-n[0], -n[1], -n[2]];
            }
            let ids = [m.push(a, n), m.push(b, n), m.push(c, n)];
            if flip {
                m.idx.extend_from_slice(&[ids[0], ids[2], ids[1]]);
            } else {
                m.idx.extend_from_slice(&ids);
            }
        }
        m
    }

    pub fn octahedron(r: f32) -> Mesh {
        let p = [
            [r, 0.0, 0.0],
            [-r, 0.0, 0.0],
            [0.0, r, 0.0],
            [0.0, -r, 0.0],
            [0.0, 0.0, r],
            [0.0, 0.0, -r],
        ];
        let f = [
            [0, 2, 4],
            [4, 2, 1],
            [1, 2, 5],
            [5, 2, 0],
            [0, 4, 3],
            [4, 1, 3],
            [1, 5, 3],
            [5, 0, 3],
        ];
        Mesh::flat_faces(&p, &f)
    }

    pub fn icosahedron(r: f32) -> Mesh {
        let t = (1.0 + 5.0f32.sqrt()) / 2.0;
        let raw = [
            [-1.0, t, 0.0],
            [1.0, t, 0.0],
            [-1.0, -t, 0.0],
            [1.0, -t, 0.0],
            [0.0, -1.0, t],
            [0.0, 1.0, t],
            [0.0, -1.0, -t],
            [0.0, 1.0, -t],
            [t, 0.0, -1.0],
            [t, 0.0, 1.0],
            [-t, 0.0, -1.0],
            [-t, 0.0, 1.0],
        ];
        let p: Vec<[f32; 3]> = raw
            .iter()
            .map(|v| {
                let n = norm(*v);
                [n[0] * r, n[1] * r, n[2] * r]
            })
            .collect();
        let f = [
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        Mesh::flat_faces(&p, &f)
    }

    /// Pares de índices de arista únicos, para dibujar en alambre.
    pub fn wire_indices(&self) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        // Se unifican los vértices por posición: las mallas planas repiten vértices en cada cara.
        let key = |v: &Vertex| (v.pos[0].to_bits(), v.pos[1].to_bits(), v.pos[2].to_bits());
        for t in self.idx.chunks(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let (ka, kb) = (key(&self.verts[a as usize]), key(&self.verts[b as usize]));
                let e = if ka <= kb { (ka, kb) } else { (kb, ka) };
                if ka != kb && seen.insert(e) {
                    out.extend_from_slice(&[a, b]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(m: &Mesh, name: &str) {
        assert!(!m.verts.is_empty() && !m.idx.is_empty(), "{name} vacía");
        assert_eq!(m.idx.len() % 3, 0, "{name}: triángulos completos");
        assert!(
            m.idx.iter().all(|i| (*i as usize) < m.verts.len()),
            "{name}: índices válidos"
        );
        for v in &m.verts {
            let l = (v.nrm[0].powi(2) + v.nrm[1].powi(2) + v.nrm[2].powi(2)).sqrt();
            assert!((l - 1.0).abs() < 1e-3, "{name}: normal unitaria ({l})");
            assert!(
                v.pos.iter().all(|c| c.is_finite()),
                "{name}: posiciones finitas"
            );
        }
    }

    #[test]
    fn every_primitive_is_well_formed() {
        check(&Mesh::sphere(0.5, 16, 12), "esfera");
        check(&Mesh::torus(0.5, 0.05, 8, 24, TAU), "toro");
        check(&Mesh::torus(0.5, 0.05, 5, 20, PI), "semitoro");
        check(&Mesh::cylinder(2.5, 2.8, 0.45, 6, false), "pedestal");
        check(&Mesh::cylinder(1.0, 1.0, 1.0, 6, true), "haz");
        check(&Mesh::cone(0.14, 0.5, 8), "cono");
        check(&Mesh::ring(1.85, 2.22, 6), "anillo");
        check(&Mesh::disc(48.0, 72), "disco");
        check(&Mesh::quad(), "cuadrado");
        check(&Mesh::cuboid([0.75, 0.55, 0.12]), "caja");
        check(&Mesh::capsule(0.035, 0.42, 6), "cápsula");
        check(&Mesh::octahedron(0.46), "octaedro");
        check(&Mesh::icosahedron(0.95), "icosaedro");
    }

    #[test]
    fn the_sphere_has_the_requested_radius_and_outward_normals() {
        let m = Mesh::sphere(2.0, 20, 14);
        for v in &m.verts {
            let r = (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt();
            assert!((r - 2.0).abs() < 1e-3);
            let d = v.pos[0] * v.nrm[0] + v.pos[1] * v.nrm[1] + v.pos[2] * v.nrm[2];
            assert!(d > 1.99, "la normal apunta hacia fuera");
        }
    }

    #[test]
    fn the_hexagonal_cylinder_has_six_sides_and_caps() {
        let m = Mesh::cylinder(2.5, 2.8, 0.45, 6, false);
        let ys: Vec<f32> = m.verts.iter().map(|v| v.pos[1]).collect();
        assert!(
            ys.iter().cloned().fold(f32::MIN, f32::max) <= 0.225 + 1e-4
                && ys.iter().cloned().fold(f32::MAX, f32::min) >= -0.225 - 1e-4
        );
        // 6 lados × 2 triángulos + 2 tapas × 6 triángulos
        assert_eq!(m.idx.len() / 3, 6 * 2 + 2 * 6);
        let open = Mesh::cylinder(1.0, 1.0, 1.0, 6, true);
        assert_eq!(open.idx.len() / 3, 12, "un haz abierto no tiene tapas");
    }

    #[test]
    fn a_cone_comes_to_a_point_and_has_no_top_cap() {
        let m = Mesh::cone(0.5, 1.0, 8);
        assert!(
            m.verts
                .iter()
                .filter(|v| (v.pos[1] - 0.5).abs() < 1e-5)
                .count()
                > 0
        );
        assert!(
            m.verts
                .iter()
                .filter(|v| (v.pos[1] - 0.5).abs() < 1e-5)
                .all(|v| v.pos[0].abs() < 1e-5 && v.pos[2].abs() < 1e-5),
            "la punta está en el eje"
        );
    }

    #[test]
    fn a_torus_stays_within_its_radius_plus_tube() {
        let m = Mesh::torus(0.5, 0.05, 8, 24, TAU);
        for v in &m.verts {
            let d = (v.pos[0].powi(2) + v.pos[1].powi(2)).sqrt();
            assert!((0.45 - 1e-4..=0.55 + 1e-4).contains(&d) && v.pos[2].abs() <= 0.05 + 1e-4);
        }
    }

    #[test]
    fn a_half_torus_only_covers_half_the_circle() {
        let m = Mesh::torus(1.0, 0.02, 4, 20, PI);
        assert!(
            m.verts.iter().all(|v| v.pos[1] >= -0.03),
            "solo el semiplano y ≥ 0"
        );
    }

    #[test]
    fn a_cuboid_has_the_requested_extent_and_faces_point_outward() {
        let m = Mesh::cuboid([2.0, 4.0, 6.0]);
        assert_eq!(m.verts.len(), 24);
        assert_eq!(m.idx.len() / 3, 12);
        let max = |k: usize| m.verts.iter().map(|v| v.pos[k]).fold(f32::MIN, f32::max);
        assert!(
            (max(0) - 1.0).abs() < 1e-6
                && (max(1) - 2.0).abs() < 1e-6
                && (max(2) - 3.0).abs() < 1e-6
        );
        // El triángulo debe girar en el sentido de su normal declarada.
        for t in m.idx.chunks(3) {
            let (a, b, c) = (
                m.verts[t[0] as usize],
                m.verts[t[1] as usize],
                m.verts[t[2] as usize],
            );
            let e1 = [
                b.pos[0] - a.pos[0],
                b.pos[1] - a.pos[1],
                b.pos[2] - a.pos[2],
            ];
            let e2 = [
                c.pos[0] - a.pos[0],
                c.pos[1] - a.pos[1],
                c.pos[2] - a.pos[2],
            ];
            let n = cross(e1, e2);
            assert!(
                n[0] * a.nrm[0] + n[1] * a.nrm[1] + n[2] * a.nrm[2] > 0.0,
                "orientación coherente con la normal"
            );
        }
    }

    #[test]
    fn polyhedra_have_the_right_face_counts_and_flat_outward_normals() {
        assert_eq!(Mesh::octahedron(1.0).idx.len() / 3, 8);
        assert_eq!(Mesh::icosahedron(1.0).idx.len() / 3, 20);
        for m in [Mesh::octahedron(1.0), Mesh::icosahedron(1.0)] {
            for t in m.idx.chunks(3) {
                let (a, b, c) = (
                    m.verts[t[0] as usize],
                    m.verts[t[1] as usize],
                    m.verts[t[2] as usize],
                );
                let ctr = [
                    (a.pos[0] + b.pos[0] + c.pos[0]) / 3.0,
                    (a.pos[1] + b.pos[1] + c.pos[1]) / 3.0,
                    (a.pos[2] + b.pos[2] + c.pos[2]) / 3.0,
                ];
                assert!(
                    ctr[0] * a.nrm[0] + ctr[1] * a.nrm[1] + ctr[2] * a.nrm[2] > 0.0,
                    "normal hacia fuera"
                );
                let e1 = [
                    b.pos[0] - a.pos[0],
                    b.pos[1] - a.pos[1],
                    b.pos[2] - a.pos[2],
                ];
                let e2 = [
                    c.pos[0] - a.pos[0],
                    c.pos[1] - a.pos[1],
                    c.pos[2] - a.pos[2],
                ];
                let n = cross(e1, e2);
                assert!(
                    n[0] * a.nrm[0] + n[1] * a.nrm[1] + n[2] * a.nrm[2] > 0.0,
                    "orientación coherente"
                );
            }
        }
    }

    #[test]
    fn the_icosahedron_vertices_lie_on_the_sphere() {
        for v in &Mesh::icosahedron(0.95).verts {
            let r = (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt();
            assert!((r - 0.95).abs() < 1e-4);
        }
    }

    #[test]
    fn wireframes_have_one_edge_per_unique_pair() {
        let ico = Mesh::icosahedron(1.0);
        assert_eq!(
            ico.wire_indices().len() / 2,
            30,
            "un icosaedro tiene 30 aristas"
        );
        let oct = Mesh::octahedron(1.0);
        assert_eq!(oct.wire_indices().len() / 2, 12, "un octaedro tiene 12");
    }

    #[test]
    fn a_ring_is_flat_and_between_its_radii() {
        let m = Mesh::ring(1.85, 2.22, 6);
        for v in &m.verts {
            let d = (v.pos[0].powi(2) + v.pos[1].powi(2)).sqrt();
            assert!((1.85 - 1e-4..=2.22 + 1e-4).contains(&d) && v.pos[2] == 0.0);
        }
    }

    #[test]
    fn a_capsule_spans_its_length_plus_both_caps() {
        let m = Mesh::capsule(0.035, 0.42, 6);
        let max = m.verts.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        let min = m.verts.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
        assert!((max - (0.21 + 0.035)).abs() < 1e-4 && (min + (0.21 + 0.035)).abs() < 1e-4);
    }

    #[test]
    fn a_disc_is_a_flat_fan_that_covers_its_radius() {
        let m = Mesh::disc(48.0, 72);
        assert_eq!(m.idx.len() / 3, 72);
        let max = m
            .verts
            .iter()
            .map(|v| (v.pos[0].powi(2) + v.pos[1].powi(2)).sqrt())
            .fold(0.0f32, f32::max);
        assert!((max - 48.0).abs() < 1e-3);
        for t in m.idx.chunks(3) {
            let (a, b, c) = (
                m.verts[t[0] as usize].pos,
                m.verts[t[1] as usize].pos,
                m.verts[t[2] as usize].pos,
            );
            let z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert!(z > 0.0, "cara hacia +Z");
        }
    }
}
