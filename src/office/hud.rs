//! Lógica del HUD de la oficina, sin Rune ni GPU: director de escena (qué evento se muestra ahora), ticker de
//! actividad, etapa de cada delegación, formateadores y selección con el ratón.
//! Reglas de `scene/SceneDirector.tsx`, `hud/*` y `state/presentation.ts` de la UI web.

use std::collections::{HashMap, HashSet, VecDeque};

use super::camera::{project, Camera, Mat4, Vec3};
use super::graph::{
    is_alert_phase, phase_label, phase_tone, Graph, InteractionKind, JevDecision, Tone, WorkEvent,
};
use super::model::{Coordinator, Desk, DeskState};
use super::state::OfficeState;

pub const PRESENTATION_QUEUE_LIMIT: usize = 3;
pub const FEED_KEEP: usize = 24;
pub const FEED_SHOWN: usize = 6;

// ───────────────────────────── formateadores ─────────────────────────────

/// "830" por debajo de mil; si no, "6,1k" con coma decimal (locale es) y un decimal.
pub fn format_token_count(n: f64) -> String {
    let a = n.abs();
    let sign = if n < 0.0 { "-" } else { "" };
    if a < 1000.0 {
        format!("{sign}{}", a.round() as i64)
    } else {
        format!("{sign}{}k", format!("{:.1}", a / 1000.0).replace('.', ","))
    }
}

/// "−6,1k tok" si ahorra, "+6,1k tok" si añade; nada si el cambio es menor de 50 tokens.
pub fn jev_saving_chip(saved: f64) -> Option<String> {
    if saved.abs() < 50.0 {
        return None;
    }
    Some(format!(
        "{}{} tok",
        if saved > 0.0 { "−" } else { "+" },
        format_token_count(saved.abs())
    ))
}

/// "$0", cuatro decimales por debajo de un centavo, tres por encima.
pub fn format_jev_cost(c: f64) -> String {
    if c == 0.0 {
        "$0".into()
    } else if c < 0.01 {
        format!("${c:.4}")
    } else {
        format!("${c:.3}")
    }
}

/// Descripción corta de la lista: 62 caracteres como máximo (59 y «…»).
pub fn short_description(s: &str) -> String {
    if s.chars().count() <= 62 {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(59).collect::<String>())
    }
}

