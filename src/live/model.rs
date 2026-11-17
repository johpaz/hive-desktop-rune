//! BIA en 3D: el GLB de la web (`bia.glb`, 49 791 vértices) leído a memoria, con sus *morph targets*
//! (mandíbula, visemas, sonrisa, parpadeo) para animar la cara desde la CPU, y las tres mallas de la boca.
//!
//! El GLB trae una malla con material PBR (color, normales, metal-rugosidad), morph targets para la cara y un
//! esqueleto de 14 huesos (columna, cuello, cabeza, brazos, alas, cola). Todo se mueve desde la CPU cada
//! fotograma: primero los morph targets y luego el skinning, unos 50 000 vértices × pocas influencias.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use crate::office::camera::{mul, Mat4};
use crate::office::xform::{translate, IDENTITY};

use super::body::{Pose, BONES};

const GLB: &[u8] = include_bytes!("../../assets/bia.glb");

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub uv: [f32; 2],
}

/// Imagen RGBA8 ya decodificada.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Una de las mallas sueltas de la boca (cavidad, dientes, lengua): color plano y una posición propia.
pub struct Part {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub color: [f32; 4],
    pub roughness: f32,
    pub offset: [f32; 3],
    /// El hueso al que va atada (la cabeza para la boca): su matriz de piel mueve la pieza entera.
    pub joint: Option<usize>,
}

/// Cuánto vale cada gesto de la cara, de 0 a 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Face {
    pub jaw: f32,
    pub aa: f32,
    pub o: f32,
    pub e: f32,
    pub u: f32,
    pub smile: f32,
    pub blink: f32,
    pub brow_up: f32,
    pub brow_down_l: f32,
    pub brow_down_r: f32,
    pub brow_inner_up: f32,
    pub cheek_l: f32,
    pub cheek_r: f32,
    pub frown: f32,
}

/// Los morph targets que se usan, en el orden de `Face::weights`.
const N_TARGETS: usize = 15;
const TARGETS: [&str; N_TARGETS] = [
    "jaw_open",
    "viseme_aa",
    "viseme_O",
    "viseme_E",
    "viseme_U",
    "mouth_smile",
    "blink_L",
    "blink_R",
    "brow_up",
    "brow_down_L",
    "brow_down_R",
    "brow_inner_up",
    "cheek_squint_L",
    "cheek_squint_R",
    "mouth_frown",
];

impl Face {
    fn weights(&self) -> [f32; N_TARGETS] {
        [
            self.jaw,
            self.aa,
            self.o,
            self.e,
            self.u,
            self.smile,
            self.blink,
            self.blink,
            self.brow_up,
            self.brow_down_l,
            self.brow_down_r,
            self.brow_inner_up,
            self.cheek_l,
            self.cheek_r,
            self.frown,
        ]
    }
}

/// Un hueso en reposo: su posición y giro locales, su giro en el mundo del modelo y su matriz inversa de enlace.
struct Joint {
    parent: Option<usize>,
    t: [f32; 3],
    rot: Mat4,
    rest_world_rot: Mat4,
    inv_bind: Mat4,
    /// Posición en `BONES` (la pose que lo mueve), si es uno de los 14.
    bone: Option<usize>,
}

struct Skeleton {
    joints: Vec<Joint>,
}

/// Matriz de rotación (columna mayor) desde un cuaternión glTF `[x, y, z, w]`.
fn quat_mat(q: [f32; 4]) -> Mat4 {
    let [x, y, z, w] = q;
    let mut m = IDENTITY;
    m[0] = 1.0 - 2.0 * (y * y + z * z);
    m[1] = 2.0 * (x * y + z * w);
    m[2] = 2.0 * (x * z - y * w);
    m[4] = 2.0 * (x * y - z * w);
    m[5] = 1.0 - 2.0 * (x * x + z * z);
    m[6] = 2.0 * (y * z + x * w);
    m[8] = 2.0 * (x * z + y * w);
    m[9] = 2.0 * (y * z - x * w);
    m[10] = 1.0 - 2.0 * (x * x + y * y);
    m
}

/// La inversa de una rotación pura es su traspuesta.
fn transpose3(m: &Mat4) -> Mat4 {
    let mut o = IDENTITY;
    for r in 0..3 {
        for c in 0..3 {
            o[c * 4 + r] = m[r * 4 + c];
        }
    }
    o
}

/// Rotación de Euler XYZ (como three.js) de un giro en ejes del modelo.
fn euler_xyz(r: [f32; 3]) -> Mat4 {
    crate::office::xform::euler(r[0], r[1], r[2])
}

