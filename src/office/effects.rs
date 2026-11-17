//! Efectos temporales de la oficina: paquetes de trabajo que vuelan entre agentes, y el oráculo de Jev con sus
//! rayos. Lógica pura (tiempos, curvas, colores); `scene` los convierte en dibujos.
//! Fórmulas de `scene/WorkTransfers.tsx` y `scene/JevOracle.tsx` de la UI web.

use std::collections::HashSet;
use std::f32::consts::PI;

use super::camera::Vec3;
use super::graph::{phase_label, phase_tone, Graph, JevDecision, Tone, WorkEvent};
use super::model::{Desk, DeskState};

pub const TRANSFER_SECONDS: f32 = 2.25;
pub const MAX_TRANSFERS: usize = 8;
pub const JEV_BEAM_SECONDS: f32 = 1.7;
pub const MAX_JEV_BEAMS: usize = 6;
/// Posición del oráculo de Jev sobre la reina.
pub const ORACLE_POSITION: Vec3 = [0.0, 7.6, 0.0];

// ───────────────────────── paquetes de trabajo ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    pub event: WorkEvent,
    /// Tiempo de simulación en que salió.
    pub started: f32,
}

/// Los vuelos en curso. Los eventos que ya existían al abrir son historia y no vuelan.
#[derive(Debug, Default)]
pub struct Transfers {
    seen: HashSet<String>,
    primed: bool,
    pub flights: Vec<Transfer>,
}

impl Transfers {
    /// `calm` es el modo de movimiento: apagado, se anotan los eventos pero nada vuela.
    pub fn update(&mut self, graph: &Graph, now: f32, calm: bool) {
        if !self.primed {
            self.seen
                .extend(graph.work_events.iter().map(|e| e.event_id.clone()));
            self.primed = true;
        } else {
            for e in &graph.work_events {
                if self.seen.insert(e.event_id.clone()) && calm {
                    self.flights.push(Transfer {
                        event: e.clone(),
                        started: now,
                    });
                }
            }
            let extra = self.flights.len().saturating_sub(MAX_TRANSFERS);
            self.flights.drain(..extra);
        }
        self.flights.retain(|f| now - f.started < TRANSFER_SECONDS);
    }
}

/// Avance del vuelo, de 0 a 1 a lo largo de sus 2,25 s.
pub fn progress(t: f32, started: f32) -> f32 {
    ((t - started) / TRANSFER_SECONDS).clamp(0.0, 1.0)
}

/// El paquete espera un instante (14 %) y luego vuela con frenado: (posición, tramo de vuelo 0..1).
pub fn packet_pose(from: Vec3, to: Vec3, progress: f32) -> (Vec3, f32) {
    let flight = ((progress - 0.14) / 0.86).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - flight).powi(3);
    let lerp = |a: f32, b: f32| a + (b - a) * eased;
    (
        [
            lerp(from[0], to[0]),
            lerp(from[1], to[1]) + (eased * PI).sin() * 2.2,
            lerp(from[2], to[2]),
        ],
        flight,
    )
}

/// Los anillos de pulso del origen duran el primer 46 % del vuelo.
pub fn rings_visible(progress: f32) -> bool {
    progress < 0.46
}

/// Escala de los anillos: laten 5,2 veces por vuelo.
pub fn ring_scale(progress: f32) -> f32 {
    0.8 + ((progress * 5.2) % 1.0) * 1.35
}

/// Color del paquete: alerta, entrega o el del agente de destino (o el de origen, o ámbar).
pub fn packet_color(
    phase: &str,
    end_color: Option<[f32; 3]>,
    actor_color: Option<[f32; 3]>,
) -> [f32; 3] {
    match phase_tone(phase) {
        Tone::Alert => [0.9843, 0.4431, 0.5216],
        Tone::Done => [0.5255, 0.9373, 0.6745],
        Tone::Work => end_color
            .or(actor_color)
            .unwrap_or([0.9608, 0.6196, 0.0431]),
    }
}

/// Texto sobre el paquete: quién hace qué a quién.
pub fn transfer_label(phase: &str, src: &str, tgt: &str) -> (String, String) {
    let verb = match phase {
        "delegated" => "llama a",
        "completed" => "entrega a",
        _ => "informa a",
    };
    (
        format!("{src} {verb} {tgt}"),
        phase_label(phase).to_uppercase(),
    )
}

// ─────────────────────────── oráculo de Jev ───────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct JevBeam {
    pub decision: JevDecision,
    pub started: f32,
}

/// Animación del oráculo: el pulso que dispara cada decisión nueva y las rotaciones que de él dependen.
#[derive(Debug, Default)]
pub struct JevFx {
    seen: HashSet<String>,
    primed: bool,
    /// De 1 (decisión recién llegada) a 0, bajando 1,4 por segundo.
    pub pulse: f32,
    /// Giro de la cubierta (x, y), del aro (z) y del núcleo (y).
    pub shell: (f32, f32),
    pub ring: f32,
    pub core: f32,
    pub beams: Vec<JevBeam>,
}

