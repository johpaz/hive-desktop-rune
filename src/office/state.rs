//! Estado vivo de la oficina: el grafo del lienzo, los agentes, la cámara y la simulación del enjambre.

use std::collections::HashMap;

use super::camera::{Camera, Vec3};
use super::effects::{self, make_field, Bursts, FieldParticle, JevFx, Trail, Transfers};
use super::graph::Graph;
use super::hud::{Director, Feed};
use super::model::{self, Coordinator, Desk, DeskState};
use super::rig::{Inertia, Mode, Rig, CORE_FOCUS};
use super::sound::SoundCues;
use super::swarm::{bee_orbit, Bee, OrbitSpec};
use crate::settings::api::Agent;

/// Cómo se ve una abeja según lo que hace (`FLAP_BY_STATE` de la UI web).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tune {
    /// rad/s
    pub flap: f32,
    pub intensity: f32,
    pub opacity: f32,
}

pub fn tune(state: DeskState) -> Tune {
    let (flap, intensity, opacity) = match state {
        DeskState::Archived => (0.0, 0.08, 0.12),
        DeskState::Idle => (0.0, 0.45, 0.72),
        DeskState::Thinking => (5.0, 0.85, 0.9),
        DeskState::ToolCall => (8.0, 1.05, 1.0),
        DeskState::Stuck => (2.0, 0.9, 0.95),
        DeskState::Disabled => (0.0, 0.12, 0.06),
    };
    Tune {
        flap,
        intensity,
        opacity,
    }
}

/// Valores suavizados que el shader holográfico recibe (se acercan a su objetivo en cada fotograma).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeeVisual {
    pub intensity: f32,
    pub opacity: f32,
    pub wing_opacity: f32,
    pub glitch: f32,
}