pub struct Bia {
    base: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Desplazamientos de cada vértice por cada objetivo de `TARGETS` (vacío si el GLB no lo trae).
    deltas: [Vec<[f32; 3]>; N_TARGETS],
    /// Por vértice: los cuatro huesos que lo mueven y cuánto cada uno (vacío si el GLB no trae esqueleto).
    skin: Vec<([u16; 4], [f32; 4])>,
    skeleton: Option<Skeleton>,
    pub parts: Vec<Part>,
    pub color: Image,
    pub normal: Image,
    pub metal_rough: Image,
}

fn decode(bytes: &[u8]) -> Option<Image> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(Image {
        w: rgba.width(),
        h: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// La lectura del GLB en curso (hilo lanzado al arrancar la app para no congelar la primera visita a HiveLive).
static PRELOAD: std::sync::Mutex<Option<std::thread::JoinHandle<Option<Bia>>>> =
    std::sync::Mutex::new(None);

/// Empieza a leer el modelo en segundo plano. Se puede llamar varias veces: solo la primera lanza el hilo.
pub fn preload() {
    let mut g = PRELOAD.lock().unwrap();
    if g.is_none() {
        *g = Some(std::thread::spawn(Bia::load));
    }
}

/// El modelo: el que se estaba leyendo en segundo plano (esperando a que acabe) o, si nadie lo pidió, leído aquí.
pub fn take() -> Option<Bia> {
    let handle = PRELOAD.lock().unwrap().take();
    match handle {
        Some(h) => h.join().ok().flatten(),
        None => Bia::load(),
    }
}

impl Bia {
    /// Lee el GLB embebido. `None` si no se puede (la pantalla cae entonces al retrato).
    pub fn load() -> Option<Bia> {
        Self::from_glb(GLB)
    }

    pub fn from_glb(bytes: &[u8]) -> Option<Bia> {
        let gltf = gltf::Gltf::from_slice(bytes).ok()?;
        let blob = gltf.blob.as_deref()?;
        let doc = &gltf.document;

        // Imágenes embebidas en el mismo binario: normal, base y metal-rugosidad (por su nombre).
        let mut by_name: HashMap<String, Image> = HashMap::new();
        for img in doc.images() {
            if let gltf::image::Source::View { view, .. } = img.source() {
                let data = &blob[view.offset()..view.offset() + view.length()];
                if let Some(d) = decode(data) {
                    by_name.insert(img.name().unwrap_or("").to_string(), d);
                }
            }
        }

        let mut face = None;
        let mut parts = Vec::new();
        let mut skeleton: Option<Skeleton> = None;
        let mut face_skin: Vec<([u16; 4], [f32; 4])> = Vec::new();
        // Padre de cada nodo, para armar la jerarquía de huesos.
        let mut parent_of: HashMap<usize, usize> = HashMap::new();
        for n in doc.nodes() {
            for c in n.children() {
                parent_of.insert(c.index(), n.index());
            }
        }
        for node in doc.nodes() {
            let Some(mesh) = node.mesh() else { continue };
            let prim = mesh.primitives().next()?;
            let r = prim.reader(|_| Some(blob)); // el GLB trae un único búfer: su binario
            let pos: Vec<[f32; 3]> = r.read_positions()?.collect();
            let nrm: Vec<[f32; 3]> = r
                .read_normals()
                .map(|n| n.collect())
                .unwrap_or_else(|| vec![[0.0, 0.0, 1.0]; pos.len()]);
            let uv: Vec<[f32; 2]> = r
                .read_tex_coords(0)
                .map(|t| t.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0; 2]; pos.len()]);
            let idx: Vec<u32> = r.read_indices()?.into_u32().collect();
            let weights: Vec<([u16; 4], [f32; 4])> = match (r.read_joints(0), r.read_weights(0)) {
                (Some(j), Some(w)) => j.into_u16().zip(w.into_f32()).collect(),
                _ => Vec::new(),
            };
            if skeleton.is_none() {
                if let Some(skin) = node.skin() {
                    let ibm: Vec<[[f32; 4]; 4]> = skin
                        .reader(|_| Some(blob))
                        .read_inverse_bind_matrices()
                        .map(|i| i.collect())
                        .unwrap_or_default();
                    let nodes: Vec<usize> = skin.joints().map(|j| j.index()).collect();
                    let joints = skin
                        .joints()
                        .enumerate()
                        .map(|(i, j)| {
                            let (t, q, _) = j.transform().decomposed();
                            let rot = quat_mat(q);
                            let flat = ibm.get(i).map(|m| {
                                let mut o = [0.0f32; 16];
                                for (c, col) in m.iter().enumerate() {
                                    o[c * 4..c * 4 + 4].copy_from_slice(col);
                                }
                                o
                            });
                            Joint {
                                parent: parent_of
                                    .get(&j.index())
                                    .and_then(|p| nodes.iter().position(|n| n == p)),
                                t,
                                rot,
                                rest_world_rot: IDENTITY,
                                inv_bind: flat.unwrap_or(IDENTITY),
                                bone: j.name().and_then(|n| BONES.iter().position(|b| *b == n)),
                            }
                        })
                        .collect::<Vec<_>>();
                    skeleton = Some(Skeleton { joints });
                }
            }
            let vertices: Vec<Vertex> = (0..pos.len())
                .map(|i| Vertex {
                    pos: pos[i],
                    nrm: nrm[i],
                    uv: uv[i],
                })
                .collect();
            if mesh.index() == 0 {
                // Los nombres de los morph targets van en los `extras` de la malla.
                let names: Vec<String> = mesh
                    .extras()
                    .as_ref()
                    .and_then(|e| serde_json::from_str::<serde_json::Value>(e.get()).ok())
                    .and_then(|v| {
                        v["targetNames"].as_array().map(|a| {
                            a.iter()
                                .filter_map(|n| n.as_str().map(str::to_string))
                                .collect()
                        })
                    })
                    .unwrap_or_default();
                let mut deltas: [Vec<[f32; 3]>; N_TARGETS] = Default::default();
                for (ti, (dp, _, _)) in r.read_morph_targets().enumerate() {
                    let Some(name) = names.get(ti) else { continue };
                    if let (Some(slot), Some(dp)) = (TARGETS.iter().position(|t| t == name), dp) {
                        deltas[slot] = dp.collect();
                    }
                }
                face_skin = weights.clone();
                face = Some((vertices, idx, deltas));
            } else {
                let pbr = prim.material().pbr_metallic_roughness();
                parts.push(Part {
                    name: node.name().unwrap_or("").to_string(),
                    vertices,
                    indices: idx,
                    color: pbr.base_color_factor(),
                    roughness: pbr.roughness_factor(),
                    offset: node.transform().decomposed().0,
                    // La boca va atada a un solo hueso (la cabeza): el del primer vértice.
                    joint: weights
                        .first()
                        .map(|(j, w)| j[w.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |m| m.0)] as usize),
                });
            }
        }
        let (base, indices, deltas) = face?;
        if let Some(sk) = skeleton.as_mut() {
            sk.settle_rest_rotations();
        }
        Some(Bia {
            base,
            indices,
            deltas,
            skin: face_skin,
            skeleton,
            parts,
            color: by_name.remove("base")?,
            normal: by_name.remove("normal")?,
            metal_rough: by_name.remove("metal-rough")?,
        })
    }

    pub fn vertex_count(&self) -> usize {
        self.base.len()
    }

    /// Los vértices con la cara en su gesto actual, en `out` (se reutiliza entre fotogramas).
    pub fn morphed(&self, face: &Face, out: &mut Vec<Vertex>) {
        out.clear();
        out.extend_from_slice(&self.base);
        for (w, delta) in face.weights().iter().zip(&self.deltas) {
            if *w == 0.0 || delta.len() != out.len() {
                continue;
            }
            for (v, d) in out.iter_mut().zip(delta) {
                v.pos[0] += d[0] * w;
                v.pos[1] += d[1] * w;
                v.pos[2] += d[2] * w;
            }
        }
    }

    /// Cuántos huesos de `BONES` trae el GLB (0 si no hay esqueleto).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn bone_count(&self) -> usize {
        self.skeleton
            .as_ref()
            .map_or(0, |s| s.joints.iter().filter(|j| j.bone.is_some()).count())
    }

    /// Las matrices de piel de cada hueso para una pose (vacío sin esqueleto).
    pub fn skin_matrices(&self, pose: &Pose) -> Vec<Mat4> {
        self.skeleton.as_ref().map_or_else(Vec::new, |s| s.skin_matrices(pose))
    }

    /// La matriz de piel del hueso al que va atada una pieza.
    pub fn joint_matrix(skin: &[Mat4], joint: Option<usize>) -> Mat4 {
        joint.and_then(|j| skin.get(j)).copied().unwrap_or(IDENTITY)
    }

    /// Mueve los vértices (ya con la cara) según el esqueleto: posición y normal, mezclando hasta cuatro huesos.
    pub fn skinned(&self, skin: &[Mat4], out: &mut [Vertex]) {
        if skin.is_empty() || self.skin.len() != out.len() {
            return;
        }
        for (v, (joints, weights)) in out.iter_mut().zip(&self.skin) {
            let (p, n) = (v.pos, v.nrm);
            let (mut np, mut nn) = ([0.0f32; 3], [0.0f32; 3]);
            for (j, w) in joints.iter().zip(weights) {
                if *w <= 0.0 {
                    continue;
                }
                let Some(m) = skin.get(*j as usize) else { continue };
                for k in 0..3 {
                    np[k] += w * (m[k] * p[0] + m[4 + k] * p[1] + m[8 + k] * p[2] + m[12 + k]);
                    nn[k] += w * (m[k] * n[0] + m[4 + k] * n[1] + m[8 + k] * n[2]);
                }
            }
            let len = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
            v.pos = np;
            if len > 1e-6 {
                v.nrm = [nn[0] / len, nn[1] / len, nn[2] / len];
            }
        }
    }
}

