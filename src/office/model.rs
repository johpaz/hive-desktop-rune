//! Modelo de la oficina: estados de escritorio, color por agente y derivación desde la lista de agentes
//! y los nodos vivos del grafo (`state/useOfficeModel.ts` de la UI web).

use super::graph::Node;
use crate::settings::api::Agent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskState {
    Archived,
    Disabled,
    Idle,
    Thinking,
    ToolCall,
    Stuck,
}

impl DeskState {
    /// `error` y `failed` cuentan como atascado; lo desconocido, como ocioso.
    pub fn from_status(s: &str) -> DeskState {
        match s {
            "tool_call" => DeskState::ToolCall,
            "thinking" => DeskState::Thinking,
            "stuck" | "error" | "failed" => DeskState::Stuck,
            _ => DeskState::Idle,
        }
    }

    /// Con varios nodos vivos para un mismo agente manda el de mayor prioridad.
    fn priority(self) -> u8 {
        match self {
            DeskState::ToolCall => 3,
            DeskState::Thinking | DeskState::Stuck => 2,
            DeskState::Idle => 1,
            DeskState::Archived | DeskState::Disabled => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DeskState::Archived => "Archivado",
            DeskState::Disabled => "Desactivado",
            DeskState::Idle => "Disponible",
            DeskState::Thinking => "Pensando",
            DeskState::ToolCall => "Ejecutando",
            DeskState::Stuck => "Necesita ayuda",
        }
    }
}

