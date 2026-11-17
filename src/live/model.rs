//! BIA en 3D: el GLB de la web (`bia.glb`, 49 791 vértices) leído a memoria, con sus *morph targets*
//! (mandíbula, visemas, sonrisa, parpadeo) para animar la cara desde la CPU, y las tres mallas de la boca.
//!
//! El GLB no trae esqueleto ni animaciones, solo una malla con material PBR (color, normales,
//! metal-rugosidad) y morph targets: moverlos cada fotograma cuesta unos 50 000 vértices × unos pocos objetivos.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

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
}

/// Los morph targets que se usan, en el orden de `Face::weights`.
const TARGETS: [&str; 8] = [
    "jaw_open",
    "viseme_aa",
    "viseme_O",
    "viseme_E",
    "viseme_U",
    "mouth_smile",
    "blink_L",
    "blink_R",
];

impl Face {
    fn weights(&self) -> [f32; 8] {
        [
            self.jaw, self.aa, self.o, self.e, self.u, self.smile, self.blink, self.blink,
        ]
    }
}

pub struct Bia {
    base: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Desplazamientos de cada vértice por cada objetivo de `TARGETS` (vacío si el GLB no lo trae).
    deltas: [Vec<[f32; 3]>; 8],
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
                let mut deltas: [Vec<[f32; 3]>; 8] = Default::default();
                for (ti, (dp, _, _)) in r.read_morph_targets().enumerate() {
                    let Some(name) = names.get(ti) else { continue };
                    if let (Some(slot), Some(dp)) = (TARGETS.iter().position(|t| t == name), dp) {
                        deltas[slot] = dp.collect();
                    }
                }
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
                });
            }
        }
        let (base, indices, deltas) = face?;
        Some(Bia {
            base,
            indices,
            deltas,
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
        assert_eq!(b.vertex_count(), 49_791);
        assert_eq!(b.indices.len(), 166_512);
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
        assert_eq!(first.vertex_count(), 49_791);
        let second = take().expect("sin precarga, se lee aquí");
        assert_eq!(second.vertex_count(), 49_791);
    }
}
