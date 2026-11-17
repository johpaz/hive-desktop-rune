//! Fuentes WGSL. Los shaders de la escena comparten `math` y `common`; el posproceso solo `math`
//! (sus enlaces de grupo 0 chocarían con los globales de la escena).

pub const MATH: &str = include_str!("shaders/math.wgsl");
pub const COMMON: &str = include_str!("shaders/common.wgsl");

pub const FLOOR: &str = include_str!("shaders/floor.wgsl");
pub const SOLID: &str = include_str!("shaders/solid.wgsl");
pub const BASIC: &str = include_str!("shaders/basic.wgsl");
pub const HOLOGRAM: &str = include_str!("shaders/hologram.wgsl");
pub const BEAM: &str = include_str!("shaders/beam.wgsl");
pub const SPRITE: &str = include_str!("shaders/sprite.wgsl");
pub const POST: &str = include_str!("shaders/post.wgsl");
pub const BLIT: &str = include_str!("shaders/blit.wgsl");

/// Shader de escena completo.
pub fn scene(body: &str) -> String {
    format!("{MATH}\n{COMMON}\n{body}")
}

/// Shader de posproceso completo.
pub fn post() -> String {
    format!("{MATH}\n{POST}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Analiza y valida el WGSL como lo haría wgpu al crear el módulo, sin necesitar una GPU.
    fn validate(name: &str, src: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        module
    }

    fn entries(m: &naga::Module) -> Vec<String> {
        m.entry_points.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn every_scene_shader_validates_and_has_its_entry_points() {
        for (name, body, expect) in [
            ("floor", FLOOR, vec!["vs", "fs"]),
            ("solid", SOLID, vec!["vs", "fs_opaque", "fs_alpha"]),
            ("basic", BASIC, vec!["vs", "fs_opaque", "fs_add"]),
            ("hologram", HOLOGRAM, vec!["vs", "fs"]),
            ("beam", BEAM, vec!["vs", "fs"]),
            ("sprite", SPRITE, vec!["vs", "fs"]),
        ] {
            let m = validate(name, &scene(body));
            let got = entries(&m);
            for e in expect {
                assert!(
                    got.iter().any(|g| g == e),
                    "{name} no tiene el punto de entrada {e}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn the_post_shader_validates_with_all_its_passes() {
        let m = validate("post", &post());
        let got = entries(&m);
        for e in ["vs", "fs_thr", "fs_down", "fs_up", "fs_comp"] {
            assert!(got.iter().any(|g| g == e), "post no tiene {e}: {got:?}");
        }
    }

    #[test]
    fn the_blit_shader_validates() {
        let m = validate("blit", BLIT);
        assert!(entries(&m).iter().any(|e| e == "fs"));
    }

    #[test]
    fn the_reversed_smoothstep_helper_behaves_like_glsl() {
        // La fórmula que usa `ss`: t = clamp((x - e0)/(e1 - e0)), t²(3 - 2t); con e0 > e1 invierte el sentido.
        let ss = |e0: f32, e1: f32, x: f32| {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        assert_eq!(
            ss(46.0, 14.0, 60.0),
            0.0,
            "más allá del borde lejano se desvanece"
        );
        assert_eq!(ss(46.0, 14.0, 0.0), 1.0, "en el centro es pleno");
        assert!((ss(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((ss(1.0, 0.0, 0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_scene_and_post_bindings_do_not_collide() {
        let post_src = post();
        assert!(
            !post_src.contains("struct Globals"),
            "el posproceso no incluye los globales de la escena"
        );
        assert!(scene(HOLOGRAM).contains("struct Globals"));
    }
}