/// `h = h*31 + unidad UTF-16`, en 32 bits sin signo.
pub fn hash(id: &str) -> u32 {
    id.encode_utf16()
        .fold(0u32, |h, u| h.wrapping_mul(31).wrapping_add(u as u32))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// `hsl(hash(id) % 360, 68%, 58%)` como RGB sRGB en 0..1.
pub fn agent_color(id: &str) -> [f32; 3] {
    hsl_to_rgb((hash(id) % 360) as f32, 0.68, 0.58)
}

/// Color `#rrggbb` a RGB sRGB en 0..1.
pub fn hex(rgb: u32) -> [f32; 3] {
    [
        ((rgb >> 16) & 255) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Desk {
    pub id: String,
    pub name: String,
    pub description: String,
    pub state: DeskState,
    pub current_tool: Option<String>,
    pub current_task: Option<String>,
    pub task_id: Option<String>,
    pub delegated_by: Option<String>,
    pub worker_count: usize,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Coordinator {
    pub id: String,
    pub name: String,
    pub description: String,
    pub state: DeskState,
}

fn data_str(node: &Node, key: &str) -> Option<String> {
    node.data
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Escritorios (trabajadores) y coordinador a partir de los agentes y los nodos vivos.
pub fn derive(agents: &[Agent], nodes: &[Node]) -> (Option<Coordinator>, Vec<Desk>) {
    let live = |id: &str| {
        nodes
            .iter()
            .filter(|n| n.kind == "agent" && n.id == id)
            .collect::<Vec<_>>()
    };
    let best = |list: &[&Node]| -> Option<(DeskState, usize)> {
        let mut top: Option<(usize, DeskState)> = None;
        for (i, n) in list.iter().enumerate() {
            let s = DeskState::from_status(&n.status);
            if top.is_none_or(|(_, t)| s.priority() > t.priority()) {
                top = Some((i, s));
            }
        }
        top.map(|(i, s)| (s, i))
    };

    // Un solo coordinador; todo lo demás es especialista y tiene su abeja, sea cual sea el `role` que traiga.
    let core = crate::settings::api::core_id(agents);
    let coordinator = agents
        .iter()
        .find(|a| Some(a.id.as_str()) == core)
        .map(|a| {
            let nodes = live(&a.id);
            let state = best(&nodes).map_or_else(|| DeskState::from_status(&a.status), |(s, _)| s);
            Coordinator {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone(),
                state,
            }
        });

    let desks = agents
        .iter()
        .filter(|a| Some(a.id.as_str()) != core)
        .map(|a| {
            let nodes = live(&a.id);
            let top = best(&nodes);
            let state = if a.status == "archived" {
                DeskState::Archived
            } else if !a.enabled {
                DeskState::Disabled
            } else {
                top.map_or(DeskState::Idle, |(s, _)| s)
            };
            let node = top.map(|(_, i)| nodes[i]);
            Desk {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone(),
                state,
                current_tool: node.and_then(|n| data_str(n, "currentTool")),
                current_task: node.and_then(|n| data_str(n, "currentTask")),
                task_id: node.and_then(|n| data_str(n, "taskId")),
                delegated_by: node.and_then(|n| data_str(n, "delegatedBy")),
                worker_count: nodes.len(),
                color: agent_color(&a.id),
            }
        })
        .collect();
    (coordinator, desks)
}

/// Categoría visual de una herramienta, probada en este orden contra su nombre en minúsculas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Knowledge,
    Browser,
    Code,
    Communication,
    Generic,
}

pub fn tool_kind(name: &str) -> ToolKind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["knowledge", "memory", "rag", "embed", "document", "vector"]) {
        ToolKind::Knowledge
    } else if has(&["browser", "web", "search", "fetch", "http", "url"]) {
        ToolKind::Browser
    } else if has(&[
        "file", "code", "exec", "shell", "terminal", "git", "patch", "write", "read",
    ]) {
        ToolKind::Code
    } else if has(&[
        "message", "mail", "slack", "discord", "telegram", "whatsapp", "notify",
    ]) {
        ToolKind::Communication
    } else {
        ToolKind::Generic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(id: &str, role: &str, status: &str, enabled: bool) -> Agent {
        Agent {
            id: id.into(),
            name: id.to_uppercase(),
            role: role.into(),
            status: status.into(),
            enabled,
            ..Default::default()
        }
    }

    fn node(id: &str, status: &str, data: serde_json::Value) -> Node {
        Node {
            id: id.into(),
            kind: "agent".into(),
            name: id.into(),
            status: status.into(),
            data,
        }
    }

    #[test]
    fn the_hash_matches_the_web_one() {
        assert_eq!(hash(""), 0);
        assert_eq!(hash("a"), 97);
        assert_eq!(hash("ab"), 97 * 31 + 98);
        // desborda en 32 bits sin signo, como `>>> 0`
        assert_eq!(
            hash("zzzzzzzzzzzzzzzz"),
            "zzzzzzzzzzzzzzzz"
                .chars()
                .fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32))
        );
    }

    #[test]
    fn the_color_is_stable_per_id_and_in_range() {
        let a = agent_color("agente-1");
        assert_eq!(a, agent_color("agente-1"));
        assert_ne!(a, agent_color("agente-2"));
        assert!(a.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn hsl_conversion_hits_known_colors() {
        let close =
            |a: [f32; 3], b: [f32; 3]| a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 0.01);
        assert!(close(hsl_to_rgb(0.0, 1.0, 0.5), [1.0, 0.0, 0.0]));
        assert!(close(hsl_to_rgb(120.0, 1.0, 0.5), [0.0, 1.0, 0.0]));
        assert!(close(hsl_to_rgb(240.0, 1.0, 0.5), [0.0, 0.0, 1.0]));
        assert!(close(hsl_to_rgb(0.0, 0.0, 0.5), [0.5, 0.5, 0.5]));
    }

    #[test]
    fn hex_unpacks_rgb() {
        assert_eq!(hex(0xff0000), [1.0, 0.0, 0.0]);
        let c = hex(0xf59e0b);
        assert!((c[0] - 0.9608).abs() < 1e-3 && (c[2] - 0.0431).abs() < 1e-3);
    }

    #[test]
    fn statuses_normalize() {
        assert_eq!(DeskState::from_status("error"), DeskState::Stuck);
        assert_eq!(DeskState::from_status("failed"), DeskState::Stuck);
        assert_eq!(DeskState::from_status("tool_call"), DeskState::ToolCall);
        assert_eq!(DeskState::from_status("thinking"), DeskState::Thinking);
        assert_eq!(DeskState::from_status("pending"), DeskState::Idle);
        assert_eq!(DeskState::from_status(""), DeskState::Idle);
    }

    #[test]
    fn there_is_one_coordinator_and_every_other_agent_gets_a_desk_whatever_its_role() {
        let agents = [
            agent("boss", "coordinator", "idle", true),
            agent("w1", "worker", "idle", true),
            agent("falso", "coordinator", "idle", true),
        ];
        let (c, desks) = derive(&agents, &[]);
        assert_eq!(
            c.map(|c| c.id),
            Some("boss".to_string()),
            "el primero marcado manda"
        );
        let ids: Vec<&str> = desks.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            ["w1", "falso"],
            "el otro «coordinador» es un especialista más, con su abeja"
        );
    }

    #[test]
    fn only_workers_become_desks_and_the_coordinator_is_apart() {
        let agents = [
            agent("boss", "coordinator", "idle", true),
            agent("w1", "worker", "idle", true),
            agent("w2", "worker", "idle", true),
        ];
        let (c, d) = derive(&agents, &[]);
        assert_eq!(c.unwrap().id, "boss");
        assert_eq!(
            d.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            ["w1", "w2"]
        );
    }

    #[test]
    fn archived_and_disabled_win_over_live_activity() {
        let agents = [
            agent("a", "worker", "archived", true),
            agent("b", "worker", "idle", false),
            agent("c", "worker", "idle", true),
        ];
        let nodes = [
            node("a", "tool_call", json!({})),
            node("b", "thinking", json!({})),
            node("c", "thinking", json!({})),
        ];
        let (_, d) = derive(&agents, &nodes);
        assert_eq!(
            (d[0].state, d[1].state, d[2].state),
            (
                DeskState::Archived,
                DeskState::Disabled,
                DeskState::Thinking
            )
        );
    }

    #[test]
    fn the_highest_priority_node_gives_the_state_and_the_fields() {
        let agents = [agent("w", "worker", "idle", true)];
        let nodes = [
            node("w", "idle", json!({"currentTask": "vieja"})),
            node(
                "w",
                "tool_call",
                json!({"currentTool": "web_search", "currentTask": "buscar", "taskId": "t1", "delegatedBy": "boss"}),
            ),
            node("w", "thinking", json!({"currentTask": "otra"})),
        ];
        let (_, d) = derive(&agents, &nodes);
        assert_eq!(d[0].state, DeskState::ToolCall);
        assert_eq!(d[0].current_tool.as_deref(), Some("web_search"));
        assert_eq!(
            (
                d[0].current_task.as_deref(),
                d[0].task_id.as_deref(),
                d[0].delegated_by.as_deref()
            ),
            (Some("buscar"), Some("t1"), Some("boss"))
        );
        assert_eq!(d[0].worker_count, 3);
    }

    #[test]
    fn nodes_of_other_kinds_or_ids_are_ignored_and_no_node_means_idle() {
        let agents = [agent("w", "worker", "idle", true)];
        let mut other = node("w", "tool_call", json!({}));
        other.kind = "task".into();
        let (_, d) = derive(&agents, &[other, node("x", "tool_call", json!({}))]);
        assert_eq!(
            (d[0].state, d[0].worker_count, d[0].current_task.clone()),
            (DeskState::Idle, 0, None)
        );
    }

    #[test]
    fn the_coordinator_state_comes_from_its_live_node_or_its_own_status() {
        let agents = [agent("boss", "coordinator", "thinking", true)];
        assert_eq!(derive(&agents, &[]).0.unwrap().state, DeskState::Thinking);
        assert_eq!(
            derive(&agents, &[node("boss", "tool_call", json!({}))])
                .0
                .unwrap()
                .state,
            DeskState::ToolCall
        );
    }

    #[test]
    fn empty_strings_in_node_data_count_as_absent() {
        let agents = [agent("w", "worker", "idle", true)];
        let (_, d) = derive(
            &agents,
            &[node(
                "w",
                "thinking",
                json!({"currentTask": "", "currentTool": null}),
            )],
        );
        assert_eq!(
            (d[0].current_task.clone(), d[0].current_tool.clone()),
            (None, None)
        );
    }

    #[test]
    fn tool_categories_are_tested_in_order() {
        assert_eq!(
            tool_kind("search_knowledge"),
            ToolKind::Knowledge,
            "knowledge gana a search"
        );
        assert_eq!(tool_kind("web_search"), ToolKind::Browser);
        assert_eq!(tool_kind("fs_read"), ToolKind::Code);
        assert_eq!(
            tool_kind("Email__enviarEmail"),
            ToolKind::Communication,
            "se compara en minúsculas: «email» contiene «mail»"
        );
        assert_eq!(tool_kind("telegram_send"), ToolKind::Communication);
        assert_eq!(tool_kind("calc"), ToolKind::Generic);
    }
}