impl Skeleton {
    /// El giro de cada hueso en el mundo del modelo, en reposo: hace falta para llevar un giro en ejes del
    /// modelo al marco local del hueso.
    fn settle_rest_rotations(&mut self) {
        for i in 0..self.joints.len() {
            let mut chain = vec![i];
            while let Some(p) = self.joints[*chain.last().unwrap()].parent {
                chain.push(p);
            }
            let world = chain.iter().rev().fold(IDENTITY, |acc, &j| mul(&acc, &self.joints[j].rot));
            self.joints[i].rest_world_rot = world;
        }
    }

    /// `piel[j] = mundo[j] · inversa_de_enlace[j]`, con el giro de la pose aplicado a cada hueso:
    /// local = reposo_local · (reposo_mundo⁻¹ · giro · reposo_mundo).
    fn skin_matrices(&self, pose: &Pose) -> Vec<Mat4> {
        let mut world: Vec<Option<Mat4>> = vec![None; self.joints.len()];
        fn solve(sk: &Skeleton, pose: &Pose, world: &mut [Option<Mat4>], i: usize) -> Mat4 {
            if let Some(m) = world[i] {
                return m;
            }
            let j = &sk.joints[i];
            let delta = j.bone.map_or(IDENTITY, |b| {
                mul(&mul(&transpose3(&j.rest_world_rot), &euler_xyz(pose[b])), &j.rest_world_rot)
            });
            let local = mul(&mul(&translate(j.t), &j.rot), &delta);
            let m = match j.parent {
                Some(p) => mul(&solve(sk, pose, world, p), &local),
                None => local,
            };
            world[i] = Some(m);
            m
        }
        (0..self.joints.len())
            .map(|i| mul(&solve(self, pose, &mut world, i), &self.joints[i].inv_bind))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bia() -> Bia {
        Bia::load().expect("el GLB embebido se lee")
    }

    #[test]
    fn the_glb_loads_with_its_face_mouth_parts_and_three_textures() {
        let b = bia();
        assert_eq!(b.vertex_count(), 49_825);
        assert_eq!(b.indices.len(), 166_365);
        assert!(
            b.indices.iter().all(|&i| (i as usize) < b.vertex_count()),
            "los índices caen dentro de la malla"
        );
        let names: Vec<&str> = b.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["boca_cavidad", "boca_dientes", "boca_lengua"]);
        for t in [&b.color, &b.normal, &b.metal_rough] {
            assert!(t.w >= 256 && t.h >= 256 && t.rgba.len() == (t.w * t.h * 4) as usize);
        }
    }