/// Nombre legible de una herramienta: sin guiones bajos ni prefijos de servidor, con mayúscula inicial.
pub fn humanize_tool(name: &str) -> String {
    let base = name.rsplit("__").next().unwrap_or(name);
    let spaced = base.replace(['_', '-'], " ").trim().to_string();
    let mut chars = spaced.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ───────────────────────── director de escena ─────────────────────────

/// Cuánto dura un evento en pantalla: las alertas más, las finalizaciones algo más que lo normal.
pub fn cue_seconds(phase: &str) -> f32 {
    if is_alert_phase(phase) {
        4.2
    } else if phase == "completed" {
        3.2
    } else {
        2.8
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub event: WorkEvent,
    pub until: f32,
}

/// Serializa los eventos «focales»: uno a la vez, con una cola corta y las alertas por delante.
#[derive(Debug, Default)]
pub struct Director {
    seen: HashSet<String>,
    primed: bool,
    queue: VecDeque<WorkEvent>,
    active: Option<Cue>,
    prev_stuck: HashMap<String, bool>,
    blocked_n: u32,
}

impl Director {
    pub fn active(&self) -> Option<&Cue> {
        self.active.as_ref()
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    fn start_next(&mut self, now: f32) {
        if let Some(event) = self.queue.pop_front() {
            let until = now + cue_seconds(&event.phase);
            self.active = Some(Cue { event, until });
        }
    }

    fn enqueue(&mut self, event: WorkEvent, now: f32) {
        let alert = is_alert_phase(&event.phase);
        // Una alerta interrumpe lo que no lo sea: lo activo vuelve a la cola, detrás de ella.
        let preempted = if alert
            && self
                .active
                .as_ref()
                .is_some_and(|c| !is_alert_phase(&c.event.phase))
        {
            self.active.take().map(|c| c.event)
        } else {
            None
        };
        if alert {
            self.queue.push_front(event);
        } else {
            self.queue.push_back(event);
        }
        if let Some(p) = preempted {
            self.queue.insert(1.min(self.queue.len()), p);
        }
        self.queue.truncate(PRESENTATION_QUEUE_LIMIT);
        if self.active.is_none() {
            self.start_next(now);
        }
    }

    /// Un fotograma: toma los eventos nuevos, genera «bloqueado» para quien se atasque y avanza los tiempos.
    /// La primera llamada marca como visto lo que ya existía (historia: no se anima).
    pub fn update(
        &mut self,
        graph: &Graph,
        desks: &[Desk],
        coordinator: Option<&Coordinator>,
        now: f32,
    ) {
        if !self.primed {
            self.seen
                .extend(graph.work_events.iter().map(|e| e.event_id.clone()));
            self.prev_stuck = desks
                .iter()
                .map(|d| (d.id.clone(), d.state == DeskState::Stuck))
                .collect();
            self.primed = true;
        } else {
            for e in &graph.work_events {
                if self.seen.insert(e.event_id.clone()) {
                    self.enqueue(e.clone(), now);
                }
            }
            for d in desks {
                let stuck = d.state == DeskState::Stuck;
                if stuck && self.prev_stuck.get(&d.id) == Some(&false) {
                    self.blocked_n += 1;
                    self.enqueue(
                        WorkEvent {
                            event_id: format!("blocked:{}:{}", d.id, self.blocked_n),
                            phase: "blocked".into(),
                            task_ref: d.task_id.clone().unwrap_or_default(),
                            task_name: d
                                .current_task
                                .clone()
                                .unwrap_or_else(|| "Ejecución del agente".into()),
                            actor_id: d.id.clone(),
                            target_id: coordinator.map(|c| c.id.clone()),
                            tool_name: None,
                            detail: Some("El agente necesita atención".into()),
                            timestamp: 0.0,
                        },
                        now,
                    );
                }
                self.prev_stuck.insert(d.id.clone(), stuck);
            }
        }
        if self.active.as_ref().is_some_and(|c| now >= c.until) {
            self.active = None;
            self.start_next(now);
        }
    }
}

// ───────────────────────────── ticker ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedKind {
    Info,
    Work,
    Warn,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedItem {
    pub text: String,
    pub color: [f32; 3],
    pub kind: FeedKind,
    /// Segundos de simulación en los que ocurrió.
    pub at: f32,
    /// Hora local (HH:MM:SS) en la que se anotó, como en la UI web.
    pub stamp: String,
}

pub const AMBER: [f32; 3] = [0.9608, 0.6196, 0.0431];
pub const ROSE: [f32; 3] = [0.9843, 0.4431, 0.5216];
pub const GREEN: [f32; 3] = [0.5255, 0.9373, 0.6745];
pub const VIOLET: [f32; 3] = [0.6549, 0.5451, 0.9804];
pub const RED: [f32; 3] = [0.9725, 0.4431, 0.4431];

/// Actividad reciente, la más nueva primero.
#[derive(Debug, Default)]
pub struct Feed {
    pub items: VecDeque<FeedItem>,
    seen_events: HashSet<String>,
    seen_jev: HashSet<String>,
    prev: HashMap<String, (DeskState, bool)>,
    primed: bool,
}

impl Feed {
    fn push(&mut self, text: String, color: [f32; 3], kind: FeedKind, at: f32) {
        self.items.push_front(FeedItem {
            text,
            color,
            kind,
            at,
            stamp: chrono::Local::now().format("%H:%M:%S").to_string(),
        });
        self.items.truncate(FEED_KEEP);
    }

    pub fn shown(&self) -> impl Iterator<Item = &FeedItem> {
        self.items.iter().take(FEED_SHOWN)
    }

    pub fn update(
        &mut self,
        graph: &Graph,
        desks: &[Desk],
        coordinator: Option<&Coordinator>,
        now: f32,
    ) {
        let name = |id: &str| -> String {
            desks
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.name.clone())
                .or_else(|| coordinator.filter(|c| c.id == id).map(|c| c.name.clone()))
                .unwrap_or_else(|| id.to_string())
        };
        let color_of = |id: &str| desks.iter().find(|d| d.id == id).map_or(AMBER, |d| d.color);

        if !self.primed {
            // Lo que ya está en curso al abrir aparece una vez; la historia no se anima.
            for i in graph.interactions() {
                self.push(
                    with_task(
                        format!("{} → {}", name(&i.source_id), name(&i.target_id)),
                        i.task_name.as_deref().unwrap_or(""),
                    ),
                    color_of(&i.target_id),
                    FeedKind::Info,
                    now,
                );
            }
            self.seen_events
                .extend(graph.work_events.iter().map(|e| e.event_id.clone()));
            self.seen_jev
                .extend(graph.jev_decisions.iter().map(|d| d.event_id.clone()));
            self.prev = desks
                .iter()
                .map(|d| (d.id.clone(), (d.state, d.current_task.is_some())))
                .collect();
            self.primed = true;
            return;
        }

        for e in &graph.work_events {
            if !self.seen_events.insert(e.event_id.clone()) {
                continue;
            }
            let target = e
                .target_id
                .as_deref()
                .map(&name)
                .unwrap_or_else(|| coordinator.map_or("—".into(), |c| c.name.clone()));
            let (color, kind) = match phase_tone(&e.phase) {
                Tone::Alert => (ROSE, FeedKind::Warn),
                Tone::Done => (GREEN, FeedKind::Done),
                Tone::Work => (
                    e.target_id
                        .as_deref()
                        .map_or_else(|| color_of(&e.actor_id), color_of),
                    FeedKind::Work,
                ),
            };
            self.push(
                with_task(
                    format!(
                        "{} · {} → {}",
                        phase_label(&e.phase),
                        name(&e.actor_id),
                        target
                    ),
                    &e.task_name,
                ),
                color,
                kind,
                now,
            );
        }

        for d in &graph.jev_decisions {
            if !self.seen_jev.insert(d.event_id.clone()) || d.kind != "context" {
                continue;
            }
            self.push(jev_line(d, &name), VIOLET, FeedKind::Info, now);
        }

        for d in desks {
            let had_task = d.current_task.is_some();
            let before = self.prev.insert(d.id.clone(), (d.state, had_task));
            if before.map(|b| b.0) == Some(d.state) {
                continue;
            }
            match (d.state, had_task) {
                (DeskState::ToolCall, false) => {
                    let tool = d
                        .current_tool
                        .as_deref()
                        .map(humanize_tool)
                        .unwrap_or_default();
                    self.push(
                        format!("{} · {}", d.name, tool),
                        d.color,
                        FeedKind::Work,
                        now,
                    );
                }
                (DeskState::Thinking, false) => self.push(
                    format!("{} está pensando…", d.name),
                    d.color,
                    FeedKind::Info,
                    now,
                ),
                (DeskState::Stuck, _) => self.push(
                    format!("{} necesita ayuda", d.name),
                    RED,
                    FeedKind::Warn,
                    now,
                ),
                _ => {}
            }
        }
    }
}

/// Añade `: «tarea»` solo si hay nombre de tarea.
fn with_task(head: String, task: &str) -> String {
    if task.is_empty() {
        head
    } else {
        format!("{head}: «{task}»")
    }
}

fn jev_line(d: &JevDecision, name: &dyn Fn(&str) -> String) -> String {
    let mut s = format!("Jev → {}: {}", name(&d.agent_id), d.summary);
    if let Some(chip) = jev_saving_chip(d.saved_tokens) {
        s.push_str(&format!(" ({chip})"));
    }
    if let Some(r) = &d.recommended_agent_id {
        s.push_str(&format!(" · sugiere {}", name(r)));
    }
    s
}

// ───────────────────────────── delegaciones ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct FlowRow {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: InteractionKind,
    pub task: String,
    /// 0 Delegado, 1 Trabajando, 2 Revisando, 3 Entregado.
    pub stage: u8,
    pub failed: bool,
}

pub const STAGES: [&str; 4] = ["Delegado", "Trabajando", "Revisando", "Entregado"];

/// Una fila por delegación o revisión en curso, con su etapa. El último evento de la tarea manda.
pub fn flow(graph: &Graph, desks: &[Desk], coordinator: Option<&Coordinator>) -> Vec<FlowRow> {
    let name = |id: &str| -> String {
        desks
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .or_else(|| coordinator.filter(|c| c.id == id).map(|c| c.name.clone()))
            .unwrap_or_else(|| id.to_string())
    };
    graph
        .interactions()
        .into_iter()
        .map(|i| {
            let latest = i
                .task_id
                .as_deref()
                .and_then(|t| graph.work_events.iter().rev().find(|e| e.task_ref == t));
            let finished =
                latest.is_some_and(|e| matches!(e.phase.as_str(), "completed" | "review_passed"));
            let working = desks.iter().any(|d| {
                d.id == i.target_id && matches!(d.state, DeskState::Thinking | DeskState::ToolCall)
            });
            let stage = if finished {
                3
            } else if i.kind == InteractionKind::Reviews {
                2
            } else if working {
                1
            } else {
                0
            };
            FlowRow {
                id: i.id.clone(),
                source: name(&i.source_id),
                target: name(&i.target_id),
                kind: i.kind,
                task: i.task_name.clone().unwrap_or_default(),
                stage,
                failed: latest.is_some_and(|e| {
                    matches!(
                        e.phase.as_str(),
                        "failed" | "aborted" | "review_failed" | "blocked"
                    )
                }),
            }
        })
        .collect()
}

// ───────────────────────── selección con el ratón ─────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Nothing,
    Queen,
    Bee(String),
}

