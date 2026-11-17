//! Estado en vivo del lienzo: grafo de nodos y aristas, eventos de trabajo y decisiones de Jev.
//! Mensajes `canvas:*` del WebSocket, según `stores/canvasStore.ts` de la UI web.

use std::collections::HashMap;

use serde_json::Value;

const MAX_WORK_EVENTS: usize = 64;
const MAX_JEV_DECISIONS: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub status: String,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkEvent {
    pub event_id: String,
    pub phase: String,
    pub task_ref: String,
    pub task_name: String,
    pub actor_id: String,
    pub target_id: Option<String>,
    pub tool_name: Option<String>,
    pub detail: Option<String>,
    /// Milisegundos desde la época.
    pub timestamp: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JevDecision {
    pub event_id: String,
    pub agent_id: String,
    pub kind: String,
    pub summary: String,
    pub saved_tokens: f64,
    pub latency_ms: f64,
    pub cost_usd: f64,
    pub recommended_agent_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct JevTotals {
    pub decisions: f64,
    pub saved_tokens: f64,
    pub cost_usd: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JevStatus {
    /// `off`, `ready` o `fallback`.
    pub state: String,
    pub backend: Option<String>,
    pub totals: Option<JevTotals>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionKind {
    Delegates,
    Reviews,
}

/// Una delegación o revisión en curso (arista del grafo).
#[derive(Clone, Debug, PartialEq)]
pub struct Interaction {
    pub id: String,
    pub kind: InteractionKind,
    pub source_id: String,
    pub target_id: String,
    pub task_id: Option<String>,
    pub task_name: Option<String>,
}

/// Cómo se pinta una fase de un evento de trabajo. `blocked` cuenta como trabajo, no como alerta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Alert,
    Done,
    Work,
}

pub fn phase_tone(phase: &str) -> Tone {
    match phase {
        "failed" | "aborted" | "review_failed" => Tone::Alert,
        "completed" | "review_passed" => Tone::Done,
        _ => Tone::Work,
    }
}

/// Fases que el director de escena trata como alerta (interrumpen y duran más).
pub fn is_alert_phase(phase: &str) -> bool {
    matches!(phase, "failed" | "aborted" | "review_failed" | "blocked")
}

pub fn phase_label(phase: &str) -> &'static str {
    match phase {
        "delegated" => "Delegado",
        "review_passed" => "Revisión aprobada",
        "review_failed" => "Revisión rechazada",
        "completed" => "Completado",
        "failed" => "Falló",
        "aborted" => "Abortado",
        "blocked" => "Bloqueado",
        _ => "Actividad",
    }
}

#[derive(Debug, Default)]
pub struct Graph {
    revision: u64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub work_events: Vec<WorkEvent>,
    pub jev_decisions: Vec<JevDecision>,
    pub jev_status: Option<JevStatus>,
    pub jev_by_agent: HashMap<String, JevTotals>,
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn node_from(v: &Value) -> Option<Node> {
    let id = s(v, "id")?;
    let data = v
        .get("data")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let status = data
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| s(v, "status"))
        .unwrap_or_else(|| "pending".into());
    let name = s(v, "label")
        .or_else(|| s(v, "name"))
        .unwrap_or_else(|| id.clone());
    Some(Node {
        id,
        kind: s(v, "type").unwrap_or_default(),
        name,
        status,
        data,
    })
}

fn edge_from(v: &Value) -> Option<Edge> {
    Some(Edge {
        id: s(v, "id")?,
        source: s(v, "source")?,
        target: s(v, "target")?,
        edge_type: s(v, "edgeType").unwrap_or_default(),
        data: v
            .get("data")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default())),
    })
}