    #[test]
    fn the_model_is_two_units_tall_and_centred_like_the_webs() {
        let b = bia();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &b.base {
            for k in 0..3 {
                min[k] = min[k].min(v.pos[k]);
                max[k] = max[k].max(v.pos[k]);
            }
        }
        assert!(
            (max[1] - 1.0).abs() < 0.01 && (min[1] + 1.0).abs() < 0.01,
            "alto de -1 a 1: {min:?} {max:?}"
        );
        assert!((max[0] - min[0] - 0.862).abs() < 0.01, "ancho de unos 0,86");
    }

    #[test]
    fn opening_the_jaw_moves_the_mouth_and_nothing_else_much() {
        let b = bia();
        let (mut closed, mut open) = (vec![], vec![]);
        b.morphed(&Face::default(), &mut closed);
        b.morphed(
            &Face {
                jaw: 1.0,
                ..Default::default()
            },
            &mut open,
        );
        let moved: Vec<f32> = closed
            .iter()
            .zip(&open)
            .map(|(a, c)| (0..3).map(|k| (a.pos[k] - c.pos[k]).abs()).sum::<f32>())
            .collect();
        let n = moved.iter().filter(|d| **d > 1e-4).count();
        assert!(n > 100, "la mandíbula mueve vértices: {n}");
        assert!(n < b.vertex_count() / 2, "pero no toda la malla: {n}");
        let ys: Vec<f32> = closed
            .iter()
            .zip(&open)
            .filter(|(a, c)| (a.pos[1] - c.pos[1]).abs() > 1e-3)
            .map(|(a, _)| a.pos[1])
            .collect();
        assert!(
            !ys.is_empty() && ys.iter().all(|y| *y > 0.4 && *y < 0.9),
            "lo que se mueve está a la altura de la cara/boca"
        );
    }

