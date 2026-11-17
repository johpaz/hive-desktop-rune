//! Director de cámara: la intro, el enfoque automático en el evento que se presenta, el regreso a la pose de
//! reposo y el seguimiento de la selección. Lógica pura sobre `Camera`.
//! Cifras de `scene/CameraRig.tsx` y `scene/cameraFocus.ts` de la UI web.

use super::camera::{Camera, Vec3};

pub const INTRO_FROM: Vec3 = [0.0, 46.0, 62.0];
pub const INTRO_TO: Vec3 = [0.0, 15.5, 24.5];
pub const INTRO_SECONDS: f32 = 1.35;
pub const DEFAULT_TARGET: Vec3 = [0.0, 2.2, 0.0];
/// Punto de la reina al que apunta la cámara (más alto que su suelo).
pub const CORE_FOCUS: Vec3 = [0.0, 3.1, 0.0];
pub const RETURN_EPSILON: f32 = 0.025;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub eye: Vec3,
    pub target: Vec3,
}

pub const DEFAULT_POSE: Pose = Pose {
    eye: INTRO_TO,
    target: DEFAULT_TARGET,
};

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len2(v: Vec3) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

fn lerp3(a: Vec3, b: Vec3, k: f32) -> Vec3 {
    [
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
    ]
}

pub fn pose_of(cam: &Camera) -> Pose {
    Pose {
        eye: cam.eye(),
        target: cam.target,
    }
}

/// Coloca la cámara orbital en una pose cartesiana. Los límites de distancia son del zoom del usuario, no de las
/// poses: la intro empieza más lejos del tope, y el primer zoom la devuelve al rango.
pub fn apply_pose(cam: &mut Camera, p: &Pose) {
    let d = sub(p.eye, p.target);
    let dist = len2(d).sqrt().max(0.5);
    cam.target = p.target;
    cam.distance = dist;
    cam.elevation = (d[1] / len2(d).sqrt().max(1e-6))
        .clamp(-1.0, 1.0)
        .asin()
        .clamp(super::camera::MIN_ELEVATION, super::camera::MAX_ELEVATION);
    cam.azimuth = d[2].atan2(d[0]).rem_euclid(std::f32::consts::TAU);
}

/// Pose para ver un evento entre dos puntos: casi desde arriba, más alto cuanto más separados estén.
pub fn event_pose(source: Vec3, target: Vec3) -> Pose {
    let mid = [
        (source[0] + target[0]) / 2.0,
        (source[1] + target[1]) / 2.0,
        (source[2] + target[2]) / 2.0,
    ];
    let spread = len2(sub(source, target)).sqrt();
    let height = (14.0 + spread * 1.05).max(22.0);
    Pose {
        eye: [mid[0], mid[1] + height, mid[2] + 0.8],
        target: mid,
    }
}

/// Pose para seguir una abeja seleccionada.
pub fn follow_pose(p: Vec3) -> Pose {
    Pose {
        eye: [p[0], p[1] + 7.0, p[2] + 11.0],
        target: p,
    }
}

/// Inercia de la órbita: al soltar el arrastre la cámara sigue girando y se frena sola, como el *damping* de
/// `OrbitControls` (cada fotograma conserva el 94 % de la velocidad).
pub const DAMPING: f32 = 0.06;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inertia {
    /// Último desplazamiento del arrastre (puntos por fotograma): lo que se conserva al soltar.
    last: (f32, f32),
    v: (f32, f32),
}

impl Inertia {
    /// Anota el desplazamiento de un movimiento del arrastre.
    pub fn track(&mut self, dx: f32, dy: f32) {
        self.last = (dx, dy);
    }

    /// Al soltar, la velocidad es la del último movimiento.
    pub fn release(&mut self) {
        self.v = self.last;
        self.last = (0.0, 0.0);
    }

    /// Cancela la inercia (nuevo arrastre, el director toma la cámara).
    pub fn stop(&mut self) {
        *self = Inertia::default();
    }

    pub fn is_moving(&self) -> bool {
        self.v.0.abs() > 0.01 || self.v.1.abs() > 0.01
    }

