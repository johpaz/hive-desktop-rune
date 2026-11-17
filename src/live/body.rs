//! El cuerpo y la expresión de BIA: la misma lógica que `cuerpo.ts` de la web, sin three ni GPU.
//!
//! El GLB trae 14 huesos; `pose` decide cuánto gira cada uno según el estado (hablar, escuchar, pensar,
//! error) y `BodyState` suaviza esas señales para que los cambios no den saltos. Los giros van en ejes
//! del MODELO (x a la derecha, y arriba, z hacia quien mira) y `model::Bia::skin_matrices` los traduce
//! al marco local de cada hueso. «L» es el lado +x, igual que `blink_L`.

use super::model::Face;
use super::state::Bia;

pub const BONES: [&str; 14] = [
    "spine", "chest", "neck", "head", "tail_1", "tail_2", "shoulder_L", "arm_L", "forearm_L",
    "wing_L", "shoulder_R", "arm_R", "forearm_R", "wing_R",
];
const SPINE: usize = 0;
const CHEST: usize = 1;
const NECK: usize = 2;
const HEAD: usize = 3;
const TAIL_1: usize = 4;
const TAIL_2: usize = 5;
const SHOULDER_L: usize = 6;
const ARM_L: usize = 7;
const FOREARM_L: usize = 8;
const WING_L: usize = 9;
const SHOULDER_R: usize = 10;
const ARM_R: usize = 11;
const FOREARM_R: usize = 12;
const WING_R: usize = 13;

/// Giro (rad) de cada hueso alrededor de x, y, z del modelo.
pub type Pose = [[f32; 3]; 14];

/// Ruido suave en [-1, 1]: suma de senos con razones irracionales (no entra en bucle).
pub fn noise(t: f32, seed: f32) -> f32 {
    let s = seed * 12.9898;
    (t * 0.73 + s).sin() * 0.5 + (t * 1.31 + s * 1.7).sin() * 0.3 + (t * 2.17 + s * 2.3).sin() * 0.2
}

/// Respiración: se inhala más rápido de lo que se exhala (≈ 40/60).
pub fn breathing(t: f32) -> f32 {
    let period = 4.2;
    let phase = (t % period) / period;
    let rise = 0.4;
    if phase < rise {
        0.5 - 0.5 * (std::f32::consts::PI * phase / rise).cos()
    } else {
        0.5 + 0.5 * (std::f32::consts::PI * (phase - rise) / (1.0 - rise)).cos()
    }
}

/// Señales ya suavizadas que mueven el cuerpo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyInput {
    pub t: f32,
    /// Voz de BIA, 0–1.
    pub voice: f32,
    /// Atención a quien habla (escuchando), 0–1.
    pub mic: f32,
    /// Golpe de voz: sube de golpe con el énfasis y cae solo.
    pub onset: f32,
    /// Pensando (la colmena trabaja), 0–1.
    pub think: f32,
    /// Error, 0–1.
    pub alert: f32,
}

