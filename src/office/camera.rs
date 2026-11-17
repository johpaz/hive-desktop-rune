//! Cámara orbital y matrices. Lógica pura: no sabe nada de wgpu ni de Rune.

pub type Mat4 = [f32; 16]; // columna mayor, como WGSL
pub type Vec3 = [f32; 3];

pub const MIN_ELEVATION: f32 = 0.15; // rad: casi a ras de suelo, sin atravesarlo
pub const MAX_ELEVATION: f32 = 1.5658; // rad: casi cenital (π/2 − 0,005), sin voltearse en el polo; el enfoque de eventos mira casi desde arriba
pub const MIN_DISTANCE: f32 = 9.0;
pub const MAX_DISTANCE: f32 = 60.0;
/// Radianes por punto de arrastre.
pub const DRAG_SPEED: f32 = 0.006;
/// Fracción de distancia por unidad de rueda (la rueda de ratón da ±1 por muesca).
pub const ZOOM_SPEED: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub azimuth: f32,
    pub elevation: f32,
    pub distance: f32,
    pub target: Vec3,
    pub fov: f32,
}

/// Pose inicial de la UI web: cámara en (0, 15.5, 24.5) mirando a (0, 2.2, 0), con 42° de campo de visión.
impl Default for Camera {
    fn default() -> Self {
        let (dy, dz) = (15.5f32 - 2.2, 24.5f32);
        Camera {
            azimuth: std::f32::consts::FRAC_PI_2,
            elevation: dy.atan2(dz),
            distance: (dy * dy + dz * dz).sqrt(),
            target: [0.0, 2.2, 0.0],
            fov: 42.0f32.to_radians(),
        }
    }
}