    /// Desplazamiento de este fotograma y frenado. Independiente de la velocidad de fotogramas.
    pub fn step(&mut self, dt: f32) -> (f32, f32) {
        if !self.is_moving() {
            self.v = (0.0, 0.0);
            return (0.0, 0.0);
        }
        // Integral exacta del frenado exponencial: el mismo recorrido con 30 o con 144 fotogramas por segundo.
        let lambda = -(1.0 - DAMPING).ln();
        let keep = (-lambda * dt * 60.0).exp();
        let gain = (1.0 - keep) / lambda;
        let out = (self.v.0 * gain, self.v.1 * gain);
        self.v = (self.v.0 * keep, self.v.1 * keep);
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// La cámara baja hasta su pose; los controles no responden.
    Intro,
    /// La persona manda.
    Free,
    /// Mira el evento que se presenta.
    Focus,
    /// Vuelve a la pose de reposo (o a la selección).
    Return,
    /// Persigue la abeja seleccionada.
    Follow,
}

#[derive(Debug)]
pub struct Rig {
    pub mode: Mode,
    started: bool,
    intro: f32,
    /// Donde la persona dejó la cámara la última vez que soltó el arrastre.
    pub resting: Pose,
    following: bool,
    had_selection: bool,
}

impl Default for Rig {
    fn default() -> Self {
        Rig {
            mode: Mode::Intro,
            started: false,
            intro: 0.0,
            resting: DEFAULT_POSE,
            following: false,
            had_selection: false,
        }
    }
}

fn ease_out_cubic(k: f32) -> f32 {
    1.0 - (1.0 - k).powi(3)
}

impl Rig {
    /// ¿Responden los controles? No durante la intro.
    pub fn controls_enabled(&self) -> bool {
        self.mode != Mode::Intro
    }

    /// Empieza un arrastre: la persona toma el mando y se cancelan el enfoque y el seguimiento.
    pub fn drag_start(&mut self) {
        if self.controls_enabled() {
            self.mode = Mode::Free;
            self.following = false;
        }
    }

    /// Termina un arrastre: esa pose pasa a ser la de reposo.
    pub fn drag_end(&mut self, cam: &Camera) {
        if self.controls_enabled() {
            self.resting = pose_of(cam);
        }
    }