pub fn pose(b: &BodyInput) -> Pose {
    let BodyInput { t, voice, mic, onset, think, alert } = *b;
    let mut p: Pose = Default::default();
    let breathe = breathing(t) - 0.5;

    // Columna y pecho: respiración y un vaivén lento; el pecho se adelanta con la voz.
    p[SPINE][0] = noise(t * 0.5, 10.0) * 0.012;
    p[SPINE][2] = noise(t * 0.4, 11.0) * 0.014;
    p[CHEST][0] = -breathe * 0.035 + voice * 0.018 - alert * 0.04;
    p[CHEST][1] = noise(t * 0.35, 12.0) * 0.02;
    p[SHOULDER_L][2] = breathe * 0.03;
    p[SHOULDER_R][2] = -breathe * 0.03;

    // Cabeza: cabeceo al hablar, se vuelve hacia quien habla y al pensar se ladea y mira de lado.
    let nod = noise(t * 1.3, 4.0) * 0.03 * (0.4 + voice * 3.0)
        + noise(t * 2.7, 5.0) * 0.01 * voice
        + onset * 0.05
        - think * 0.05
        - alert * 0.06;
    let turn = mic * 0.1 + think * (0.16 + noise(t * 0.4, 3.0) * 0.03) + noise(t * 0.3, 7.0) * 0.02;
    let tilt = noise(t * 0.8, 6.0) * 0.015 + think * 0.07;
    p[NECK] = [nod * 0.4, turn * 0.4, tilt * 0.4];
    p[HEAD] = [nod * 0.6, turn * 0.6, tilt * 0.6];

    // Brazos: sin morphs de manos en esta versión, el ademán sale de la voz (cada brazo a su ritmo).
    for (s, arm, fore, seed, think_arm) in [
        (1.0, ARM_L, FOREARM_L, 20.0, 1.0),
        (-1.0, ARM_R, FOREARM_R, 21.0, 0.0),
    ] {
        let gesture = voice * (0.5 + 0.5 * noise(t * 0.8, seed)).clamp(0.0, 1.0);
        let drift = noise(t * 0.55, seed) * 0.04;
        p[arm][2] = s * (gesture * 0.22 + drift);
        p[arm][1] = -s * (gesture * 0.16 + think * 0.12 * think_arm);
        p[fore][2] = s * gesture * 0.32;
        p[fore][1] = -s * think * 0.35 * think_arm;
    }

    // Alas: aleteo suave siempre, más ancho trabajando y abiertas ante un error.
    let flap = (t * 2.1).sin() * (0.035 + think * 0.03 + voice * 0.02) + alert * 0.18;
    p[WING_L][2] = flap;
    p[WING_R][2] = -flap;
    p[WING_L][1] = noise(t * 0.6, 30.0) * 0.03;
    p[WING_R][1] = -noise(t * 0.6, 31.0) * 0.03;

    // Cola: se mece con retardo creciente hacia la punta, como un péndulo.
    let sway = 0.05 + voice * 0.03 + think * 0.02;
    p[TAIL_1][2] = (t * 0.9).sin() * sway + noise(t * 0.3, 40.0) * 0.02;
    p[TAIL_1][0] = noise(t * 0.5, 41.0) * 0.03;
    p[TAIL_2][2] = (t * 0.9 - 0.9).sin() * sway * 1.7;
    p[TAIL_2][0] = noise(t * 0.5 - 0.8, 42.0) * 0.05;
    p
}

/// Gestos de expresión (cejas, mejillas, ceño) que se suman a la boca y al parpadeo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Expression {
    pub brow_up: f32,
    pub brow_down_l: f32,
    pub brow_down_r: f32,
    pub brow_inner_up: f32,
    pub cheek_l: f32,
    pub cheek_r: f32,
    pub frown: f32,
}

/// La cara de cada estado: escuchar abre las cejas, hablar las acompaña con la voz, pensar frunce una más
/// que la otra (duda, no enfado) y el error junta el ceño con las comisuras abajo.
fn target(bia: Bia, think: f32, onset: f32) -> Expression {
    let mut e = Expression::default();
    match bia {
        Bia::Listening => {
            e.brow_up = 0.3;
            e.brow_inner_up = 0.15;
            e.cheek_l = 0.1;
            e.cheek_r = 0.1;
        }
        Bia::Speaking => {
            e.brow_up = 0.12 + onset * 0.3;
            e.cheek_l = 0.15;
            e.cheek_r = 0.15;
        }
        Bia::Error => {
            e.brow_inner_up = 0.7;
            e.brow_down_l = 0.3;
            e.brow_down_r = 0.3;
            e.frown = 0.6;
        }
        Bia::Idle | Bia::Connecting | Bia::Working => {}
    }
    if think > 0.0 {
        e.brow_down_l = e.brow_down_l.max(0.45 * think);
        e.brow_down_r = e.brow_down_r.max(0.12 * think);
        e.brow_inner_up = e.brow_inner_up.max(0.2 * think);
    }
    e
}

fn damp(x: f32, to: f32, lambda: f32, dt: f32) -> f32 {
    x + (to - x) * (1.0 - (-lambda * dt).exp())
}

/// Las señales suavizadas del cuerpo y la expresión, que avanzan cada fotograma.
#[derive(Clone, Copy, Debug, Default)]
pub struct BodyState {
    input: BodyInput,
    expr: Expression,
    prev_level: f32,
}