/// Radio de captura en puntos alrededor del centro proyectado de cada objeto.
pub const PICK_RADIUS: f32 = 30.0;
const QUEEN_CENTER: Vec3 = [0.0, 3.1, 0.0];

/// Qué hay bajo `point` (puntos, desde la esquina del hueco): lo más cercano a la cámara dentro del radio.
/// Las abejas lejanas son pequeñas, así que su radio baja con la distancia.
pub fn pick(o: &OfficeState, point: (f32, f32), viewport: (f32, f32)) -> Pick {
    if viewport.0 < 1.0 || viewport.1 < 1.0 {
        return Pick::Nothing;
    }
    let vp: Mat4 = o.camera.view_proj(viewport.0 / viewport.1);
    let eye = o.camera.eye();
    let dist = |p: Vec3| {
        ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)).sqrt()
    };
    let mut best: Option<(f32, Pick)> = None;
    let mut consider = |world: Vec3, radius_scale: f32, what: Pick| {
        let Some((x, y)) = project(&vp, world, viewport) else {
            return;
        };
        let d = dist(world);
        // Radio de captura inversamente proporcional a la distancia (referencia: la pose inicial).
        let r = (PICK_RADIUS * radius_scale * (28.0 / d.max(1.0))).clamp(14.0, 60.0);
        let off = ((x - point.0).powi(2) + (y - point.1).powi(2)).sqrt();
        if off <= r && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, what));
        }
    };
    consider(QUEEN_CENTER, 2.0, Pick::Queen);
    for (id, sim) in &o.bees {
        consider(sim.bee.pos, 1.0, Pick::Bee(id.clone()));
    }
    best.map_or(Pick::Nothing, |(_, p)| p)
}

/// Aplica una selección: tocar lo ya seleccionado lo suelta; tocar el vacío también.
pub fn apply_pick(selected: &mut Option<String>, coordinator_id: Option<&str>, pick: &Pick) {
    let id = match pick {
        Pick::Nothing => None,
        Pick::Queen => coordinator_id.map(str::to_string),
        Pick::Bee(id) => Some(id.clone()),
    };
    *selected = match (id, selected.as_ref()) {
        (Some(new), Some(cur)) if &new == cur => None,
        (new, _) => new,
    };
}

/// Posición proyectada (puntos) de una abeja, para anclarle etiquetas. `None` si está detrás de la cámara.
pub fn screen_pos(camera: &Camera, world: Vec3, viewport: (f32, f32)) -> Option<(f32, f32)> {
    if viewport.0 < 1.0 || viewport.1 < 1.0 {
        return None;
    }
    project(&camera.view_proj(viewport.0 / viewport.1), world, viewport)
}

#[cfg(test)]
mod tests;