impl Default for BeeVisual {
    fn default() -> Self {
        BeeVisual {
            intensity: 0.65,
            opacity: 0.8,
            wing_opacity: 0.4,
            glitch: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BeeSim {
    pub bee: Bee,
    pub spec: OrbitSpec,
    /// Hacia dónde mira (el vuelo, suavizado).
    pub dir: Vec3,
    pub vis: BeeVisual,
    /// Cabeceo (rad) y escala del gesto actual: asiente al recibir una delegación, cabecea al informar.
    pub pitch: f32,
    pub gscale: f32,
}

/// Animación de la reina.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueenSim {
    pub yaw: f32,
    pub scale: f32,
    pub crown: f32,
    pub light: f32,
    /// Interviene en el evento que se presenta ahora.
    pub involved: bool,
}

impl Default for QueenSim {
    fn default() -> Self {
        QueenSim {
            yaw: 0.0,
            scale: 1.0,
            crown: 0.0,
            light: 6.0,
            involved: false,
        }
    }
}

pub struct OfficeState {
    pub graph: Graph,
    pub agents: Vec<Agent>,
    pub camera: Camera,
    pub bees: HashMap<String, BeeSim>,
    pub queen: QueenSim,
    pub t: f32,
    /// Movimiento "enfoque" (true) o "fijo" (false).
    pub calm: bool,
    pub high_quality: bool,
    pub selected: Option<String>,
    /// Último desplazamiento del arrastre (para orbitar con el incremento, no con el total).
    pub drag_last: (f32, f32),
    /// Tamaño en puntos del hueco donde se pinta (lo fija el compositor); sirve para anclar las etiquetas.
    pub viewport: (f32, f32),
    pub director: Director,
    pub feed: Feed,
    pub transfers: Transfers,
    pub jev_fx: JevFx,
    pub rig: Rig,
    /// Partículas ambientales (semilla fija), ráfagas y estelas.
    pub field: Vec<FieldParticle>,
    pub bursts: Bursts,
    pub trails: HashMap<String, Trail>,
    /// Inercia de la órbita y si hay un arrastre en curso.
    pub inertia: Inertia,
    pub dragging: bool,
    /// Sonido de los eventos: apagado por defecto; `pending_sounds` son las fases que acaban de llegar.
    pub sound_on: bool,
    pub sound: SoundCues,
    pub pending_sounds: Vec<String>,
    /// Altavoz de los efectos (aparte del de los clips de A2UI, que se corta al empezar otro).
    pub fx: std::sync::Arc<dyn crate::a2ui::audio::Sink>,
    /// La lista de agentes del HUD, plegada.
    pub roster_collapsed: bool,
    /// En una ventana estrecha el Centro de operaciones arranca plegado; esto lo abre.
    pub roster_open_narrow: bool,
    /// El panel de Jev, plegado.
    pub jev_collapsed: bool,
}

impl Default for OfficeState {
    fn default() -> Self {
        OfficeState {
            graph: Graph::default(),
            agents: Vec::new(),
            camera: Camera::default(),
            bees: HashMap::new(),
            queen: QueenSim::default(),
            t: 0.0,
            calm: true,
            high_quality: true,
            selected: None,
            drag_last: (0.0, 0.0),
            viewport: (0.0, 0.0),
            director: Director::default(),
            feed: Feed::default(),
            transfers: Transfers::default(),
            jev_fx: JevFx::default(),
            rig: Rig::default(),
            field: make_field(),
            bursts: Bursts::default(),
            trails: HashMap::new(),
            inertia: Inertia::default(),
            dragging: false,
            sound_on: false,
            sound: SoundCues::default(),
            pending_sounds: Vec::new(),
            fx: std::sync::Arc::new(crate::a2ui::audio::Speaker::default()),
            roster_collapsed: false,
            roster_open_narrow: false,
            jev_collapsed: false,
        }
    }
}

/// Se acerca a `to` con un suavizado exponencial independiente de la velocidad de fotogramas.
fn approach(v: &mut f32, to: f32, k: f32) {
    *v += (to - *v) * k;
}

fn normalize(v: Vec3) -> Vec3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-9 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

impl OfficeState {
    pub fn desks(&self) -> (Option<Coordinator>, Vec<Desk>) {
        model::derive(&self.agents, &self.graph.nodes)
    }

    /// Avanza la simulación `dt` segundos.
    /// Reproduce los efectos pendientes por el altavoz de efectos.
    pub fn play_pending_sounds(&mut self) {
        for phase in std::mem::take(&mut self.pending_sounds) {
            if let Some(pcm) = super::sound::render(&phase) {
                let _ = self.fx.cue(&pcm);
            }
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.t += dt;
        let (coordinator, desks) = self.desks();
        let count = desks.len();

        // Primero se actualiza lo que se muestra (director, ticker y efectos): así el resto del fotograma ya
        // reacciona al evento nuevo, sin un fotograma de retraso.
        self.director
            .update(&self.graph, &desks, coordinator.as_ref(), self.t);
        self.feed
            .update(&self.graph, &desks, coordinator.as_ref(), self.t);
        self.transfers.update(&self.graph, self.t, self.calm);
        self.jev_fx.update(&self.graph, dt, self.t, self.calm);

        // Ráfagas (al terminar una herramienta) y estelas: solo hace falta la posición de cada abeja.
        let bees = &self.bees;
        self.bursts
            .update(&desks, self.t, self.calm && self.high_quality, &|id| {
                bees.get(id).map(|b| b.bee.pos)
            });
        self.trails
            .retain(|id, _| desks.iter().any(|d| &d.id == id));
        for d in &desks {
            if let Some(b) = self.bees.get(&d.id) {
                self.trails.entry(d.id.clone()).or_default().update(
                    b.bee.pos,
                    self.t,
                    effects::leaves_trail(d.state),
                );
            }
        }

        // El evento que se presenta ahora (si hay): quién actúa y a quién va dirigido.
        let boss_id = coordinator.as_ref().map(|c| c.id.clone());
        let cue = self.director.active().map(|c| {
            (
                c.event.actor_id.clone(),
                c.event
                    .target_id
                    .clone()
                    .or_else(|| boss_id.clone())
                    .unwrap_or_else(|| c.event.actor_id.clone()),
                c.event.phase.clone(),
            )
        });
        let selected = self.selected.clone();

        self.bees.retain(|id, _| desks.iter().any(|d| &d.id == id));
        for (i, desk) in desks.iter().enumerate() {
            let spec = bee_orbit(i, count);
            let sim = self.bees.entry(desk.id.clone()).or_insert_with(|| BeeSim {
                bee: Bee::new(&spec),
                spec,
                dir: [0.0, 0.0, 1.0],
                vis: BeeVisual::default(),
                pitch: 0.0,
                gscale: 1.0,
            });
            sim.spec = spec;
            sim.bee.step(
                &spec,
                desk.state,
                desk.current_task.is_some(),
                dt,
                self.t,
                self.calm,
            );

            let v = [
                sim.bee.pos[0] - sim.bee.prev_pos[0],
                sim.bee.pos[1] - sim.bee.prev_pos[1],
                sim.bee.pos[2] - sim.bee.prev_pos[2],
            ];
            if v[0] * v[0] + v[1] * v[1] + v[2] * v[2] > 1e-8 {
                let k = 1.0 - 0.01f32.powf(dt);
                let target = normalize(v);
                sim.dir = normalize([
                    sim.dir[0] + (target[0] - sim.dir[0]) * k,
                    sim.dir[1] + (target[1] - sim.dir[1]) * k,
                    sim.dir[2] + (target[2] - sim.dir[2]) * k,
                ]);
            }

            // Con un evento en pantalla, quien no interviene (ni está seleccionado) se atenúa.
            let involved = cue
                .as_ref()
                .is_some_and(|(a, t, _)| *a == desk.id || *t == desk.id);
            let emphasis =
                if cue.is_some() && !involved && selected.as_deref() != Some(desk.id.as_str()) {
                    0.28
                } else {
                    1.0
                };
            let tn = tune(desk.state);
            let k = 1.0 - 0.002f32.powf(dt);
            approach(&mut sim.vis.intensity, tn.intensity, k);
            approach(&mut sim.vis.opacity, tn.opacity * emphasis, k);
            approach(&mut sim.vis.wing_opacity, 0.4 * emphasis, k);

            // Gestos (solo con movimiento): asiente al recibir una delegación, cabecea al informar.
            let acknowledging = cue
                .as_ref()
                .is_some_and(|(_, t, phase)| *t == desk.id && phase == "delegated");
            let reporting = cue.as_ref().is_some_and(|(a, _, phase)| {
                *a == desk.id && (phase == "completed" || phase == "failed")
            });
            let pitch_target = if !self.calm {
                0.0
            } else if acknowledging {
                (8.5 * self.t).sin() * 0.22
            } else if reporting {
                -0.16
            } else {
                0.0
            };
            approach(&mut sim.pitch, pitch_target, 1.0 - (-7.0f32 * dt).exp());
            approach(
                &mut sim.gscale,
                if involved && self.calm { 1.12 } else { 1.0 },
                1.0 - (-6.0f32 * dt).exp(),
            );
            approach(
                &mut sim.vis.glitch,
                if desk.state == DeskState::Stuck {
                    1.0
                } else {
                    0.0
                },
                k,
            );
        }

        // Reina: respira y mueve las alas más rápido cuando hay trabajo.
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
        // Si interviene en el evento actual, se vuelve hacia el otro agente, crece un poco y gira más la corona.
        let involved = cue
            .as_ref()
            .is_some_and(|(a, t, _)| Some(a) == boss_id.as_ref() || Some(t) == boss_id.as_ref());
        self.queen.involved = involved;
        let other = cue.as_ref().and_then(|(a, t, _)| {
            if Some(a) == boss_id.as_ref() {
                self.bee_pos(t)
            } else {
                self.bee_pos(a)
            }
        });
        let yaw_target = match (involved, other) {
            (true, Some(p)) => p[0].atan2(p[2]),
            _ if self.calm => (0.38 * self.t).sin() * 0.08,
            _ => 0.0,
        };
        approach(&mut self.queen.yaw, yaw_target, 1.0 - (-4.5f32 * dt).exp());
        approach(
            &mut self.queen.scale,
            if involved && self.calm { 1.08 } else { 1.0 },
            1.0 - (-5.0f32 * dt).exp(),
        );
        if self.calm {
            self.queen.crown += dt * if involved { 1.2 } else { 0.22 };
        }
        // Como en la UI web, no depende de dt: se acerca un 4 % por fotograma.
        let want_light = if active { 12.0 } else { 6.0 };
        self.queen.light += (want_light - self.queen.light) * 0.04;

        // Cámara: intro, enfoque en el evento, regreso y seguimiento de la selección. La reina se mira un poco
        // más arriba (`CORE_FOCUS`) que el punto de donde salen los rayos.
        let focus = |id: &str| -> Option<Vec3> {
            if Some(id) == boss_id.as_deref() {
                Some(CORE_FOCUS)
            } else {
                self.bee_pos(id)
            }
        };
        let cue_points = cue
            .as_ref()
            .and_then(|(a, t, _)| Some((focus(a)?, focus(t)?)));
        let selected_pos = selected.as_deref().and_then(focus);
        self.rig
            .step(&mut self.camera, dt, self.calm, cue_points, selected_pos);

        // Inercia: solo si la persona manda (el director o la intro la cancelan).
        if self.rig.mode == Mode::Free && !self.dragging && self.rig.controls_enabled() {
            let (dx, dy) = self.inertia.step(dt);
            if dx != 0.0 || dy != 0.0 {
                self.camera.orbit(dx, dy);
            }
        } else if !self.dragging {
            self.inertia.stop();
        }

        // Sonido: los eventos nuevos se anotan siempre (para no recibir un aluvión al activarlo) y solo se guardan si está activo.
        let heard = self.sound.update(&self.graph);
        if self.sound_on && self.calm {
            self.pending_sounds.extend(heard);
        }
    }

    /// Posición actual de la abeja de un agente.
    pub fn bee_pos(&self, id: &str) -> Option<Vec3> {
        self.bees.get(id).map(|b| b.bee.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        o
    }

    fn run(o: &mut OfficeState, secs: f32) {
        let mut t = 0.0;
        while t < secs {
            o.step(1.0 / 60.0);
            t += 1.0 / 60.0;
        }
    }

    #[test]
    fn there_is_one_bee_per_worker_and_none_for_the_coordinator() {
        let mut o = office(5);
        o.step(0.016);
        assert_eq!(o.bees.len(), 5);
        assert!(!o.bees.contains_key("boss"));
    }

    #[test]
    fn bees_come_and_go_with_their_agents() {
        let mut o = office(3);
        o.step(0.016);
        o.agents.push(agent("late", "worker"));
        o.step(0.016);
        assert_eq!(o.bees.len(), 4);
        o.agents.retain(|a| a.id != "w0");
        o.step(0.016);
        assert!(!o.bees.contains_key("w0") && o.bees.len() == 3);
    }

    #[test]
    fn a_new_bee_starts_on_its_orbit_with_the_default_visuals() {
        let mut o = office(1);
        o.step(0.0);
        let b = &o.bees["w0"];
        assert_eq!(b.vis, BeeVisual::default());
        assert_eq!(b.bee.radius, bee_orbit(0, 1).radius);
    }

    #[test]
    fn working_bees_gather_on_the_ring_and_light_up() {
        let mut o = office(1);
        o.graph.nodes.push(crate::office::graph::Node {
            id: "w0".into(),
            kind: "agent".into(),
            name: "w0".into(),
            status: "tool_call".into(),
            data: serde_json::json!({"currentTask": "buscar", "currentTool": "web_search"}),
        });
        run(&mut o, 20.0);
        let b = &o.bees["w0"];
        assert!((b.bee.radius - crate::office::swarm::WORK_RING_RADIUS).abs() < 0.05);
        assert!(
            (b.vis.intensity - 1.05).abs() < 0.01 && (b.vis.opacity - 1.0).abs() < 0.01,
            "{:?}",
            b.vis
        );
    }

    #[test]
    fn a_stuck_bee_glitches_and_a_calm_one_does_not() {
        let mut o = office(2);
        o.graph.nodes.push(crate::office::graph::Node {
            id: "w0".into(),
            kind: "agent".into(),
            name: "w0".into(),
            status: "error".into(),
            data: serde_json::json!({}),
        });
        run(&mut o, 3.0);
        assert!(o.bees["w0"].vis.glitch > 0.99);
        assert!(o.bees["w1"].vis.glitch < 0.01);
    }

    #[test]
    fn visuals_converge_without_overshooting_even_with_a_huge_step() {
        let mut o = office(1);
        o.step(0.0);
        o.step(50.0);
        let v = o.bees["w0"].vis;
        assert!(
            (v.intensity - 0.45).abs() < 1e-3 && (v.opacity - 0.72).abs() < 1e-3,
            "{v:?}"
        );
    }

    #[test]
    fn the_heading_follows_the_flight_and_stays_unit_length() {
        let mut o = office(1);
        o.graph.nodes.push(crate::office::graph::Node {
            id: "w0".into(),
            kind: "agent".into(),
            name: "w0".into(),
            status: "thinking".into(),
            data: serde_json::json!({}),
        });
        run(&mut o, 10.0);
        let b = &o.bees["w0"];
        let l = (b.dir[0].powi(2) + b.dir[1].powi(2) + b.dir[2].powi(2)).sqrt();
        assert!((l - 1.0).abs() < 1e-4);
        let v = [
            b.bee.pos[0] - b.bee.prev_pos[0],
            b.bee.pos[1] - b.bee.prev_pos[1],
            b.bee.pos[2] - b.bee.prev_pos[2],
        ];
        let vn = normalize(v);
        assert!(
            b.dir[0] * vn[0] + b.dir[1] * vn[1] + b.dir[2] * vn[2] > 0.9,
            "mira hacia donde vuela"
        );
    }

    #[test]
    fn an_idle_bee_keeps_its_heading() {
        let mut o = office(1);
        run(&mut o, 30.0);
        let before = o.bees["w0"].dir;
        run(&mut o, 5.0);
        let after = o.bees["w0"].dir;
        assert!(
            (before[0] - after[0]).abs()
                + (before[1] - after[1]).abs()
                + (before[2] - after[2]).abs()
                < 0.2
        );
    }

    #[test]
    fn the_queen_light_rises_with_activity_and_the_crown_only_spins_when_calm() {
        let mut o = office(1);
        o.graph.nodes.push(crate::office::graph::Node {
            id: "w0".into(),
            kind: "agent".into(),
            name: "w0".into(),
            status: "thinking".into(),
            data: serde_json::json!({}),
        });
        run(&mut o, 10.0);
        assert!((o.queen.light - 12.0).abs() < 0.1, "{}", o.queen.light);
        let crown = o.queen.crown;
        assert!(crown > 1.0);
        o.calm = false;
        run(&mut o, 5.0);
        assert_eq!(
            o.queen.crown, crown,
            "con el movimiento fijo la corona no gira"
        );
    }

    #[test]
    fn the_tune_table_matches_the_web_values() {
        assert_eq!(
            tune(DeskState::ToolCall),
            Tune {
                flap: 8.0,
                intensity: 1.05,
                opacity: 1.0
            }
        );
        assert_eq!(tune(DeskState::Idle).flap, 0.0);
        assert_eq!(tune(DeskState::Disabled).opacity, 0.06);
        assert_eq!(tune(DeskState::Archived).intensity, 0.08);
    }

    // ── gestos, atenuación y la reina en el evento actual ──

    fn with_cue(phase: &str, actor: &str, target: Option<&str>) -> OfficeState {
        let mut o = office(3);
        run(&mut o, 1.5); // termina la intro de cámara y el director absorbe la historia vacía
        o.graph.apply("canvas:work_event", &serde_json::json!({"eventId": "c1", "phase": phase, "actorId": actor, "targetId": target, "taskName": "T"}), None, 0.0);
        o.step(1.0 / 60.0);
        o
    }

    fn settle(o: &mut OfficeState, secs: f32) {
        // Menos que la duración del evento (2,8 s) para que siga en pantalla.
        run(o, secs);
    }

    #[test]
    fn the_receiving_bee_nods_and_grows_while_the_others_fade() {
        let mut o = with_cue("delegated", "boss", Some("w0"));
        assert!(o.director.active().is_some());
        let mut max_pitch: f32 = 0.0;
        for _ in 0..90 {
            o.step(1.0 / 60.0);
            max_pitch = max_pitch.max(o.bees["w0"].pitch.abs());
        }
        assert!(max_pitch > 0.1, "asiente: {max_pitch}");
        assert!(
            o.bees["w0"].gscale > 1.1,
            "crece un 12 %: {}",
            o.bees["w0"].gscale
        );
        assert!(
            (o.bees["w1"].gscale - 1.0).abs() < 1e-3 && o.bees["w1"].pitch.abs() < 1e-3,
            "las demás no se inmutan"
        );
        assert!(
            o.bees["w1"].vis.opacity < 0.3,
            "las que no intervienen se atenúan: {}",
            o.bees["w1"].vis.opacity
        );
        assert!(
            o.bees["w0"].vis.opacity > 0.6,
            "la que interviene no: {}",
            o.bees["w0"].vis.opacity
        );
    }

    #[test]
    fn the_reporting_bee_dips_its_head() {
        let mut o = with_cue("completed", "w1", Some("boss"));
        settle(&mut o, 1.5);
        assert!(
            (o.bees["w1"].pitch + 0.16).abs() < 0.01,
            "cabecea: {}",
            o.bees["w1"].pitch
        );
        let mut o = with_cue("failed", "w2", None);
        settle(&mut o, 1.5);
        assert!(
            (o.bees["w2"].pitch + 0.16).abs() < 0.01,
            "también al fallar"
        );
    }

    #[test]
    fn everyone_recovers_when_the_event_ends() {
        let mut o = with_cue("delegated", "boss", Some("w0"));
        run(&mut o, 8.0);
        assert!(o.director.active().is_none());
        assert!((o.bees["w0"].gscale - 1.0).abs() < 0.01 && o.bees["w0"].pitch.abs() < 0.01);
        assert!(
            o.bees["w1"].vis.opacity > 0.65,
            "recupera su opacidad: {}",
            o.bees["w1"].vis.opacity
        );
    }

    #[test]
    fn a_selected_bee_is_not_dimmed_during_an_event() {
        let mut o = with_cue("delegated", "boss", Some("w0"));
        o.selected = Some("w2".into());
        settle(&mut o, 1.5);
        assert!(
            o.bees["w2"].vis.opacity > 0.6,
            "{}",
            o.bees["w2"].vis.opacity
        );
        assert!(o.bees["w1"].vis.opacity < 0.3);
    }

    #[test]
    fn motion_off_means_no_gestures_but_the_dimming_stays() {
        let mut o = office(3);
        o.calm = false;
        o.step(1.0 / 60.0);
        o.graph.apply("canvas:work_event", &serde_json::json!({"eventId": "c", "phase": "delegated", "actorId": "boss", "targetId": "w0"}), None, 0.0);
        run(&mut o, 1.5);
        assert!(
            o.bees["w0"].pitch.abs() < 1e-3 && (o.bees["w0"].gscale - 1.0).abs() < 1e-3,
            "sin gestos con el movimiento apagado"
        );
        assert!(
            o.bees["w1"].vis.opacity < 0.3,
            "pero la atenuación es de claridad, no de movimiento"
        );
    }

    #[test]
    fn the_queen_turns_toward_the_other_agent_grows_and_spins_her_crown_faster() {
        let mut o = with_cue("delegated", "boss", Some("w0"));
        assert!(o.queen.involved);
        settle(&mut o, 1.8);
        let p = o.bee_pos("w0").unwrap();
        assert!(
            (o.queen.yaw - p[0].atan2(p[2])).abs() < 0.35,
            "mira hacia la abeja: {} frente a {}",
            o.queen.yaw,
            p[0].atan2(p[2])
        );
        assert!(o.queen.scale > 1.05, "{}", o.queen.scale);
        let crown = o.queen.crown;
        run(&mut o, 0.5);
        assert!(
            o.queen.crown - crown > 0.5,
            "la corona gira a 1,2 rad/s: {}",
            o.queen.crown - crown
        );
        let mut idle = office(3);
        run(&mut idle, 3.0);
        assert!(!idle.queen.involved && (idle.queen.scale - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_event_pulls_the_camera_and_the_selection_is_followed() {
        let mut o = with_cue("delegated", "boss", Some("w0"));
        run(&mut o, 1.2);
        assert_eq!(o.rig.mode, crate::office::rig::Mode::Focus);
        assert!(
            o.camera.eye()[1] > 20.0,
            "la cámara sube para ver el evento: {:?}",
            o.camera.eye()
        );
        let mut o = office(2);
        run(&mut o, 2.0);
        o.selected = Some("w0".into());
        run(&mut o, 6.0);
        assert_eq!(o.rig.mode, crate::office::rig::Mode::Follow);
        let p = o.bee_pos("w0").unwrap();
        let t = o.camera.target;
        assert!(
            ((t[0] - p[0]).powi(2) + (t[1] - p[1]).powi(2) + (t[2] - p[2]).powi(2)).sqrt() < 2.5,
            "mira a la abeja (que sigue orbitando): {t:?} {p:?}"
        );
    }

    #[test]
    fn a_flung_camera_keeps_orbiting_and_the_director_cancels_it() {
        let mut o = office(1);
        run(&mut o, 2.0); // termina la intro
        o.inertia.track(12.0, 0.0);
        o.inertia.release();
        let az = o.camera.azimuth;
        run(&mut o, 0.5);
        assert!(o.camera.azimuth != az, "sigue girando tras soltar");
        let mut o = office(1);
        run(&mut o, 2.0);
        o.dragging = true;
        o.inertia.track(12.0, 0.0);
        o.inertia.release();
        let az = o.camera.azimuth;
        run(&mut o, 0.5);
        assert_eq!(
            o.camera.azimuth, az,
            "con un arrastre en curso no hay inercia"
        );
        let mut o = office(1);
        run(&mut o, 2.0);
        o.inertia.track(12.0, 0.0);
        o.inertia.release();
        o.selected = Some("w0".into()); // el seguimiento toma la cámara
        run(&mut o, 0.3);
        assert!(!o.inertia.is_moving(), "el director cancela la inercia");
    }

    #[test]
    fn a_flung_camera_does_not_turn_during_the_intro() {
        let mut o = office(1);
        o.step(0.0);
        o.inertia.track(12.0, 0.0);
        o.inertia.release();
        o.step(0.2);
        assert!(
            !o.inertia.is_moving(),
            "los controles no responden durante la intro"
        );
    }

    #[test]
    fn sounds_queue_only_when_enabled_and_never_flood_on_enabling() {
        let mut o = office(2);
        run(&mut o, 0.1);
        o.graph.apply(
            "canvas:work_event",
            &serde_json::json!({"eventId": "a", "phase": "delegated", "actorId": "boss"}),
            None,
            0.0,
        );
        run(&mut o, 0.05);
        assert!(o.pending_sounds.is_empty(), "apagado no suena nada");
        o.sound_on = true;
        run(&mut o, 0.05);
        assert!(
            o.pending_sounds.is_empty(),
            "al activarlo no suenan los eventos de antes"
        );
        o.graph.apply(
            "canvas:work_event",
            &serde_json::json!({"eventId": "b", "phase": "completed", "actorId": "boss"}),
            None,
            0.0,
        );
        run(&mut o, 0.05);
        assert_eq!(o.pending_sounds, ["completed"]);
        o.calm = false;
        o.pending_sounds.clear();
        o.graph.apply(
            "canvas:work_event",
            &serde_json::json!({"eventId": "c", "phase": "failed", "actorId": "boss"}),
            None,
            0.0,
        );
        run(&mut o, 0.05);
        assert!(
            o.pending_sounds.is_empty(),
            "con el movimiento apagado tampoco suena"
        );
    }

    #[test]
    fn pending_sounds_play_through_the_effects_sink_without_cutting_it() {
        use std::sync::{Arc, Mutex};
        #[derive(Default)]
        struct Mem(Mutex<Vec<usize>>, Mutex<u32>);
        impl crate::a2ui::audio::Sink for Mem {
            fn play(&self, _: &[u8]) -> Result<(), String> {
                *self.1.lock().unwrap() += 1;
                Ok(())
            }
            fn stop(&self) {}
            fn queued_ms(&self) -> u64 {
                0
            }
            fn cue(&self, pcm: &[u8]) -> Result<(), String> {
                self.0.lock().unwrap().push(pcm.len());
                Ok(())
            }
        }
        let mem = Arc::new(Mem::default());
        let mut o = office(1);
        o.fx = mem.clone();
        o.pending_sounds = vec!["completed".into(), "failed".into(), "desconocida".into()];
        o.play_pending_sounds();
        assert!(o.pending_sounds.is_empty());
        assert_eq!(
            mem.0.lock().unwrap().len(),
            2,
            "las fases sin sonido se ignoran"
        );
        assert_eq!(*mem.1.lock().unwrap(), 0, "nunca interrumpe");
        assert_eq!(
            mem.0.lock().unwrap()[0],
            24000 * 2 * 34 / 100,
            "0,34 s de PCM16 a 24 kHz"
        );
    }
}
