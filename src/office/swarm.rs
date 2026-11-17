//! Cinemática del enjambre: cada abeja orbita en una de tres capas inclinadas alrededor del núcleo.
//! No hay dirección ni azar: todo sale del índice de la abeja y se mezcla con exponenciales.
//! Fórmulas de `scene/swarm.ts` y `scene/SwarmDriver.tsx` de la UI web.

use super::camera::Vec3;
use super::model::DeskState;

pub const WORK_RING_RADIUS: f32 = 4.2;
pub const WORK_RING_Y: f32 = 4.4;
pub const DOCK_RADIUS: f32 = 20.0;
pub const DOCK_Y: f32 = 1.4;
/// (radio, y base, inclinación)
pub const SHELLS: [(f32, f32, f32); 3] = [(7.5, 2.8, 0.55), (10.5, 3.8, 0.9), (13.5, 4.8, 1.25)];

fn golden() -> f32 {
    std::f32::consts::PI * (3.0 - 5.0f32.sqrt())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitSpec {
    pub radius: f32,
    pub y_base: f32,
    pub inclination: f32,
    pub phase: f32,
    /// rad/s
    pub angular_speed: f32,
}

/// Órbita de la abeja `index` de `count`. El índice es su posición entre los trabajadores.
pub fn bee_orbit(index: usize, count: usize) -> OrbitSpec {
    let (radius, y_base, inclination) = SHELLS[index % 3];
    let slot = index / 3;
    let per_shell = count.div_ceil(3).max(1);
    OrbitSpec {
        radius,
        y_base,
        inclination,
        phase: slot as f32 * std::f32::consts::TAU / per_shell as f32
            + (index % 3) as f32 * golden() * 0.37,
        angular_speed: 0.05 + ((index * 7) % 5) as f32 * 0.016,
    }
}

pub fn orbit_position(spec: &OrbitSpec, radius: f32, y: f32, angle: f32) -> Vec3 {
    [
        angle.cos() * radius,
        y + angle.sin() * spec.inclination,
        angle.sin() * radius,
    ]
}

/// Cuánto de su velocidad angular conserva según lo que hace. Una abeja ociosa se queda quieta.
pub fn speed_factor(state: DeskState, has_task: bool) -> f32 {
    match state {
        DeskState::Disabled | DeskState::Archived | DeskState::Idle => 0.0,
        DeskState::ToolCall => 0.65,
        DeskState::Thinking => {
            if has_task {
                0.42
            } else {
                0.2
            }
        }
        DeskState::Stuck => 0.08,
    }
}

/// (radio, y) hacia los que se mueve: aparcada, en el anillo de trabajo o en su capa de origen.
pub fn target_radius_y(state: DeskState, has_task: bool, spec: &OrbitSpec) -> (f32, f32) {
    match state {
        DeskState::Disabled | DeskState::Archived => (DOCK_RADIUS, DOCK_Y),
        _ if has_task => (WORK_RING_RADIUS, WORK_RING_Y),
        _ => (spec.radius, spec.y_base),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bee {
    pub pos: Vec3,
    pub prev_pos: Vec3,
    pub angle: f32,
    pub radius: f32,
    pub y: f32,
    pub speed_mul: f32,
}

impl Bee {
    /// Nace en su fase, sobre su capa, con velocidad completa.
    pub fn new(spec: &OrbitSpec) -> Bee {
        let pos = orbit_position(spec, spec.radius, spec.y_base, spec.phase);
        Bee {
            pos,
            prev_pos: pos,
            angle: spec.phase,
            radius: spec.radius,
            y: spec.y_base,
            speed_mul: 1.0,
        }
    }

    /// Un fotograma. `t` es el tiempo total (segundos); `calm` es el modo de movimiento (apagado = quieta).
    pub fn step(
        &mut self,
        spec: &OrbitSpec,
        state: DeskState,
        has_task: bool,
        dt: f32,
        t: f32,
        calm: bool,
    ) {
        let k = 1.0 - 0.12f32.powf(dt);
        let (want_radius, want_y) = target_radius_y(state, has_task, spec);
        let want_speed = if calm {
            speed_factor(state, has_task)
        } else {
            0.0
        };
        self.radius += (want_radius - self.radius) * k;
        self.y += (want_y - self.y) * k;
        self.speed_mul += (want_speed - self.speed_mul) * k;
        self.angle += spec.angular_speed * self.speed_mul * dt;
        self.prev_pos = self.pos;
        self.pos = orbit_position(spec, self.radius, self.y, self.angle);
        if calm && state == DeskState::Stuck {
            self.pos[0] += (31.0 * t + 7.0 * spec.phase).sin() * 0.22;
            self.pos[1] += (41.0 * t + 3.0 * spec.phase).cos() * 0.15;
            self.pos[2] += (37.0 * t + 5.0 * spec.phase).sin() * 0.22;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: Vec3, b: Vec3) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    fn run(bee: &mut Bee, spec: &OrbitSpec, state: DeskState, task: bool, secs: f32) {
        let mut t = 0.0;
        while t < secs {
            bee.step(spec, state, task, 1.0 / 60.0, t, true);
            t += 1.0 / 60.0;
        }
    }

    #[test]
    fn bees_are_spread_over_the_three_shells_by_index() {
        let shells: Vec<f32> = (0..7).map(|i| bee_orbit(i, 7).radius).collect();
        assert_eq!(shells, [7.5, 10.5, 13.5, 7.5, 10.5, 13.5, 7.5]);
    }

    #[test]
    fn slots_on_a_shell_are_evenly_spaced_in_phase() {
        let count = 9; // 3 por capa
        let phases: Vec<f32> = [0usize, 3, 6]
            .iter()
            .map(|i| bee_orbit(*i, count).phase)
            .collect();
        let step = std::f32::consts::TAU / 3.0;
        assert!(
            (phases[1] - phases[0] - step).abs() < 1e-5
                && (phases[2] - phases[1] - step).abs() < 1e-5
        );
    }

    #[test]
    fn angular_speeds_follow_the_index_pattern() {
        let speeds: Vec<f32> = (0..6).map(|i| bee_orbit(i, 6).angular_speed).collect();
        let expected = [
            0.05,
            0.05 + 0.016 * 2.0,
            0.05 + 0.016 * 4.0,
            0.05 + 0.016 * 1.0,
            0.05 + 0.016 * 3.0,
            0.05,
        ];
        for (a, b) in speeds.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-6, "{a} {b}");
        }
    }

    #[test]
    fn a_single_bee_has_a_valid_orbit() {
        let o = bee_orbit(0, 1);
        assert_eq!((o.radius, o.phase), (7.5, 0.0));
    }

    #[test]
    fn the_position_lies_on_the_inclined_shell() {
        let o = bee_orbit(1, 3);
        let p = orbit_position(&o, o.radius, o.y_base, 1.0);
        assert!(((p[0] * p[0] + p[2] * p[2]).sqrt() - o.radius).abs() < 1e-4);
        assert!((p[1] - (o.y_base + 1.0f32.sin() * o.inclination)).abs() < 1e-5);
    }

    #[test]
    fn an_idle_bee_hovers_still() {
        let spec = bee_orbit(0, 3);
        let mut bee = Bee::new(&spec);
        run(&mut bee, &spec, DeskState::Idle, false, 12.0);
        let (a, b) = (bee.pos, bee.prev_pos);
        run(&mut bee, &spec, DeskState::Idle, false, 2.0);
        assert!(
            dist(a, bee.pos) < 0.05 && dist(a, b) < 0.1,
            "casi inmóvil: {}",
            dist(a, bee.pos)
        );
    }

    #[test]
    fn a_bee_with_a_task_gathers_on_the_work_ring() {
        let spec = bee_orbit(2, 3);
        let mut bee = Bee::new(&spec);
        run(&mut bee, &spec, DeskState::Thinking, true, 30.0);
        assert!(
            (bee.radius - WORK_RING_RADIUS).abs() < 0.01 && (bee.y - WORK_RING_Y).abs() < 0.01,
            "{} {}",
            bee.radius,
            bee.y
        );
    }

    #[test]
    fn a_disabled_bee_docks_far_away_and_low() {
        let spec = bee_orbit(0, 3);
        let mut bee = Bee::new(&spec);
        run(&mut bee, &spec, DeskState::Disabled, false, 30.0);
        assert!((bee.radius - DOCK_RADIUS).abs() < 0.01 && (bee.y - DOCK_Y).abs() < 0.01);
    }

    #[test]
    fn a_working_bee_that_finishes_returns_to_its_shell() {
        let spec = bee_orbit(1, 3);
        let mut bee = Bee::new(&spec);
        run(&mut bee, &spec, DeskState::ToolCall, true, 20.0);
        run(&mut bee, &spec, DeskState::Idle, false, 40.0);
        assert!((bee.radius - spec.radius).abs() < 0.01 && (bee.y - spec.y_base).abs() < 0.01);
    }

    #[test]
    fn motion_off_freezes_the_swarm_but_not_its_destination() {
        let spec = bee_orbit(0, 3);
        let mut bee = Bee::new(&spec);
        for i in 0..600 {
            bee.step(
                &spec,
                DeskState::ToolCall,
                true,
                1.0 / 60.0,
                i as f32 / 60.0,
                false,
            );
        }
        assert!(bee.speed_mul < 0.01, "no orbita con el movimiento apagado");
        assert!(
            (bee.radius - WORK_RING_RADIUS).abs() < 0.05,
            "sí se reubica"
        );
    }

    #[test]
    fn a_stuck_bee_jitters_within_a_small_radius_and_is_deterministic() {
        let spec = bee_orbit(0, 3);
        let (mut a, mut b) = (Bee::new(&spec), Bee::new(&spec));
        let mut max_off: f32 = 0.0;
        for i in 0..300 {
            let t = i as f32 / 60.0;
            a.step(&spec, DeskState::Stuck, false, 1.0 / 60.0, t, true);
            b.step(&spec, DeskState::Stuck, false, 1.0 / 60.0, t, true);
            let clean = orbit_position(&spec, a.radius, a.y, a.angle);
            max_off = max_off.max(dist(clean, a.pos));
        }
        assert_eq!(a, b, "sin azar: misma entrada, mismo resultado");
        assert!(max_off > 0.05 && max_off < 0.5, "tiembla poco: {max_off}");
    }

    #[test]
    fn speed_factors_match_the_table() {
        assert_eq!(speed_factor(DeskState::ToolCall, false), 0.65);
        assert_eq!(speed_factor(DeskState::Thinking, true), 0.42);
        assert_eq!(speed_factor(DeskState::Thinking, false), 0.2);
        assert_eq!(speed_factor(DeskState::Stuck, true), 0.08);
        assert_eq!(speed_factor(DeskState::Idle, true), 0.0);
        assert_eq!(speed_factor(DeskState::Archived, false), 0.0);
    }

    #[test]
    fn a_big_step_reaches_the_target_without_overshooting() {
        let spec = bee_orbit(0, 3);
        let mut bee = Bee::new(&spec);
        bee.step(&spec, DeskState::Thinking, true, 100.0, 0.0, true);
        assert!(
            bee.radius >= WORK_RING_RADIUS - 1e-3 && bee.radius <= spec.radius,
            "{}",
            bee.radius
        );
    }
}