    /// Un fotograma. `cue` son los puntos origen y destino del evento que se presenta; `selected`, la posición
    /// de lo seleccionado. Con `calm` apagado no hay director (y no hay intro).
    pub fn step(
        &mut self,
        cam: &mut Camera,
        dt: f32,
        calm: bool,
        cue: Option<(Vec3, Vec3)>,
        selected: Option<Vec3>,
    ) {
        if !self.started {
            self.started = true;
            if calm {
                apply_pose(
                    cam,
                    &Pose {
                        eye: INTRO_FROM,
                        target: DEFAULT_TARGET,
                    },
                );
                self.mode = Mode::Intro;
            } else {
                apply_pose(cam, &DEFAULT_POSE);
                self.mode = Mode::Free;
            }
        }

        if self.mode == Mode::Intro {
            self.intro += dt;
            let k = (self.intro / INTRO_SECONDS).min(1.0);
            apply_pose(
                cam,
                &Pose {
                    eye: lerp3(INTRO_FROM, INTRO_TO, ease_out_cubic(k)),
                    target: DEFAULT_TARGET,
                },
            );
            if k >= 1.0 {
                self.mode = Mode::Free;
                self.resting = DEFAULT_POSE;
            }
            self.had_selection = selected.is_some();
            return;
        }

        // Seleccionar algo nuevo activa el seguimiento (hasta que la persona arrastre).
        if selected.is_some() && !self.had_selection {
            self.following = true;
        }
        let lost_selection = selected.is_none() && self.had_selection;
        self.had_selection = selected.is_some();

        if !calm {
            self.mode = Mode::Free;
            return;
        }

        let desired = if let Some((from, to)) = cue {
            self.mode = Mode::Focus;
            Some(event_pose(from, to))
        } else {
            if self.mode == Mode::Focus {
                self.mode = Mode::Return; // el evento terminó
            }
            match (selected, self.following) {
                (Some(p), true) => {
                    self.mode = Mode::Follow;
                    Some(follow_pose(p))
                }
                _ => {
                    if self.mode == Mode::Follow || lost_selection {
                        self.mode = Mode::Return;
                    }
                    (self.mode == Mode::Return).then_some(self.resting)
                }
            }
        };
        let Some(want) = desired else { return };

        let now = pose_of(cam);
        let target = lerp3(now.target, want.target, 1.0 - 0.035f32.powf(dt));
        let eye = lerp3(now.eye, want.eye, 1.0 - 0.06f32.powf(dt));
        apply_pose(cam, &Pose { eye, target });
        if self.mode == Mode::Return
            && len2(sub(target, want.target)) < RETURN_EPSILON
            && len2(sub(eye, want.eye)) < RETURN_EPSILON
        {
            self.mode = Mode::Free;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3, eps: f32) -> bool {
        len2(sub(a, b)).sqrt() < eps
    }

    fn run(
        rig: &mut Rig,
        cam: &mut Camera,
        secs: f32,
        calm: bool,
        cue: Option<(Vec3, Vec3)>,
        sel: Option<Vec3>,
    ) {
        let mut t = 0.0;
        while t < secs {
            rig.step(cam, 1.0 / 60.0, calm, cue, sel);
            t += 1.0 / 60.0;
        }
    }

    fn settled() -> (Rig, Camera) {
        let (mut rig, mut cam) = (Rig::default(), Camera::default());
        run(&mut rig, &mut cam, 2.0, true, None, None);
        (rig, cam)
    }

    #[test]
    fn a_pose_survives_the_round_trip_through_the_orbit_camera() {
        for p in [
            DEFAULT_POSE,
            Pose {
                eye: [5.0, 20.0, -9.0],
                target: [1.0, 2.0, 3.0],
            },
            Pose {
                eye: [-12.0, 6.0, 4.0],
                target: [0.0, 3.1, 0.0],
            },
        ] {
            let mut cam = Camera::default();
            apply_pose(&mut cam, &p);
            assert!(
                close(cam.eye(), p.eye, 1e-3),
                "{:?} → {:?}",
                p.eye,
                cam.eye()
            );
            assert_eq!(cam.target, p.target);
        }
    }

    #[test]
    fn the_intro_starts_far_and_high_with_the_controls_off_and_lands_on_the_default_pose() {
        let (mut rig, mut cam) = (Rig::default(), Camera::default());
        rig.step(&mut cam, 0.0, true, None, None);
        assert_eq!(rig.mode, Mode::Intro);
        assert!(!rig.controls_enabled());
        assert!(close(cam.eye(), INTRO_FROM, 1e-2), "{:?}", cam.eye());
        run(&mut rig, &mut cam, 0.6, true, None, None);
        assert!(
            cam.eye()[1] < 46.0 && cam.eye()[1] > 15.5,
            "a mitad de camino"
        );
        run(&mut rig, &mut cam, 1.0, true, None, None);
        assert_eq!(rig.mode, Mode::Free);
        assert!(rig.controls_enabled());
        assert!(
            close(cam.eye(), INTRO_TO, 1e-2) && close(cam.target, DEFAULT_TARGET, 1e-4),
            "aterriza en la pose inicial: {:?}",
            cam.eye()
        );
        assert_eq!(rig.resting, DEFAULT_POSE);
    }

    #[test]
    fn the_intro_eases_out_so_it_slows_down_at_the_end() {
        let (mut rig, mut cam) = (Rig::default(), Camera::default());
        rig.step(&mut cam, 0.0, true, None, None);
        let mut ys = vec![cam.eye()[1]];
        for _ in 0..81 {
            rig.step(&mut cam, 1.0 / 60.0, true, None, None);
            ys.push(cam.eye()[1]);
        }
        let early = ys[0] - ys[10];
        let late = ys[70] - ys[80];
        assert!(
            early > late * 3.0,
            "baja deprisa al principio y frena al final: {early} frente a {late}"
        );
    }

    #[test]
    fn with_motion_off_there_is_no_intro_and_no_director() {
        let (mut rig, mut cam) = (Rig::default(), Camera::default());
        run(&mut rig, &mut cam, 0.1, false, None, None);
        assert_eq!(rig.mode, Mode::Free);
        assert!(
            close(cam.eye(), INTRO_TO, 1e-2),
            "sin intro: ya en su sitio"
        );
        let before = cam.eye();
        run(
            &mut rig,
            &mut cam,
            3.0,
            false,
            Some(([0.0; 3], [10.0, 4.0, 0.0])),
            Some([8.0, 3.0, 1.0]),
        );
        assert_eq!(
            cam.eye(),
            before,
            "con el movimiento apagado la cámara no se mueve sola"
        );
    }

    #[test]
    fn the_event_pose_looks_down_from_above_the_midpoint_and_rises_with_the_spread() {
        let near = event_pose([0.0, 4.4, 0.0], [2.0, 4.4, 0.0]);
        assert_eq!(near.target, [1.0, 4.4, 0.0]);
        assert!(
            (near.eye[1] - (4.4 + 22.0)).abs() < 1e-4,
            "altura mínima de 22: {:?}",
            near.eye
        );
        assert!((near.eye[2] - 0.8).abs() < 1e-5 && (near.eye[0] - 1.0).abs() < 1e-5);
        let far = event_pose([-10.0, 4.4, 0.0], [10.0, 4.4, 0.0]);
        assert!(
            (far.eye[1] - (4.4 + 14.0 + 20.0 * 1.05)).abs() < 1e-3,
            "sube con la separación: {:?}",
            far.eye
        );
        let same = event_pose([3.0, 4.0, 3.0], [3.0, 4.0, 3.0]);
        assert_eq!(
            same.target,
            [3.0, 4.0, 3.0],
            "un punto solo también funciona"
        );
    }

    #[test]
    fn an_active_event_pulls_the_camera_over_it_and_it_returns_when_the_event_ends() {
        let (mut rig, mut cam) = settled();
        let cue = ([0.0, 3.1, 0.0], [9.0, 4.4, 2.0]);
        run(&mut rig, &mut cam, 0.1, true, Some(cue), None);
        assert_eq!(rig.mode, Mode::Focus);
        run(&mut rig, &mut cam, 6.0, true, Some(cue), None);
        let want = event_pose(cue.0, cue.1);
        assert!(
            close(cam.target, want.target, 0.3) && close(cam.eye(), want.eye, 0.6),
            "converge sobre el evento: {:?} / {:?}",
            cam.eye(),
            want.eye
        );
        run(&mut rig, &mut cam, 10.0, true, None, None);
        assert_eq!(rig.mode, Mode::Free, "vuelve y suelta el mando");
        assert!(
            close(cam.eye(), INTRO_TO, 0.2) && close(cam.target, DEFAULT_TARGET, 0.2),
            "regresa a la pose de reposo: {:?}",
            cam.eye()
        );
    }

    #[test]
    fn the_camera_moves_gradually_not_in_a_jump() {
        let (mut rig, mut cam) = settled();
        let start = cam.eye();
        rig.step(
            &mut cam,
            1.0 / 60.0,
            true,
            Some(([0.0; 3], [9.0, 4.0, 2.0])),
            None,
        );
        let moved = len2(sub(cam.eye(), start)).sqrt();
        assert!(
            moved > 0.0 && moved < 3.0,
            "un fotograma mueve poco: {moved}"
        );
    }

    #[test]
    fn selecting_follows_the_bee_and_deselecting_returns_to_rest() {
        let (mut rig, mut cam) = settled();
        let bee = [8.0, 3.0, -4.0];
        run(&mut rig, &mut cam, 8.0, true, None, Some(bee));
        assert_eq!(rig.mode, Mode::Follow);
        let want = follow_pose(bee);
        assert!(close(cam.target, want.target, 0.3) && close(cam.eye(), want.eye, 0.6));
        run(&mut rig, &mut cam, 10.0, true, None, None);
        assert_eq!(rig.mode, Mode::Free);
        assert!(
            close(cam.eye(), INTRO_TO, 0.3),
            "al soltar la selección regresa: {:?}",
            cam.eye()
        );
    }

    #[test]
    fn dragging_cancels_the_follow_and_the_new_pose_becomes_the_resting_one() {
        let (mut rig, mut cam) = settled();
        let bee = [8.0, 3.0, -4.0];
        run(&mut rig, &mut cam, 3.0, true, None, Some(bee));
        assert_eq!(rig.mode, Mode::Follow);
        rig.drag_start();
        assert_eq!(rig.mode, Mode::Free);
        cam.orbit(120.0, 0.0);
        rig.drag_end(&cam);
        let pose = rig.resting;
        let held = cam.eye();
        run(&mut rig, &mut cam, 3.0, true, None, Some(bee));
        assert!(
            close(cam.eye(), held, 1e-4),
            "con la selección aún activa, la cámara ya no la sigue"
        );
        assert_eq!(rig.resting, pose);
        assert!(close(pose.eye, held, 1e-3));
    }

    #[test]
    fn an_event_interrupts_following_and_returns_to_the_selection_afterwards() {
        let (mut rig, mut cam) = settled();
        let bee = [8.0, 3.0, -4.0];
        run(&mut rig, &mut cam, 8.0, true, None, Some(bee));
        let cue = ([0.0, 3.1, 0.0], [-9.0, 4.4, 5.0]);
        run(&mut rig, &mut cam, 6.0, true, Some(cue), Some(bee));
        assert_eq!(rig.mode, Mode::Focus);
        run(&mut rig, &mut cam, 8.0, true, None, Some(bee));
        assert_eq!(
            rig.mode,
            Mode::Follow,
            "al acabar el evento vuelve a seguir a la abeja"
        );
        assert!(close(cam.target, bee, 0.3));
    }

    #[test]
    fn the_controls_ignore_a_drag_during_the_intro() {
        let (mut rig, mut cam) = (Rig::default(), Camera::default());
        rig.step(&mut cam, 0.0, true, None, None);
        rig.drag_start();
        assert_eq!(rig.mode, Mode::Intro, "la intro no se interrumpe");
        let before = rig.resting;
        rig.drag_end(&cam);
        assert_eq!(rig.resting, before);
    }

    #[test]
    fn poses_are_applied_as_given_and_the_next_zoom_brings_them_back_into_range() {
        use super::super::camera::{MAX_DISTANCE, MIN_DISTANCE};
        let mut cam = Camera::default();
        apply_pose(
            &mut cam,
            &Pose {
                eye: INTRO_FROM,
                target: DEFAULT_TARGET,
            },
        );
        assert!(
            cam.distance > MAX_DISTANCE,
            "la intro empieza fuera del tope de zoom: {}",
            cam.distance
        );
        cam.zoom(0.0);
        assert_eq!(
            cam.distance, MAX_DISTANCE,
            "y el zoom del usuario la devuelve al rango"
        );
        apply_pose(
            &mut cam,
            &Pose {
                eye: [0.0, 3.0, 1.0],
                target: [0.0, 2.2, 0.0],
            },
        );
        assert!(cam.distance < MIN_DISTANCE);
        cam.zoom(0.0);
        assert_eq!(cam.distance, MIN_DISTANCE);
    }

    // ── inercia ──

    #[test]
    fn a_released_drag_keeps_turning_and_slows_to_a_stop() {
        let mut i = Inertia::default();
        i.track(10.0, 0.0);
        assert!(!i.is_moving(), "mientras se arrastra no hay inercia");
        i.release();
        assert!(i.is_moving());
        let (first, _) = i.step(1.0 / 60.0);
        assert!(
            (first - 9.697).abs() < 0.01,
            "el primer fotograma avanza casi lo que el arrastre: {first}"
        );
        let (second, _) = i.step(1.0 / 60.0);
        assert!(
            (second / first - 0.94).abs() < 1e-3,
            "y pierde un 6 % por fotograma: {second}"
        );
        let mut total = first + second;
        for _ in 0..600 {
            total += i.step(1.0 / 60.0).0;
        }
        assert!(!i.is_moving(), "acaba parando");
        let lambda = -(1.0 - DAMPING).ln();
        assert!(
            (total - 10.0 / lambda).abs() < 0.5,
            "recorre v / λ en total: {total}"
        );
    }

    #[test]
    fn inertia_does_not_depend_on_the_frame_rate() {
        let run = |dt: f32| {
            let mut i = Inertia::default();
            i.track(6.0, -3.0);
            i.release();
            let (mut x, mut y, mut t) = (0.0, 0.0, 0.0);
            while t < 2.0 {
                let (dx, dy) = i.step(dt);
                x += dx;
                y += dy;
                t += dt;
            }
            (x, y)
        };
        let (a, b) = (run(1.0 / 30.0), run(1.0 / 144.0));
        assert!(
            (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05,
            "{a:?} frente a {b:?}"
        );
        assert!(a.0 > 0.0 && a.1 < 0.0, "conserva la dirección");
    }

    #[test]
    fn stop_and_a_standing_release_leave_no_motion() {
        let mut i = Inertia::default();
        i.track(8.0, 8.0);
        i.release();
        i.stop();
        assert_eq!(i.step(1.0 / 60.0), (0.0, 0.0));
        let mut still = Inertia::default();
        still.track(0.0, 0.0);
        still.release();
        assert!(!still.is_moving(), "soltar sin haberse movido no gira nada");
    }
}