impl Camera {
    /// Arrastrar a la derecha gira la escena hacia la derecha; hacia arriba sube la cámara.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.azimuth = (self.azimuth - dx * DRAG_SPEED).rem_euclid(std::f32::consts::TAU);
        self.elevation = (self.elevation + dy * DRAG_SPEED).clamp(MIN_ELEVATION, MAX_ELEVATION);
    }

    /// `delta` > 0 acerca (rueda hacia delante), < 0 aleja. Proporcional a la distancia para que se sienta igual de cerca que de lejos.
    pub fn zoom(&mut self, delta: f32) {
        self.distance =
            (self.distance * (1.0 - delta * ZOOM_SPEED)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn eye(&self) -> Vec3 {
        let (se, ce) = (self.elevation.sin(), self.elevation.cos());
        [
            self.target[0] + self.distance * ce * self.azimuth.cos(),
            self.target[1] + self.distance * se,
            self.target[2] + self.distance * ce * self.azimuth.sin(),
        ]
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        mul(
            &perspective(self.fov, aspect, 0.1, 200.0),
            &look_at(self.eye(), self.target),
        )
    }
}

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
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

/// Proyección con profundidad 0..1 (wgpu).
pub fn perspective(fov: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
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

pub fn look_at(eye: Vec3, at: Vec3) -> Mat4 {
    let norm = |v: Vec3| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let sub = |a: Vec3, b: Vec3| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: Vec3, b: Vec3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: Vec3, b: Vec3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
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

/// Proyecta un punto del mundo a coordenadas de pantalla en puntos (para el HUD y para elegir con el ratón).
/// `None` si queda detrás de la cámara.
pub fn project(vp: &Mat4, p: Vec3, size: (f32, f32)) -> Option<(f32, f32)> {
    let x = vp[0] * p[0] + vp[4] * p[1] + vp[8] * p[2] + vp[12];
    let y = vp[1] * p[0] + vp[5] * p[1] + vp[9] * p[2] + vp[13];
    let w = vp[3] * p[0] + vp[7] * p[1] + vp[11] * p[2] + vp[15];
    if w <= 1e-6 {
        return None;
    }
    Some((
        (x / w * 0.5 + 0.5) * size.0,
        (1.0 - (y / w * 0.5 + 0.5)) * size.1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_default_pose_is_the_web_one() {
        let c = Camera::default();
        let e = c.eye();
        assert!(
            close(e[0], 0.0) && close(e[1], 15.5) && close(e[2], 24.5),
            "{e:?}"
        );
        assert_eq!(c.target, [0.0, 2.2, 0.0]);
        assert!((c.fov.to_degrees() - 42.0).abs() < 1e-3);
    }

    #[test]
    fn the_distance_limits_are_the_web_ones() {
        assert_eq!((MIN_DISTANCE, MAX_DISTANCE), (9.0, 60.0));
        assert!(
            (MIN_ELEVATION - (std::f32::consts::FRAC_PI_2 - 1.42)).abs() < 0.01,
            "el límite polar de 1,42 rad"
        );
    }

    #[test]
    fn the_eye_sits_at_the_orbit_distance_from_the_target() {
        let c = Camera {
            target: [1.0, 2.0, 3.0],
            ..Camera::default()
        };
        let e = c.eye();
        let d = ((e[0] - 1.0).powi(2) + (e[1] - 2.0).powi(2) + (e[2] - 3.0).powi(2)).sqrt();
        assert!(close(d, c.distance), "{d}");
        assert!(e[1] > c.target[1], "mira desde arriba");
    }

    #[test]
    fn dragging_turns_the_azimuth_and_clamps_the_elevation() {
        let mut c = Camera::default();
        let az = c.azimuth;
        c.orbit(100.0, 0.0);
        assert!(c.azimuth != az && c.azimuth >= 0.0 && c.azimuth < std::f32::consts::TAU);
        c.orbit(0.0, 100000.0);
        assert!(close(c.elevation, MAX_ELEVATION), "no pasa del cénit");
        c.orbit(0.0, -100000.0);
        assert!(close(c.elevation, MIN_ELEVATION), "no atraviesa el suelo");
    }

    #[test]
    fn the_azimuth_wraps_instead_of_growing_forever() {
        let mut c = Camera::default();
        for _ in 0..1000 {
            c.orbit(50.0, 0.0);
        }
        assert!(c.azimuth >= 0.0 && c.azimuth < std::f32::consts::TAU);
    }

    #[test]
    fn zoom_is_proportional_and_clamped() {
        let mut c = Camera {
            distance: 20.0,
            ..Camera::default()
        };
        c.zoom(1.0);
        assert!(close(c.distance, 18.0), "{}", c.distance);
        c.zoom(-1.0);
        assert!(
            close(c.distance, 19.8),
            "alejar es proporcional, no simétrico exacto: {}",
            c.distance
        );
        for _ in 0..200 {
            c.zoom(1.0);
        }
        assert!(close(c.distance, MIN_DISTANCE));
        for _ in 0..200 {
            c.zoom(-1.0);
        }
        assert!(close(c.distance, MAX_DISTANCE));
    }

    #[test]
    fn the_target_projects_to_the_center_of_the_screen() {
        let c = Camera::default();
        let vp = c.view_proj(1.6);
        let (x, y) = project(&vp, c.target, (800.0, 500.0)).expect("delante de la cámara");
        assert!(close(x, 400.0) && close(y, 250.0), "{x} {y}");
    }

    #[test]
    fn a_point_behind_the_camera_does_not_project() {
        let c = Camera::default();
        let e = c.eye();
        let behind = [e[0] * 2.0, e[1] * 2.0, e[2] * 2.0];
        assert_eq!(project(&c.view_proj(1.0), behind, (100.0, 100.0)), None);
    }

    #[test]
    fn up_in_the_world_is_up_on_the_screen() {
        let c = Camera::default();
        let vp = c.view_proj(1.0);
        let below = project(&vp, [0.0, 0.0, 0.0], (100.0, 100.0)).unwrap();
        let above = project(&vp, [0.0, 2.0, 0.0], (100.0, 100.0)).unwrap();
        assert!(above.1 < below.1, "y de pantalla crece hacia abajo");
    }

    #[test]
    fn identity_is_neutral_for_mul() {
        let id: Mat4 = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let vp = Camera::default().view_proj(1.3);
        let m = mul(&vp, &id);
        assert!(vp.iter().zip(m.iter()).all(|(a, b)| close(*a, *b)));
    }
}