/// Velocidad de giro de la cubierta según el estado y el pulso. Parada con el movimiento apagado.
pub fn oracle_spin(calm: bool, fallback: bool, pulse: f32) -> f32 {
    if !calm {
        return 0.0;
    }
    (if fallback { 0.12 } else { 0.35 }) + pulse * 2.2
}

/// Brillo del núcleo: latido rápido y pobre en fallback, lento y fuerte en normal, más el pulso.
pub fn core_emissive(fallback: bool, t: f32, pulse: f32) -> f32 {
    let base = if fallback {
        0.25 + (9.0 * t).sin().max(0.0) * 0.15
    } else {
        0.7 + (1.6 * t).sin() * 0.12
    };
    base + pulse * 1.6
}

/// Intensidad de la luz del oráculo.
pub fn oracle_light(fallback: bool, pulse: f32) -> f32 {
    (if fallback { 1.2 } else { 3.0 }) + pulse * 9.0
}

impl JevFx {
    /// Un fotograma. Las decisiones que ya existían al abrir no pulsan ni disparan rayos.
    pub fn update(&mut self, graph: &Graph, dt: f32, now: f32, calm: bool) {
        let fallback = graph
            .jev_status
            .as_ref()
            .is_some_and(|j| j.state == "fallback");
        if !self.primed {
            self.seen
                .extend(graph.jev_decisions.iter().map(|d| d.event_id.clone()));
            self.primed = true;
        } else {
            for d in &graph.jev_decisions {
                if self.seen.insert(d.event_id.clone()) {
                    self.pulse = 1.0;
                    // Con el movimiento apagado el oráculo pulsa, pero no hay rayos.
                    if calm {
                        self.beams.push(JevBeam {
                            decision: d.clone(),
                            started: now,
                        });
                    }
                }
            }
            let extra = self.beams.len().saturating_sub(MAX_JEV_BEAMS);
            self.beams.drain(..extra);
        }
        self.beams.retain(|b| now - b.started < JEV_BEAM_SECONDS);

        let spin = oracle_spin(calm, fallback, self.pulse);
        self.shell.1 += dt * spin;
        self.shell.0 += dt * spin * 0.4;
        self.core -= dt * spin * 0.6;
        self.ring += dt * if calm { 0.5 + self.pulse * 3.0 } else { 0.0 };
        self.pulse = (self.pulse - 1.4 * dt).max(0.0);
    }
}

/// Punto de la curva cuadrática del rayo de Jev, con el control a mitad de camino y 2,4 más arriba.
pub fn bezier(start: Vec3, end: Vec3, s: f32) -> Vec3 {
    let mid = [
        (start[0] + end[0]) / 2.0,
        (start[1] + end[1]) / 2.0 + 2.4,
        (start[2] + end[2]) / 2.0,
    ];
    let (a, b, c) = ((1.0 - s) * (1.0 - s), 2.0 * (1.0 - s) * s, s * s);
    [
        a * start[0] + b * mid[0] + c * end[0],
        a * start[1] + b * mid[1] + c * end[1],
        a * start[2] + b * mid[2] + c * end[2],
    ]
}

/// Opacidad del rayo: aparece en el primer 20 %, se mantiene y se apaga desde el 55 %.
pub fn beam_opacity(progress: f32) -> f32 {
    if progress < 0.2 {
        progress / 0.2
    } else {
        1.0 - ((progress - 0.55) / 0.45).max(0.0)
    }
}

/// La chispa recorre la curva en el primer 45 % del rayo.
pub fn spark_param(progress: f32) -> f32 {
    (progress / 0.45).min(1.0)
}

/// Texto del rayo: «Kev · Contexto» según el tipo de decisión.
pub fn jev_beam_label(backend: Option<&str>, kind: &str) -> String {
    let who = if backend == Some("kev") { "Kev" } else { "Jev" };
    let what = match kind {
        "context" => "Contexto",
        "iteration" => "Iteración",
        "parallel" => "Paralelismo",
        other => other,
    };
    format!("{who} · {what}")
}

// ─────────────── partículas, ráfagas y estelas ───────────────

/// Generador pseudoaleatorio mínimo (xorshift32) con semilla fija: la escena es reproducible.
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Rng {
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Un sprite que mira siempre a la cámara: posición, tamaño base en píxeles, constante de atenuación por
/// distancia, color lineal y opacidad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub pos: Vec3,
    pub size: f32,
    pub falloff: f32,
    pub color: [f32; 3],
    pub alpha: f32,
}

pub const FIELD_COUNT: usize = 780;
pub const FIELD_HEIGHT: f32 = 22.0;
pub const FIELD_OPACITY: f32 = 0.16;

/// Una partícula del campo ambiental: posición de origen y semilla.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldParticle {
    pub pos: Vec3,
    pub seed: f32,
}