    #[test]
    fn a_neutral_face_is_the_untouched_mesh_and_blinking_squashes_the_eyes() {
        let b = bia();
        let (mut a, mut c) = (vec![], vec![]);
        b.morphed(&Face::default(), &mut a);
        assert_eq!(a, b.base, "sin gestos, la malla original");
        b.morphed(
            &Face {
                blink: 1.0,
                ..Default::default()
            },
            &mut c,
        );
        assert!(a != c, "el parpadeo cambia la malla");
    }

    #[test]
    fn preloading_hands_over_the_model_once_and_later_calls_load_it_again() {
        preload();
        preload(); // la segunda no lanza otro hilo
        let first = take().expect("el modelo precargado");
        assert_eq!(first.vertex_count(), 49_825);
        let second = take().expect("sin precarga, se lee aquí");
        assert_eq!(second.vertex_count(), 49_825);
    }

    #[test]
    fn the_glb_brings_the_fourteen_bones_and_a_skin_for_every_vertex() {
        let b = bia();
        assert_eq!(b.bone_count(), BONES.len());
        assert_eq!(b.skin.len(), b.vertex_count());
        for (_, w) in &b.skin {
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-3, "los pesos de cada vértice suman 1: {sum}");
        }
    }

    #[test]
    fn at_rest_the_skeleton_leaves_the_mesh_where_it_was() {
        let b = bia();
        let skin = b.skin_matrices(&Default::default());
        let mut v = b.base.clone();
        b.skinned(&skin, &mut v);
        let worst = v
            .iter()
            .zip(&b.base)
            .map(|(a, c)| (0..3).map(|k| (a.pos[k] - c.pos[k]).abs()).fold(0.0, f32::max))
            .fold(0.0, f32::max);
        assert!(worst < 2e-3, "sin giro no se mueve nada: {worst}");
    }

    #[test]
    fn turning_the_head_moves_the_face_but_not_the_hips() {
        let b = bia();
        let mut pose: Pose = Default::default();
        pose[3] = [0.0, 0.0, 0.3]; // la cabeza se ladea
        let skin = b.skin_matrices(&pose);
        let mut v = b.base.clone();
        b.skinned(&skin, &mut v);
        let moved = |y0: f32, y1: f32| {
            v.iter()
                .zip(&b.base)
                .filter(|(_, c)| c.pos[1] > y0 && c.pos[1] < y1)
                .map(|(a, c)| (0..3).map(|k| (a.pos[k] - c.pos[k]).abs()).sum::<f32>())
                .fold(0.0, f32::max)
        };
        assert!(moved(0.78, 1.0) > 0.03, "la cabeza se mueve");
        assert!(moved(-1.0, 0.1) < 2e-3, "la cola y la cadera quedan quietas");
    }

    #[test]
    fn the_mouth_parts_ride_on_the_head_bone() {
        let b = bia();
        let head = BONES.iter().position(|n| *n == "head").unwrap();
        let sk = b.skeleton.as_ref().unwrap();
        for p in &b.parts {
            let j = p.joint.expect("la boca va atada a un hueso");
            assert_eq!(sk.joints[j].bone, Some(head), "{} sigue a la cabeza", p.name);
        }
    }

    #[test]
    fn the_new_expressions_move_the_brows_and_cheeks() {
        let b = bia();
        let (mut a, mut c) = (vec![], vec![]);
        b.morphed(&Face::default(), &mut a);
        b.morphed(&Face { brow_up: 1.0, cheek_l: 1.0, frown: 1.0, ..Default::default() }, &mut c);
        let n = a.iter().zip(&c).filter(|(x, y)| x.pos != y.pos).count();
        assert!(n > 100, "las cejas, la mejilla y el ceño mueven vértices: {n}");
        assert!(n < b.vertex_count() / 4, "pero solo la cara: {n}");
    }
}