impl BodyState {
    pub fn step(&mut self, dt: f32, t: f32, bia: Bia, level: f32) {
        let i = &mut self.input;
        i.t = t;
        i.voice = damp(i.voice, level, 12.0, dt);
        let listening = if bia == Bia::Listening { 0.3 } else { 0.0 };
        i.mic = damp(i.mic, listening, 10.0, dt);
        i.think = damp(i.think, if bia == Bia::Working { 1.0 } else { 0.0 }, 3.0, dt);
        i.alert = damp(i.alert, if bia == Bia::Error { 1.0 } else { 0.0 }, 4.0, dt);
        // El golpe de voz: el flanco de subida del nivel, que luego cae solo.
        let rise = (level - self.prev_level).max(0.0);
        i.onset = (i.onset * 0.86f32.powf(dt * 60.0)).max(rise * 4.0).min(1.0);
        self.prev_level = level;

        let want = target(bia, i.think, i.onset);
        let e = &mut self.expr;
        let brow = |x: f32, to: f32| damp(x, to, 8.0, dt);
        let slow = |x: f32, to: f32| damp(x, to, 4.0, dt);
        e.brow_up = brow(e.brow_up, want.brow_up);
        e.brow_down_l = slow(e.brow_down_l, want.brow_down_l);
        e.brow_down_r = slow(e.brow_down_r, want.brow_down_r);
        e.brow_inner_up = slow(e.brow_inner_up, want.brow_inner_up);
        e.cheek_l = slow(e.cheek_l, want.cheek_l);
        e.cheek_r = slow(e.cheek_r, want.cheek_r);
        e.frown = slow(e.frown, want.frown);
    }

    pub fn pose(&self) -> Pose {
        pose(&self.input)
    }

    /// `face` con la expresión de este fotograma sumada.
    pub fn with_expression(&self, face: Face) -> Face {
        let e = &self.expr;
        Face {
            brow_up: e.brow_up,
            brow_down_l: e.brow_down_l,
            brow_down_r: e.brow_down_r,
            brow_inner_up: e.brow_inner_up,
            cheek_l: e.cheek_l,
            cheek_r: e.cheek_r,
            frown: e.frown,
            ..face
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bone_gets_a_finite_turn() {
        let p = pose(&BodyInput { t: 3.0, voice: 1.0, mic: 1.0, onset: 1.0, think: 1.0, alert: 1.0 });
        assert!(p.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn thinking_tilts_and_turns_the_head_but_rest_barely_does() {
        let rest = pose(&BodyInput { t: 3.0, ..Default::default() })[HEAD];
        let think = pose(&BodyInput { t: 3.0, think: 1.0, ..Default::default() })[HEAD];
        assert!(think[1].abs() > rest[1].abs() + 0.05);
        assert!(think[2] > rest[2] + 0.02);
    }

    #[test]
    fn the_wings_open_in_opposite_directions() {
        let p = pose(&BodyInput { t: 0.7, alert: 1.0, ..Default::default() });
        assert!(p[WING_L][2].abs() > 0.05);
        assert_eq!(p[WING_L][2].signum(), -p[WING_R][2].signum());
    }

    #[test]
    fn speaking_moves_the_forearms_and_silence_does_not() {
        let quiet = pose(&BodyInput { t: 2.0, ..Default::default() });
        let talking = pose(&BodyInput { t: 2.0, voice: 1.0, ..Default::default() });
        let moved = (0..3).any(|k| (talking[FOREARM_L][k] - quiet[FOREARM_L][k]).abs() > 1e-3)
            || (0..3).any(|k| (talking[FOREARM_R][k] - quiet[FOREARM_R][k]).abs() > 1e-3);
        assert!(moved);
    }

    #[test]
    fn expressions_follow_the_state_and_ease_in() {
        let mut s = BodyState::default();
        for i in 0..120 {
            s.step(1.0 / 60.0, i as f32 / 60.0, Bia::Listening, 0.0);
        }
        let f = s.with_expression(Face::default());
        assert!(f.brow_up > 0.25 && f.frown < 0.01, "escuchando: cejas arriba");
        for i in 0..240 {
            s.step(1.0 / 60.0, 2.0 + i as f32 / 60.0, Bia::Error, 0.0);
        }
        let f = s.with_expression(Face::default());
        assert!(f.frown > 0.5 && f.brow_inner_up > 0.6, "error: ceño y comisuras");
    }

    #[test]
    fn the_voice_onset_pulses_and_decays() {
        let mut s = BodyState::default();
        s.step(1.0 / 60.0, 0.0, Bia::Speaking, 0.4);
        let peak = s.input.onset;
        assert!(peak > 0.5);
        for i in 0..60 {
            s.step(1.0 / 60.0, i as f32 / 60.0, Bia::Speaking, 0.4);
        }
        assert!(s.input.onset < peak * 0.1);
    }
}