/// 780 puntos repartidos en un disco de radio 42 y 22 de alto, con semilla fija.
pub fn make_field() -> Vec<FieldParticle> {
    let mut rng = Rng(0x9e3779b9);
    (0..FIELD_COUNT)
        .map(|_| {
            let r = rng.next_f32().sqrt() * 42.0;
            let a = rng.next_f32() * std::f32::consts::TAU;
            let y = rng.next_f32() * FIELD_HEIGHT;
            FieldParticle {
                pos: [r * a.cos(), y, r * a.sin()],
                seed: rng.next_f32(),
            }
        })
        .collect()
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Estado de una partícula del campo en el tiempo `t`: sube, se mece y se desvanece cerca del suelo y del techo.
pub fn field_sprite(p: &FieldParticle, t: f32, color: [f32; 3]) -> Sprite {
    let rise = t * (0.08 + p.seed * 0.12);
    let x = p.pos[0] + (t * 0.25 + p.seed * 40.0).sin() * 1.6;
    let z = p.pos[2] + (t * 0.2 + p.seed * 31.0).cos() * 1.6;
    let y = (p.pos[1] + rise * 6.0).rem_euclid(FIELD_HEIGHT);
    let fade = smooth(0.0, 3.0, y) * smooth(FIELD_HEIGHT, 16.0, y);
    let c = 0.6 + p.seed * 0.8;
    Sprite {
        pos: [x, y, z],
        size: 1.4 + p.seed * 2.4,
        falloff: 28.0,
        color: [color[0] * c, color[1] * c, color[2] * c],
        alpha: FIELD_OPACITY * fade,
    }
}

pub const BURST_PARTICLES: usize = 64;
pub const BURST_SECONDS: f32 = 1.4;
pub const BURST_KEEP: f32 = 1.7;

#[derive(Clone, Debug, PartialEq)]
pub struct Burst {
    pub origin: Vec3,
    pub color: [f32; 3],
    pub started: f32,
    /// Velocidad y semilla de cada partícula.
    pub particles: Vec<(Vec3, f32)>,
}

/// Ráfagas de chispas: una por cada abeja que termina de usar una herramienta.
#[derive(Debug)]
pub struct Bursts {
    pub active: Vec<Burst>,
    prev_tool: std::collections::HashMap<String, bool>,
    rng: Rng,
}

impl Default for Bursts {
    fn default() -> Self {
        Bursts {
            active: Vec::new(),
            prev_tool: Default::default(),
            rng: Rng(0x2545f491),
        }
    }
}

impl Bursts {
    /// `pos_of` da la posición de la abeja de un agente; `enabled` es «movimiento en Enfoque y calidad alta».
    pub fn update(
        &mut self,
        desks: &[Desk],
        now: f32,
        enabled: bool,
        pos_of: &dyn Fn(&str) -> Option<Vec3>,
    ) {
        for d in desks {
            let tool = d.state == DeskState::ToolCall;
            let was = self.prev_tool.insert(d.id.clone(), tool).unwrap_or(false);
            if was && !tool && enabled {
                if let Some(origin) = pos_of(&d.id) {
                    let particles = (0..BURST_PARTICLES)
                        .map(|_| {
                            let theta = self.rng.next_f32() * std::f32::consts::TAU;
                            let phi = (2.0 * self.rng.next_f32() - 1.0).acos();
                            let speed = 0.6 + self.rng.next_f32() * 1.4;
                            let dir = [
                                phi.sin() * theta.cos(),
                                phi.cos() * 0.9,
                                phi.sin() * theta.sin(),
                            ];
                            (
                                [dir[0] * speed, dir[1] * speed, dir[2] * speed],
                                self.rng.next_f32(),
                            )
                        })
                        .collect();
                    self.active.push(Burst {
                        origin,
                        color: d.color,
                        started: now,
                        particles,
                    });
                }
            }
        }
        self.active.retain(|b| now - b.started < BURST_KEEP);
    }
}

/// Las chispas de una ráfaga en el tiempo `now`: salen en abanico, frenan y se apagan en 1,4 s.
pub fn burst_sprites(b: &Burst, now: f32) -> Vec<Sprite> {
    let t = ((now - b.started) / BURST_SECONDS).clamp(0.0, 1.0);
    let life = 1.0 - t;
    let travel = t * 3.2 - t * t * 1.1;
    b.particles
        .iter()
        .map(|(v, seed)| Sprite {
            pos: [
                b.origin[0] + v[0] * travel,
                b.origin[1] + v[1] * travel + t * 1.2,
                b.origin[2] + v[2] * travel,
            ],
            size: (2.2 + seed * 2.0) * life,
            falloff: 30.0,
            color: [
                b.color[0] * (0.8 + life),
                b.color[1] * (0.8 + life),
                b.color[2] * (0.8 + life),
            ],
            alpha: life,
        })
        .collect()
}

pub const TRAIL_SECONDS: f32 = 2.2;
pub const TRAIL_INTERVAL: f32 = 1.0 / 30.0;

/// Estela de una abeja: sus últimas posiciones con la hora de cada una.
#[derive(Clone, Debug, Default)]
pub struct Trail {
    pub points: Vec<(Vec3, f32)>,
}

/// ¿Deja estela? Las ociosas, las desactivadas y las archivadas no.
pub fn leaves_trail(state: DeskState) -> bool {
    matches!(
        state,
        DeskState::Thinking | DeskState::ToolCall | DeskState::Stuck
    )
}

impl Trail {
    pub fn update(&mut self, pos: Vec3, now: f32, active: bool) {
        if !active {
            self.points.clear();
            return;
        }
        // La tolerancia evita que el redondeo de varios fotogramas pequeños deje pasar un fotograma de más.
        if self
            .points
            .last()
            .is_none_or(|(_, t)| now - t >= TRAIL_INTERVAL - 1e-3)
        {
            self.points.push((pos, now));
        }
        self.points.retain(|(_, t)| now - t <= TRAIL_SECONDS);
    }
}

/// Grosor relativo (0..1) de un punto de la estela según su edad: se afina con el cuadrado.
pub fn trail_width(age: f32) -> f32 {
    let w = (1.0 - age / TRAIL_SECONDS).clamp(0.0, 1.0);
    w * w
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn event(g: &mut Graph, id: &str, phase: &str) {
        g.apply(
            "canvas:work_event",
            &json!({"eventId": id, "phase": phase, "actorId": "a", "targetId": "b"}),
            None,
            0.0,
        );
    }

    fn decision(g: &mut Graph, id: &str) {
        g.apply(
            "canvas:jev_decision",
            &json!({"eventId": id, "agentId": "w", "kind": "context"}),
            None,
            0.0,
        );
    }

    // ── paquetes ──

    #[test]
    fn history_does_not_fly_and_new_events_do() {
        let mut t = Transfers::default();
        let mut g = Graph::default();
        event(&mut g, "old", "delegated");
        t.update(&g, 0.0, true);
        assert!(t.flights.is_empty(), "lo que ya existía al abrir no vuela");
        event(&mut g, "new", "delegated");
        t.update(&g, 1.0, true);
        assert_eq!(t.flights.len(), 1);
        assert_eq!(
            (t.flights[0].event.event_id.as_str(), t.flights[0].started),
            ("new", 1.0)
        );
        t.update(&g, 1.5, true);
        assert_eq!(t.flights.len(), 1, "un evento vuela una sola vez");
    }

    #[test]
    fn flights_end_after_two_and_a_quarter_seconds() {
        let mut t = Transfers::default();
        let mut g = Graph::default();
        t.update(&g, 0.0, true);
        event(&mut g, "e", "completed");
        t.update(&g, 1.0, true);
        t.update(&g, 3.2, true);
        assert_eq!(t.flights.len(), 1, "aún queda (2,2 s)");
        t.update(&g, 3.3, true);
        assert!(t.flights.is_empty());
    }

    #[test]
    fn at_most_eight_flights_are_kept_and_motion_off_launches_none() {
        let mut t = Transfers::default();
        let mut g = Graph::default();
        t.update(&g, 0.0, true);
        for i in 0..12 {
            event(&mut g, &format!("e{i}"), "delegated");
        }
        t.update(&g, 0.5, true);
        assert_eq!(t.flights.len(), MAX_TRANSFERS);
        assert_eq!(
            t.flights.last().unwrap().event.event_id,
            "e11",
            "se conservan los últimos"
        );

        let mut quiet = Transfers::default();
        let mut g = Graph::default();
        quiet.update(&g, 0.0, false);
        event(&mut g, "x", "delegated");
        quiet.update(&g, 1.0, false);
        assert!(
            quiet.flights.is_empty(),
            "con el movimiento apagado no vuela nada"
        );
        quiet.update(&g, 2.0, true);
        assert!(
            quiet.flights.is_empty(),
            "y no se recupera después: ya está visto"
        );
    }

    #[test]
    fn the_packet_waits_then_flies_with_an_arc_and_lands() {
        let (from, to) = ([0.0, 4.4, 0.0], [10.0, 4.4, 0.0]);
        let (p0, f0) = packet_pose(from, to, 0.0);
        assert_eq!((p0, f0), (from, 0.0));
        let (p, f) = packet_pose(from, to, 0.14);
        assert_eq!((p, f), (from, 0.0), "espera el primer 14 %");
        // La cima del arco está donde el suavizado vale 0,5: flight = 1 − 0,5^(1/3), o sea el 31,7 % del avance.
        let peak_progress = 0.14 + 0.86 * (1.0 - 0.5f32.powf(1.0 / 3.0));
        let (peak, _) = packet_pose(from, to, peak_progress);
        assert!(
            close(peak[1], 4.4 + 2.2) && close(peak[0], 5.0),
            "la cima sube 2,2 sobre la línea, a mitad de camino: {peak:?}"
        );
        let (late, _) = packet_pose(from, to, 0.57);
        assert!(
            late[1] > 4.4 && late[1] < peak[1],
            "después baja hacia el destino sin tocarlo aún: {}",
            late[1]
        );
        assert!(late[0] > 5.0, "el frenado cúbico lo adelanta: {}", late[0]);
        let (end, fe) = packet_pose(from, to, 1.0);
        assert!(
            close(end[0], 10.0) && close(end[1], 4.4) && close(end[2], 0.0) && close(fe, 1.0),
            "aterriza en el destino: {end:?}"
        );
    }

    #[test]
    fn progress_is_clamped_to_the_flight() {
        assert_eq!(progress(0.0, 1.0), 0.0);
        assert!(close(progress(2.125, 1.0), 0.5));
        assert_eq!(progress(99.0, 1.0), 1.0);
    }

    #[test]
    fn the_pulse_rings_show_early_and_beat_five_times_per_flight() {
        assert!(rings_visible(0.0) && rings_visible(0.45) && !rings_visible(0.46));
        assert!(close(ring_scale(0.0), 0.8));
        assert!(
            ring_scale(0.19) > ring_scale(0.0),
            "crecen dentro de cada latido"
        );
        assert!(
            ring_scale(0.2) < ring_scale(0.19),
            "y se reinician al empezar el siguiente"
        );
        assert!(ring_scale(0.5) <= 0.8 + 1.35);
    }

    #[test]
    fn the_packet_color_follows_the_phase_tone() {
        let (end, actor) = (Some([0.1, 0.2, 0.3]), Some([0.4, 0.5, 0.6]));
        assert_eq!(packet_color("failed", end, actor), [0.9843, 0.4431, 0.5216]);
        assert_eq!(
            packet_color("completed", end, actor),
            [0.5255, 0.9373, 0.6745]
        );
        assert_eq!(
            packet_color("delegated", end, actor),
            [0.1, 0.2, 0.3],
            "el trabajo toma el color del destino"
        );
        assert_eq!(
            packet_color("delegated", None, actor),
            [0.4, 0.5, 0.6],
            "o el del origen"
        );
        assert_eq!(
            packet_color("blocked", None, None),
            [0.9608, 0.6196, 0.0431],
            "o el ámbar"
        );
    }

    #[test]
    fn labels_say_who_does_what_to_whom() {
        assert_eq!(
            transfer_label("delegated", "Pepito", "Buscador"),
            ("Pepito llama a Buscador".into(), "DELEGADO".into())
        );
        assert_eq!(
            transfer_label("completed", "Buscador", "Pepito").0,
            "Buscador entrega a Pepito"
        );
        assert_eq!(
            transfer_label("failed", "A", "B"),
            ("A informa a B".into(), "FALLÓ".into())
        );
    }

    // ── oráculo ──

    #[test]
    fn a_new_decision_pulses_the_oracle_and_fires_a_beam_but_history_does_not() {
        let mut j = JevFx::default();
        let mut g = Graph::default();
        decision(&mut g, "old");
        j.update(&g, 0.016, 0.0, true);
        assert_eq!((j.pulse, j.beams.len()), (0.0, 0), "la historia no pulsa");
        decision(&mut g, "new");
        j.update(&g, 0.016, 1.0, true);
        assert_eq!(j.beams.len(), 1);
        assert!(j.pulse > 0.97, "{}", j.pulse);
        j.update(&g, 0.016, 1.016, true);
        assert_eq!(j.beams.len(), 1, "una decisión dispara un solo rayo");
    }

    #[test]
    fn the_pulse_decays_at_one_point_four_per_second() {
        let mut j = JevFx::default();
        let mut g = Graph::default();
        j.update(&g, 0.0, 0.0, true);
        decision(&mut g, "d");
        j.update(&g, 0.0, 1.0, true);
        assert_eq!(j.pulse, 1.0);
        j.update(&g, 0.5, 1.5, true);
        assert!(close(j.pulse, 1.0 - 0.7), "{}", j.pulse);
        j.update(&g, 5.0, 6.5, true);
        assert_eq!(j.pulse, 0.0);
    }

    #[test]
    fn beams_expire_after_one_point_seven_seconds_and_at_most_six_are_kept() {
        let mut j = JevFx::default();
        let mut g = Graph::default();
        j.update(&g, 0.0, 0.0, true);
        for i in 0..10 {
            decision(&mut g, &format!("d{i}"));
        }
        j.update(&g, 0.016, 1.0, true);
        assert_eq!(j.beams.len(), MAX_JEV_BEAMS);
        j.update(&g, 0.016, 2.8, true);
        assert!(j.beams.is_empty());
    }

    #[test]
    fn motion_off_pulses_without_beams_and_stops_the_spin() {
        let mut j = JevFx::default();
        let mut g = Graph::default();
        j.update(&g, 0.0, 0.0, false);
        decision(&mut g, "d");
        j.update(&g, 0.1, 1.0, false);
        assert_eq!(j.beams.len(), 0);
        assert!(j.pulse > 0.8, "sigue pulsando");
        assert_eq!(
            (j.shell, j.ring, j.core),
            ((0.0, 0.0), 0.0, 0.0),
            "pero no gira"
        );
    }

    #[test]
    fn spin_core_and_light_follow_the_state() {
        assert_eq!(oracle_spin(false, false, 1.0), 0.0);
        assert!(close(oracle_spin(true, false, 0.0), 0.35));
        assert!(close(oracle_spin(true, true, 0.0), 0.12));
        assert!(close(oracle_spin(true, false, 1.0), 2.55));
        assert!(
            core_emissive(false, 0.0, 0.0) > core_emissive(true, 0.0, 0.0),
            "normal más brillante que fallback"
        );
        assert!(close(core_emissive(false, 0.0, 1.0), 0.7 + 1.6));
        assert!(
            core_emissive(true, 0.17, 0.0) > 0.25,
            "el fallback late rápido"
        );
        assert!(close(oracle_light(false, 0.0), 3.0) && close(oracle_light(true, 1.0), 1.2 + 9.0));
    }

    #[test]
    fn the_shell_spins_faster_than_the_core_turns_back() {
        let mut j = JevFx::default();
        let g = Graph::default();
        for _ in 0..60 {
            j.update(&g, 1.0 / 60.0, 0.0, true);
        }
        assert!(
            j.shell.1 > 0.3 && close(j.shell.0, j.shell.1 * 0.4),
            "{:?}",
            j.shell
        );
        assert!(
            j.core < 0.0 && close(j.core, -j.shell.1 * 0.6),
            "el núcleo gira al revés, a un 60 %"
        );
        assert!(j.ring > 0.4);
    }

    #[test]
    fn the_bezier_starts_ends_and_arches_over_the_midpoint() {
        let (a, b) = ([0.0, 7.6, 0.0], [8.0, 3.0, 0.0]);
        assert_eq!(bezier(a, b, 0.0), a);
        assert!(close(bezier(a, b, 1.0)[0], 8.0) && close(bezier(a, b, 1.0)[1], 3.0));
        let m = bezier(a, b, 0.5);
        assert!(close(m[0], 4.0), "{m:?}");
        assert!(m[1] > (7.6 + 3.0) / 2.0, "se arquea hacia arriba: {}", m[1]);
        assert!(close(
            m[1],
            0.25 * 7.6 + 0.5 * ((7.6 + 3.0) / 2.0 + 2.4) + 0.25 * 3.0
        ));
    }

    #[test]
    fn the_beam_fades_in_holds_and_fades_out() {
        assert_eq!(beam_opacity(0.0), 0.0);
        assert!(close(beam_opacity(0.1), 0.5));
        assert_eq!(beam_opacity(0.2), 1.0);
        assert_eq!(beam_opacity(0.55), 1.0);
        assert!(close(beam_opacity(0.775), 0.5));
        assert!(close(beam_opacity(1.0), 0.0));
        assert!(close(spark_param(0.225), 0.5) && spark_param(0.9) == 1.0);
    }

    #[test]
    fn beam_labels_name_the_backend_and_the_kind() {
        assert_eq!(jev_beam_label(Some("jev"), "context"), "Jev · Contexto");
        assert_eq!(jev_beam_label(Some("kev"), "iteration"), "Kev · Iteración");
        assert_eq!(jev_beam_label(None, "parallel"), "Jev · Paralelismo");
        assert_eq!(jev_beam_label(None, "raro"), "Jev · raro");
    }

    // ── partículas, ráfagas y estelas ──

    #[test]
    fn the_generator_is_deterministic_and_stays_in_the_unit_interval() {
        let (mut a, mut b) = (Rng(7), Rng(7));
        for _ in 0..1000 {
            let (x, y) = (a.next_f32(), b.next_f32());
            assert_eq!(x, y);
            assert!((0.0..1.0).contains(&x), "{x}");
        }
        let mut z = Rng(0);
        assert!(
            (0.0..1.0).contains(&z.next_f32()),
            "la semilla cero no se queda atascada"
        );
    }

    #[test]
    fn the_field_has_780_particles_inside_the_disc_and_the_height() {
        let f = make_field();
        assert_eq!(f.len(), FIELD_COUNT);
        assert_eq!(f, make_field(), "semilla fija: mismo campo siempre");
        for p in &f {
            assert!((p.pos[0].powi(2) + p.pos[2].powi(2)).sqrt() <= 42.0 + 1e-3);
            assert!((0.0..FIELD_HEIGHT).contains(&p.pos[1]) && (0.0..1.0).contains(&p.seed));
        }
        let spread = f
            .iter()
            .map(|p| (p.pos[0].powi(2) + p.pos[2].powi(2)).sqrt())
            .sum::<f32>()
            / f.len() as f32;
        assert!(
            spread > 20.0 && spread < 34.0,
            "distribuidas por todo el disco (raíz cuadrada): {spread}"
        );
    }

    #[test]
    fn field_particles_rise_wrap_around_and_fade_at_the_ends() {
        let p = FieldParticle {
            pos: [3.0, 10.0, -4.0],
            seed: 0.5,
        };
        let a = field_sprite(&p, 0.0, [1.0; 3]);
        let b = field_sprite(&p, 5.0, [1.0; 3]);
        assert!(b.pos[1] > a.pos[1], "sube");
        let wrapped = field_sprite(&p, 10_000.0, [1.0; 3]);
        assert!(
            (0.0..FIELD_HEIGHT).contains(&wrapped.pos[1]),
            "da la vuelta al llegar al techo: {}",
            wrapped.pos[1]
        );
        let low = FieldParticle {
            pos: [0.0, 0.0, 0.0],
            seed: 0.0,
        };
        assert_eq!(
            field_sprite(&low, 0.0, [1.0; 3]).alpha,
            0.0,
            "invisible a ras de suelo"
        );
        let mid = FieldParticle {
            pos: [0.0, 10.0, 0.0],
            seed: 0.0,
        };
        assert!(
            (field_sprite(&mid, 0.0, [1.0; 3]).alpha - FIELD_OPACITY).abs() < 1e-6,
            "pleno a media altura"
        );
        let top = FieldParticle {
            pos: [0.0, 21.9, 0.0],
            seed: 0.0,
        };
        assert!(
            field_sprite(&top, 0.0, [1.0; 3]).alpha < 0.002,
            "y se apaga arriba"
        );
        assert!(
            (a.pos[0] - 3.0).abs() <= 1.6 + 1e-3 && (a.pos[2] + 4.0).abs() <= 1.6 + 1e-3,
            "se mece en un radio de 1,6"
        );
    }

    #[test]
    fn field_size_color_and_falloff_follow_the_seed() {
        let lo = field_sprite(
            &FieldParticle {
                pos: [0.0, 10.0, 0.0],
                seed: 0.0,
            },
            0.0,
            [1.0, 0.5, 0.0],
        );
        let hi = field_sprite(
            &FieldParticle {
                pos: [0.0, 10.0, 0.0],
                seed: 1.0,
            },
            0.0,
            [1.0, 0.5, 0.0],
        );
        assert!(close(lo.size, 1.4) && close(hi.size, 3.8) && lo.falloff == 28.0);
        assert!(
            close(lo.color[0], 0.6) && close(hi.color[0], 1.4),
            "brillo de 0,6 a 1,4"
        );
        assert!(close(lo.color[1], 0.3));
    }

    fn desk(id: &str, state: DeskState) -> Desk {
        Desk {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            state,
            current_tool: None,
            current_task: None,
            task_id: None,
            delegated_by: None,
            worker_count: 1,
            color: [0.2, 0.5, 0.9],
        }
    }

    #[test]
    fn a_burst_fires_when_a_tool_call_ends_and_only_then() {
        let mut b = Bursts::default();
        let pos = |_: &str| Some([1.0, 3.0, 2.0]);
        b.update(&[desk("w", DeskState::Idle)], 0.0, true, &pos);
        b.update(&[desk("w", DeskState::ToolCall)], 1.0, true, &pos);
        assert!(
            b.active.is_empty(),
            "empezar una herramienta no dispara nada"
        );
        b.update(&[desk("w", DeskState::ToolCall)], 1.5, true, &pos);
        b.update(&[desk("w", DeskState::Thinking)], 2.0, true, &pos);
        assert_eq!(b.active.len(), 1);
        assert_eq!(
            (
                b.active[0].origin,
                b.active[0].started,
                b.active[0].particles.len()
            ),
            ([1.0, 3.0, 2.0], 2.0, BURST_PARTICLES)
        );
        b.update(&[desk("w", DeskState::Thinking)], 2.5, true, &pos);
        assert_eq!(b.active.len(), 1, "una sola ráfaga por herramienta");
    }

    #[test]
    fn bursts_need_the_effect_enabled_and_a_bee_and_expire_after_one_point_seven_seconds() {
        let mut off = Bursts::default();
        let pos = |_: &str| Some([0.0; 3]);
        off.update(&[desk("w", DeskState::ToolCall)], 0.0, false, &pos);
        off.update(&[desk("w", DeskState::Idle)], 1.0, false, &pos);
        assert!(
            off.active.is_empty(),
            "sin movimiento o en calidad baja no hay ráfagas"
        );
        let mut no_bee = Bursts::default();
        no_bee.update(&[desk("w", DeskState::ToolCall)], 0.0, true, &|_| None);
        no_bee.update(&[desk("w", DeskState::Idle)], 1.0, true, &|_| None);
        assert!(no_bee.active.is_empty(), "sin abeja no hay dónde salir");
        let mut b = Bursts::default();
        b.update(&[desk("w", DeskState::ToolCall)], 0.0, true, &pos);
        b.update(&[desk("w", DeskState::Idle)], 1.0, true, &pos);
        b.update(&[desk("w", DeskState::Idle)], 2.6, true, &pos);
        assert_eq!(b.active.len(), 1, "a los 1,6 s aún dura");
        b.update(&[desk("w", DeskState::Idle)], 2.8, true, &pos);
        assert!(b.active.is_empty());
    }

    #[test]
    fn burst_velocities_cover_the_sphere_with_the_documented_speeds() {
        let mut b = Bursts::default();
        let pos = |_: &str| Some([0.0; 3]);
        b.update(&[desk("w", DeskState::ToolCall)], 0.0, true, &pos);
        b.update(&[desk("w", DeskState::Idle)], 1.0, true, &pos);
        let burst = &b.active[0];
        let speeds: Vec<f32> = burst
            .particles
            .iter()
            .map(|(v, _)| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
            .collect();
        assert!(
            speeds
                .iter()
                .all(|s| *s >= 0.6 * 0.9 - 1e-3 && *s <= 2.0 + 1e-3),
            "velocidades de 0,6 a 2,0"
        );
        assert!(
            burst.particles.iter().any(|(v, _)| v[1] > 0.0)
                && burst.particles.iter().any(|(v, _)| v[1] < 0.0),
            "salen hacia arriba y hacia abajo"
        );
        assert!(
            burst.particles.iter().any(|(v, _)| v[0] > 0.0)
                && burst.particles.iter().any(|(v, _)| v[0] < 0.0)
        );
    }

    #[test]
    fn sparks_fan_out_slow_down_and_fade_over_one_point_four_seconds() {
        let burst = Burst {
            origin: [0.0, 3.0, 0.0],
            color: [1.0, 0.0, 0.0],
            started: 0.0,
            particles: vec![([2.0, 0.0, 0.0], 0.5)],
        };
        let at = |t: f32| burst_sprites(&burst, t)[0];
        let s0 = at(0.0);
        assert_eq!(s0.pos, [0.0, 3.0, 0.0]);
        assert!(close(s0.alpha, 1.0) && close(s0.size, 2.2 + 0.5 * 2.0) && s0.falloff == 30.0);
        let (s1, s2) = (at(0.35), at(0.7));
        assert!(s1.pos[0] > 0.0 && s2.pos[0] > s1.pos[0], "se alejan");
        assert!(s1.pos[1] > 3.0, "y suben un poco (1,2 en total)");
        assert!(
            s2.alpha < s1.alpha && s2.size < s1.size,
            "se apagan y se encogen"
        );
        let end = at(BURST_SECONDS);
        assert!(close(end.alpha, 0.0) && close(end.size, 0.0));
        assert!(
            close(end.pos[0], 2.0 * (3.2 - 1.1)),
            "el recorrido total es 3,2 − 1,1 = 2,1 veces la velocidad: {}",
            end.pos[0]
        );
        assert_eq!(at(10.0).alpha, 0.0, "pasado su tiempo no vuelve");
    }

    #[test]
    fn only_working_bees_leave_a_trail() {
        for (s, yes) in [
            (DeskState::Thinking, true),
            (DeskState::ToolCall, true),
            (DeskState::Stuck, true),
            (DeskState::Idle, false),
            (DeskState::Disabled, false),
            (DeskState::Archived, false),
        ] {
            assert_eq!(leaves_trail(s), yes, "{s:?}");
        }
    }

    #[test]
    fn a_trail_samples_at_thirty_hertz_keeps_two_point_two_seconds_and_clears_when_idle() {
        let mut t = Trail::default();
        for i in 0..300 {
            t.update([i as f32 * 0.01, 3.0, 0.0], i as f32 / 120.0, true); // 120 fotogramas por segundo
        }
        let n = t.points.len();
        assert!(
            n > 60 && n < 72,
            "unos 30 puntos por segundo durante 2,2 s: {n}"
        );
        assert!(
            t.points
                .iter()
                .all(|(_, ts)| 299.0 / 120.0 - ts <= TRAIL_SECONDS + 1e-4),
            "no guarda lo más viejo"
        );
        assert!(
            t.points.windows(2).all(|w| w[1].1 > w[0].1),
            "en orden de tiempo"
        );
        t.update([0.0; 3], 3.0, false);
        assert!(
            t.points.is_empty(),
            "una abeja que se queda quieta pierde la estela"
        );
    }

    #[test]
    fn the_trail_thins_with_the_square_of_its_age() {
        assert_eq!(trail_width(0.0), 1.0);
        assert!(close(trail_width(TRAIL_SECONDS / 2.0), 0.25));
        assert_eq!(trail_width(TRAIL_SECONDS), 0.0);
        assert_eq!(trail_width(99.0), 0.0);
    }
}