impl Graph {
    /// Aplica un mensaje `canvas:*`. `msg_timestamp` es el del mensaje (si lo trae) y `now` la hora actual
    /// en milisegundos. Devuelve si algo cambió.
    pub fn apply(
        &mut self,
        kind: &str,
        data: &Value,
        msg_timestamp: Option<f64>,
        now: f64,
    ) -> bool {
        match kind {
            "canvas:snapshot" => {
                if let Some(revision) = data["revision"].as_u64() {
                    if revision < self.revision {
                        return false;
                    }
                    self.revision = revision;
                }
                self.nodes = data
                    .get("nodes")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(node_from).collect())
                    .unwrap_or_default();
                self.edges = data
                    .get("edges")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(edge_from).collect())
                    .unwrap_or_default();
                true
            }
            "canvas:node_add" => match node_from(data) {
                Some(n) => {
                    match self.nodes.iter().position(|x| x.id == n.id) {
                        Some(i) => self.nodes[i] = n,
                        None => self.nodes.push(n),
                    }
                    true
                }
                None => false,
            },
            "canvas:node_update" => {
                let Some(id) = s(data, "id").or_else(|| s(data, "nodeId")) else {
                    return false;
                };
                let Some(node) = self.nodes.iter_mut().find(|n| n.id == id) else {
                    return false;
                };
                let patch = data.get("data").or_else(|| data.get("changes"));
                if let (Some(Value::Object(patch)), Value::Object(target)) = (patch, &mut node.data)
                {
                    for (k, v) in patch {
                        target.insert(k.clone(), v.clone());
                    }
                }
                if let Some(status) = patch
                    .and_then(|p| p.get("status"))
                    .and_then(Value::as_str)
                    .or_else(|| data.get("status").and_then(Value::as_str))
                {
                    node.status = status.to_string();
                }
                true
            }
            "canvas:node_remove" => {
                let Some(id) = s(data, "id") else {
                    return false;
                };
                let before = self.nodes.len();
                self.nodes.retain(|n| n.id != id);
                self.nodes.len() != before
            }
            "canvas:edge_add" => match edge_from(data) {
                Some(e) if !self.edges.iter().any(|x| x.id == e.id) => {
                    self.edges.push(e);
                    true
                }
                _ => false,
            },
            "canvas:edge_remove" => {
                let Some(id) = s(data, "id") else {
                    return false;
                };
                let before = self.edges.len();
                self.edges.retain(|e| e.id != id);
                self.edges.len() != before
            }
            "canvas:work_event" => {
                if data["revision"]
                    .as_u64()
                    .is_some_and(|revision| revision < self.revision)
                {
                    return false;
                }
                let (Some(event_id), Some(phase), Some(actor_id)) =
                    (s(data, "eventId"), s(data, "phase"), s(data, "actorId"))
                else {
                    return false;
                };
                if self.work_events.iter().any(|e| e.event_id == event_id) {
                    return false;
                }
                self.work_events.push(WorkEvent {
                    event_id,
                    phase,
                    task_ref: s(data, "taskRef").unwrap_or_default(),
                    task_name: s(data, "taskName").unwrap_or_default(),
                    actor_id,
                    target_id: s(data, "targetId"),
                    tool_name: s(data, "toolName"),
                    detail: s(data, "detail"),
                    timestamp: msg_timestamp.unwrap_or(now),
                });
                let extra = self.work_events.len().saturating_sub(MAX_WORK_EVENTS);
                self.work_events.drain(..extra);
                true
            }
            "canvas:jev_decision" => {
                let (Some(event_id), Some(agent_id)) = (s(data, "eventId"), s(data, "agentId"))
                else {
                    return false;
                };
                if self.jev_decisions.iter().any(|d| d.event_id == event_id) {
                    return false;
                }
                let d = JevDecision {
                    event_id,
                    agent_id: agent_id.clone(),
                    kind: s(data, "kind").unwrap_or_default(),
                    summary: s(data, "summary").unwrap_or_default(),
                    saved_tokens: num(data, "savedTokens"),
                    latency_ms: num(data, "latencyMs"),
                    cost_usd: num(data, "costUsd"),
                    recommended_agent_id: s(data, "recommendedAgentId"),
                };
                let t = self.jev_by_agent.entry(agent_id).or_default();
                t.decisions += 1.0;
                t.saved_tokens += d.saved_tokens;
                t.cost_usd += d.cost_usd;
                self.jev_decisions.push(d);
                let extra = self.jev_decisions.len().saturating_sub(MAX_JEV_DECISIONS);
                self.jev_decisions.drain(..extra);
                // Una decisión implica que Jev está activo; los totales vienen en el mensaje.
                let totals = data.get("totals").map(|t| JevTotals {
                    decisions: num(t, "decisions"),
                    saved_tokens: num(t, "savedTokens"),
                    cost_usd: num(t, "costUsd"),
                });
                self.jev_status = Some(JevStatus {
                    state: "ready".into(),
                    backend: self.jev_status.as_ref().and_then(|j| j.backend.clone()),
                    totals,
                });
                true
            }
            "canvas:jev_status" => {
                let totals = data.get("totals").map(|t| JevTotals {
                    decisions: num(t, "decisions"),
                    saved_tokens: num(t, "savedTokens"),
                    cost_usd: num(t, "costUsd"),
                });
                self.jev_status = Some(JevStatus {
                    state: s(data, "state").unwrap_or_else(|| "off".into()),
                    backend: s(data, "backend"),
                    totals,
                });
                true
            }
            _ => false,
        }
    }

    /// Delegaciones y revisiones: las aristas `delegates` y `reviews`.
    pub fn interactions(&self) -> Vec<Interaction> {
        self.edges
            .iter()
            .filter_map(|e| {
                let kind = match e.edge_type.as_str() {
                    "delegates" => InteractionKind::Delegates,
                    "reviews" => InteractionKind::Reviews,
                    _ => return None,
                };
                Some(Interaction {
                    id: e.id.clone(),
                    kind,
                    source_id: e.source.clone(),
                    target_id: e.target.clone(),
                    task_id: s(&e.data, "taskId"),
                    task_name: s(&e.data, "taskName"),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apply(g: &mut Graph, kind: &str, d: Value) -> bool {
        g.apply(kind, &d, None, 1000.0)
    }

    #[test]
    fn a_snapshot_replaces_everything() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "old", "type": "agent"}),
        );
        apply(
            &mut g,
            "canvas:snapshot",
            json!({"nodes": [{"id": "a", "type": "agent", "label": "Ana", "status": "thinking"}], "edges": [{"id": "e", "source": "a", "target": "b", "edgeType": "delegates"}]}),
        );
        assert_eq!((g.nodes.len(), g.edges.len()), (1, 1));
        assert_eq!(
            (g.nodes[0].name.as_str(), g.nodes[0].status.as_str()),
            ("Ana", "thinking")
        );
    }

    #[test]
    fn node_add_defaults_the_name_and_status() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "n1", "type": "agent"}),
        );
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "n2", "type": "agent", "name": "Dos", "status": "idle", "data": {"status": "tool_call"}}),
        );
        assert_eq!(
            (g.nodes[0].name.as_str(), g.nodes[0].status.as_str()),
            ("n1", "pending")
        );
        assert_eq!(
            (g.nodes[1].name.as_str(), g.nodes[1].status.as_str()),
            ("Dos", "tool_call"),
            "data.status gana a status"
        );
    }

    #[test]
    fn adding_a_node_twice_replaces_it() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "n", "type": "agent", "label": "A"}),
        );
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "n", "type": "agent", "label": "B"}),
        );
        assert_eq!((g.nodes.len(), g.nodes[0].name.as_str()), (1, "B"));
    }

    #[test]
    fn node_update_merges_data_and_sets_the_status() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "n", "type": "agent", "data": {"currentTask": "x", "keep": 1}}),
        );
        assert!(apply(
            &mut g,
            "canvas:node_update",
            json!({"nodeId": "n", "changes": {"status": "tool_call", "currentTool": "web"}})
        ));
        assert_eq!(g.nodes[0].status, "tool_call");
        assert_eq!(
            (
                g.nodes[0].data["currentTool"].as_str(),
                g.nodes[0].data["currentTask"].as_str(),
                g.nodes[0].data["keep"].as_i64()
            ),
            (Some("web"), Some("x"), Some(1))
        );
        assert!(
            !apply(
                &mut g,
                "canvas:node_update",
                json!({"id": "ghost", "data": {}})
            ),
            "un nodo desconocido se ignora"
        );
    }

    #[test]
    fn nodes_and_edges_can_be_removed_and_edges_dedupe() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:node_add",
            json!({"id": "n", "type": "agent"}),
        );
        let edge = json!({"id": "e", "source": "a", "target": "b", "edgeType": "reviews"});
        assert!(apply(&mut g, "canvas:edge_add", edge.clone()));
        assert!(
            !apply(&mut g, "canvas:edge_add", edge),
            "misma id: no se duplica"
        );
        assert!(apply(&mut g, "canvas:edge_remove", json!({"id": "e"})) && g.edges.is_empty());
        assert!(apply(&mut g, "canvas:node_remove", json!({"id": "n"})) && g.nodes.is_empty());
        assert!(!apply(&mut g, "canvas:node_remove", json!({"id": "n"})));
    }

    #[test]
    fn only_delegates_and_reviews_are_interactions() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:edge_add",
            json!({"id": "1", "source": "a", "target": "b", "edgeType": "delegates", "data": {"taskId": "t", "taskName": "Buscar"}}),
        );
        apply(
            &mut g,
            "canvas:edge_add",
            json!({"id": "2", "source": "a", "target": "c", "edgeType": "reviews"}),
        );
        apply(
            &mut g,
            "canvas:edge_add",
            json!({"id": "3", "source": "a", "target": "d", "edgeType": "depends"}),
        );
        let i = g.interactions();
        assert_eq!(i.len(), 2);
        assert_eq!(
            (
                i[0].kind,
                i[0].task_id.as_deref(),
                i[0].task_name.as_deref()
            ),
            (InteractionKind::Delegates, Some("t"), Some("Buscar"))
        );
        assert_eq!(i[1].kind, InteractionKind::Reviews);
    }

    #[test]
    fn work_events_dedupe_use_the_message_timestamp_and_are_capped() {
        let mut g = Graph::default();
        let ev = |id: &str| json!({"eventId": id, "phase": "delegated", "taskRef": "t", "taskName": "T", "actorId": "a", "targetId": null});
        assert!(g.apply("canvas:work_event", &ev("e1"), Some(5.0), 1000.0));
        assert!(
            !g.apply("canvas:work_event", &ev("e1"), None, 1000.0),
            "misma eventId"
        );
        assert!(g.apply("canvas:work_event", &ev("e2"), None, 1000.0));
        assert_eq!(
            (g.work_events[0].timestamp, g.work_events[1].timestamp),
            (5.0, 1000.0)
        );
        assert_eq!(g.work_events[0].target_id, None);
        for i in 0..100 {
            g.apply("canvas:work_event", &ev(&format!("x{i}")), None, 0.0);
        }
        assert_eq!(g.work_events.len(), MAX_WORK_EVENTS);
        assert_eq!(
            g.work_events.last().unwrap().event_id,
            "x99",
            "se conservan los últimos"
        );
    }

    #[test]
    fn incomplete_work_events_are_ignored() {
        let mut g = Graph::default();
        assert!(!apply(
            &mut g,
            "canvas:work_event",
            json!({"phase": "delegated", "actorId": "a"})
        ));
        assert!(!apply(
            &mut g,
            "canvas:work_event",
            json!({"eventId": "e", "actorId": "a"})
        ));
    }

    #[test]
    fn jev_decisions_accumulate_per_agent_and_mark_jev_ready() {
        let mut g = Graph::default();
        let d = |id: &str, saved: f64| json!({"eventId": id, "agentId": "w", "kind": "context", "summary": "s", "savedTokens": saved, "latencyMs": 40, "costUsd": 0.002, "totals": {"decisions": 7, "savedTokens": 900, "costUsd": 0.05}});
        assert!(apply(&mut g, "canvas:jev_decision", d("d1", 100.0)));
        assert!(apply(&mut g, "canvas:jev_decision", d("d2", 250.0)));
        assert!(
            !apply(&mut g, "canvas:jev_decision", d("d1", 100.0)),
            "repetida"
        );
        let t = &g.jev_by_agent["w"];
        assert_eq!((t.decisions, t.saved_tokens), (2.0, 350.0));
        assert!((t.cost_usd - 0.004).abs() < 1e-9);
        let st = g.jev_status.as_ref().unwrap();
        assert_eq!(
            (st.state.as_str(), st.totals.as_ref().unwrap().decisions),
            ("ready", 7.0)
        );
    }

    #[test]
    fn jev_decisions_are_capped() {
        let mut g = Graph::default();
        for i in 0..40 {
            g.apply(
                "canvas:jev_decision",
                &json!({"eventId": format!("d{i}"), "agentId": "w"}),
                None,
                0.0,
            );
        }
        assert_eq!(g.jev_decisions.len(), MAX_JEV_DECISIONS);
    }

    #[test]
    fn jev_status_is_stored_and_a_decision_keeps_its_backend() {
        let mut g = Graph::default();
        apply(
            &mut g,
            "canvas:jev_status",
            json!({"state": "fallback", "backend": "kev"}),
        );
        assert_eq!(g.jev_status.as_ref().unwrap().state, "fallback");
        apply(
            &mut g,
            "canvas:jev_decision",
            json!({"eventId": "d", "agentId": "w"}),
        );
        let st = g.jev_status.as_ref().unwrap();
        assert_eq!(
            (st.state.as_str(), st.backend.as_deref()),
            ("ready", Some("kev"))
        );
    }

    #[test]
    fn unknown_messages_change_nothing() {
        let mut g = Graph::default();
        assert!(!apply(&mut g, "canvas:ping", json!({})));
        assert!(!apply(&mut g, "a2ui:createSurface", json!({})));
    }

    #[test]
    fn tones_match_the_web_ones_and_blocked_is_work_but_an_alert_for_the_director() {
        assert_eq!(phase_tone("failed"), Tone::Alert);
        assert_eq!(phase_tone("review_failed"), Tone::Alert);
        assert_eq!(phase_tone("completed"), Tone::Done);
        assert_eq!(phase_tone("review_passed"), Tone::Done);
        assert_eq!(phase_tone("delegated"), Tone::Work);
        assert_eq!(phase_tone("blocked"), Tone::Work);
        assert!(
            is_alert_phase("blocked") && is_alert_phase("aborted") && !is_alert_phase("completed")
        );
        assert_eq!(phase_label("review_failed"), "Revisión rechazada");
    }
}
