use super::*;
use crate::office::model::Desk;
use crate::settings::api::Agent;
use serde_json::json;

fn desk(id: &str, state: DeskState, task: Option<&str>) -> Desk {
    Desk {
        id: id.into(),
        name: id.to_uppercase(),
        description: String::new(),
        state,
        current_tool: Some("web_search".into()),
        current_task: task.map(str::to_string),
        task_id: task.map(|_| format!("task-{id}")),
        delegated_by: None,
        worker_count: 1,
        color: [0.2, 0.6, 0.9],
    }
}

fn coord() -> Coordinator {
    Coordinator {
        id: "boss".into(),
        name: "Pepito".into(),
        description: String::new(),
        state: DeskState::Idle,
    }
}

fn event(id: &str, phase: &str) -> WorkEvent {
    WorkEvent {
        event_id: id.into(),
        phase: phase.into(),
        task_ref: "t".into(),
        task_name: "Buscar".into(),
        actor_id: "a".into(),
        target_id: Some("b".into()),
        tool_name: None,
        detail: None,
        timestamp: 0.0,
    }
}

fn graph_with(events: &[WorkEvent]) -> Graph {
    let mut g = Graph::default();
    for e in events {
        g.apply(
            "canvas:work_event",
            &json!({"eventId": e.event_id, "phase": e.phase, "taskRef": e.task_ref, "taskName": e.task_name, "actorId": e.actor_id, "targetId": e.target_id}),
            None,
            0.0,
        );
    }
    g
}

// ── formateadores ──

#[test]
fn token_counts_use_the_spanish_decimal_comma() {
    assert_eq!(format_token_count(830.0), "830");
    assert_eq!(format_token_count(6100.0), "6,1k");
    assert_eq!(format_token_count(1000.0), "1,0k");
    assert_eq!(format_token_count(-2500.0), "-2,5k");
    assert_eq!(format_token_count(0.0), "0");
}

#[test]
fn the_saving_chip_shows_the_sign_and_hides_small_changes() {
    assert_eq!(jev_saving_chip(6100.0), Some("−6,1k tok".into()));
    assert_eq!(
        jev_saving_chip(-300.0),
        Some("+300 tok".into()),
        "añadir tokens se marca con +"
    );
    assert_eq!(jev_saving_chip(49.0), None);
    assert_eq!(jev_saving_chip(-49.9), None);
    assert_eq!(jev_saving_chip(50.0), Some("−50 tok".into()));
}

#[test]
fn costs_get_more_decimals_when_they_are_tiny() {
    assert_eq!(format_jev_cost(0.0), "$0");
    assert_eq!(format_jev_cost(0.0012), "$0.0012");
    assert_eq!(format_jev_cost(0.5), "$0.500");
    assert_eq!(format_jev_cost(0.01), "$0.010");
}

#[test]
fn long_descriptions_are_cut_at_sixty_two_characters() {
    let s62 = "x".repeat(62);
    assert_eq!(short_description(&s62), s62);
    let cut = short_description(&"y".repeat(63));
    assert_eq!(cut.chars().count(), 60);
    assert!(cut.ends_with('…'));
    assert_eq!(
        short_description("ñandú ".repeat(20).as_str())
            .chars()
            .count(),
        60,
        "cuenta caracteres, no bytes"
    );
}

#[test]
fn tool_names_are_humanized() {
    assert_eq!(humanize_tool("web_search"), "Web search");
    assert_eq!(humanize_tool("Email__enviarEmail"), "EnviarEmail");
    assert_eq!(humanize_tool("fs-read"), "Fs read");
    assert_eq!(humanize_tool(""), "");
}

// ── director ──

fn desks_stuck(stuck: bool) -> Vec<Desk> {
    vec![desk(
        "w",
        if stuck {
            DeskState::Stuck
        } else {
            DeskState::Idle
        },
        Some("Enviar informe"),
    )]
}

#[test]
fn history_is_not_animated_and_the_first_new_event_starts_at_once() {
    let mut d = Director::default();
    let mut g = graph_with(&[event("old", "delegated")]);
    d.update(&g, &[], None, 0.0);
    assert!(
        d.active().is_none(),
        "lo que ya existía al abrir no se presenta"
    );
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "new", "phase": "delegated", "actorId": "a"}),
        None,
        0.0,
    );
    d.update(&g, &[], None, 1.0);
    let c = d.active().expect("el evento nuevo se presenta");
    assert_eq!(c.event.event_id, "new");
    assert!(
        (c.until - 3.8).abs() < 1e-5,
        "2,8 s desde que empieza: {}",
        c.until
    );
}

#[test]
fn an_event_is_shown_once_and_the_next_one_follows_when_it_expires() {
    let mut d = Director::default();
    let mut g = Graph::default();
    d.update(&g, &[], None, 0.0);
    for id in ["e1", "e2"] {
        g.apply(
            "canvas:work_event",
            &json!({"eventId": id, "phase": "delegated", "actorId": "a"}),
            None,
            0.0,
        );
    }
    d.update(&g, &[], None, 0.0);
    assert_eq!(d.active().unwrap().event.event_id, "e1");
    assert_eq!(d.queued(), 1);
    d.update(&g, &[], None, 2.0);
    assert_eq!(d.active().unwrap().event.event_id, "e1", "aún no caduca");
    d.update(&g, &[], None, 2.9);
    assert_eq!(
        d.active().unwrap().event.event_id,
        "e2",
        "al caducar entra el siguiente"
    );
    d.update(&g, &[], None, 6.0);
    assert!(
        d.active().is_none() && d.queued() == 0,
        "y cada evento se presenta una sola vez"
    );
}

#[test]
fn durations_depend_on_the_phase() {
    assert_eq!(cue_seconds("delegated"), 2.8);
    assert_eq!(cue_seconds("completed"), 3.2);
    for p in ["failed", "aborted", "review_failed", "blocked"] {
        assert_eq!(cue_seconds(p), 4.2, "{p}");
    }
}

#[test]
fn the_queue_keeps_at_most_three_waiting_events() {
    let mut d = Director::default();
    let mut g = Graph::default();
    d.update(&g, &[], None, 0.0);
    for i in 0..8 {
        g.apply(
            "canvas:work_event",
            &json!({"eventId": format!("e{i}"), "phase": "delegated", "actorId": "a"}),
            None,
            0.0,
        );
    }
    d.update(&g, &[], None, 0.0);
    assert!(d.active().is_some());
    assert_eq!(d.queued(), PRESENTATION_QUEUE_LIMIT);
}

#[test]
fn an_alert_interrupts_a_normal_event_which_waits_behind_it() {
    let mut d = Director::default();
    let mut g = Graph::default();
    d.update(&g, &[], None, 0.0);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "n", "phase": "delegated", "actorId": "a"}),
        None,
        0.0,
    );
    d.update(&g, &[], None, 0.0);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "x", "phase": "failed", "actorId": "a"}),
        None,
        0.0,
    );
    d.update(&g, &[], None, 0.5);
    let c = d.active().unwrap();
    assert_eq!(c.event.event_id, "x", "la alerta pasa por delante");
    assert!(
        (c.until - 4.7).abs() < 1e-5,
        "con su propia duración de 4,2 s: {}",
        c.until
    );
    assert_eq!(d.queued(), 1, "el evento interrumpido vuelve a la cola");
    d.update(&g, &[], None, 5.0);
    assert_eq!(
        d.active().unwrap().event.event_id,
        "n",
        "y se presenta después"
    );
}

#[test]
fn an_alert_does_not_interrupt_another_alert() {
    let mut d = Director::default();
    let mut g = Graph::default();
    d.update(&g, &[], None, 0.0);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "a1", "phase": "failed", "actorId": "a"}),
        None,
        0.0,
    );
    d.update(&g, &[], None, 0.0);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "a2", "phase": "aborted", "actorId": "a"}),
        None,
        0.0,
    );
    d.update(&g, &[], None, 1.0);
    assert_eq!(d.active().unwrap().event.event_id, "a1");
    assert_eq!(d.queued(), 1);
}

#[test]
fn a_newly_stuck_agent_generates_a_blocked_event_for_the_coordinator() {
    let mut d = Director::default();
    let g = Graph::default();
    let c = coord();
    d.update(&g, &desks_stuck(false), Some(&c), 0.0);
    assert!(d.active().is_none());
    d.update(&g, &desks_stuck(true), Some(&c), 1.0);
    let cue = d.active().expect("se presenta un evento de bloqueo");
    assert_eq!(cue.event.phase, "blocked");
    assert_eq!(
        (cue.event.actor_id.as_str(), cue.event.target_id.as_deref()),
        ("w", Some("boss"))
    );
    assert_eq!(cue.event.task_name, "Enviar informe");
    assert_eq!(
        cue.event.detail.as_deref(),
        Some("El agente necesita atención")
    );
    // Seguir atascado no genera otro, y recuperarse y volver a atascarse sí.
    d.update(&g, &desks_stuck(true), Some(&c), 9.0);
    assert!(d.active().is_none() && d.queued() == 0);
    d.update(&g, &desks_stuck(false), Some(&c), 10.0);
    d.update(&g, &desks_stuck(true), Some(&c), 11.0);
    assert!(d.active().is_some());
}

#[test]
fn an_agent_already_stuck_when_opening_is_history() {
    let mut d = Director::default();
    d.update(&Graph::default(), &desks_stuck(true), Some(&coord()), 0.0);
    assert!(d.active().is_none());
    let mut no_task = desks_stuck(false);
    no_task[0].current_task = None;
    no_task[0].state = DeskState::Stuck;
    let mut d = Director::default();
    d.update(&Graph::default(), &desks_stuck(false), None, 0.0);
    d.update(&Graph::default(), &no_task, None, 1.0);
    assert_eq!(
        d.active().unwrap().event.task_name,
        "Ejecución del agente",
        "sin tarea, un nombre por defecto"
    );
}

// ── ticker ──

#[test]
fn the_feed_starts_with_the_running_delegations_and_not_the_history() {
    let mut g = graph_with(&[event("old", "completed")]);
    g.apply("canvas:edge_add", &json!({"id": "e", "source": "boss", "target": "w", "edgeType": "delegates", "data": {"taskId": "t", "taskName": "Buscar"}}), None, 0.0);
    let mut f = Feed::default();
    f.update(&g, &[desk("w", DeskState::Idle, None)], Some(&coord()), 0.0);
    assert_eq!(f.items.len(), 1);
    assert_eq!(f.items[0].text, "Pepito → W: «Buscar»");
}

#[test]
fn a_missing_task_name_leaves_no_empty_quotes() {
    assert_eq!(with_task("A → B".into(), ""), "A → B");
    assert_eq!(with_task("A → B".into(), "Buscar"), "A → B: «Buscar»");
    let mut g = Graph::default();
    g.apply(
        "canvas:edge_add",
        &json!({"id": "e", "source": "boss", "target": "w", "edgeType": "delegates"}),
        None,
        0.0,
    );
    let mut f = Feed::default();
    f.update(&g, &[desk("w", DeskState::Idle, None)], Some(&coord()), 0.0);
    assert_eq!(f.items[0].text, "Pepito → W");
}

#[test]
fn every_feed_line_carries_a_local_clock_stamp() {
    let mut f = Feed::default();
    let mut g = Graph::default();
    f.update(&g, &[], None, 0.0);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "e", "phase": "delegated", "actorId": "a"}),
        None,
        0.0,
    );
    f.update(&g, &[], None, 1.0);
    let s = &f.items[0].stamp;
    assert_eq!(s.len(), 8, "{s}");
    assert!(
        s.chars().enumerate().all(|(i, c)| if i == 2 || i == 5 {
            c == ':'
        } else {
            c.is_ascii_digit()
        }),
        "{s}"
    );
}

#[test]
fn new_work_events_are_lines_colored_by_their_tone() {
    let mut f = Feed::default();
    let mut g = Graph::default();
    let desks = [
        desk("a", DeskState::Idle, None),
        desk("b", DeskState::Idle, None),
    ];
    f.update(&g, &desks, None, 0.0);
    g.apply("canvas:work_event", &json!({"eventId": "1", "phase": "delegated", "actorId": "a", "targetId": "b", "taskName": "Buscar"}), None, 0.0);
    g.apply("canvas:work_event", &json!({"eventId": "2", "phase": "completed", "actorId": "b", "targetId": "a", "taskName": "Buscar"}), None, 0.0);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "3", "phase": "failed", "actorId": "a", "taskName": "Enviar"}),
        None,
        0.0,
    );
    f.update(&g, &desks, Some(&coord()), 5.0);
    let lines: Vec<&str> = f.items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(
        lines,
        [
            "Falló · A → Pepito: «Enviar»",
            "Completado · B → A: «Buscar»",
            "Delegado · A → B: «Buscar»"
        ],
        "la más nueva primero"
    );
    assert_eq!(
        (f.items[0].kind, f.items[1].kind, f.items[2].kind),
        (FeedKind::Warn, FeedKind::Done, FeedKind::Work)
    );
    assert_eq!((f.items[0].color, f.items[1].color), (ROSE, GREEN));
    assert_eq!(
        f.items[2].color, desks[1].color,
        "el trabajo toma el color del destino"
    );
    f.update(&g, &desks, Some(&coord()), 6.0);
    assert_eq!(f.items.len(), 3, "no se repiten");
}

#[test]
fn only_context_decisions_reach_the_feed() {
    let mut f = Feed::default();
    let mut g = Graph::default();
    let desks = [
        desk("w", DeskState::Idle, None),
        desk("r", DeskState::Idle, None),
    ];
    f.update(&g, &desks, None, 0.0);
    g.apply("canvas:jev_decision", &json!({"eventId": "d1", "agentId": "w", "kind": "context", "summary": "recorta el contexto", "savedTokens": 6100, "recommendedAgentId": "r"}), None, 0.0);
    g.apply(
        "canvas:jev_decision",
        &json!({"eventId": "d2", "agentId": "w", "kind": "iteration", "summary": "no"}),
        None,
        0.0,
    );
    f.update(&g, &desks, None, 1.0);
    assert_eq!(f.items.len(), 1);
    assert_eq!(
        f.items[0].text,
        "Jev → W: recorta el contexto (−6,1k tok) · sugiere R"
    );
    assert_eq!(f.items[0].color, VIOLET);
}

#[test]
fn state_changes_are_reported_only_for_the_interesting_cases() {
    let mut f = Feed::default();
    let g = Graph::default();
    f.update(&g, &[desk("w", DeskState::Idle, None)], None, 0.0);
    f.update(&g, &[desk("w", DeskState::ToolCall, None)], None, 1.0);
    f.update(&g, &[desk("w", DeskState::Thinking, None)], None, 2.0);
    f.update(&g, &[desk("w", DeskState::Idle, None)], None, 3.0);
    f.update(&g, &[desk("w", DeskState::Stuck, Some("x"))], None, 4.0);
    f.update(&g, &[desk("w", DeskState::ToolCall, Some("x"))], None, 5.0);
    let lines: Vec<&str> = f.items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(
        lines,
        ["W necesita ayuda", "W está pensando…", "W · Web search"],
        "con tarea asignada solo se avisa del atasco"
    );
    assert_eq!(f.items[0].kind, FeedKind::Warn);
}

#[test]
fn the_feed_keeps_twenty_four_and_shows_six() {
    let mut f = Feed::default();
    let mut g = Graph::default();
    f.update(&g, &[], None, 0.0);
    for i in 0..40 {
        g.apply(
            "canvas:work_event",
            &json!({"eventId": format!("e{i}"), "phase": "delegated", "actorId": "a"}),
            None,
            0.0,
        );
    }
    f.update(&g, &[], None, 1.0);
    assert_eq!(f.items.len(), FEED_KEEP);
    assert_eq!(f.shown().count(), FEED_SHOWN);
}

// ── delegaciones ──

#[test]
fn a_delegation_moves_through_its_stages() {
    let mut g = Graph::default();
    g.apply("canvas:edge_add", &json!({"id": "e", "source": "boss", "target": "w", "edgeType": "delegates", "data": {"taskId": "t1", "taskName": "Buscar"}}), None, 0.0);
    let row = |g: &Graph, state| flow(g, &[desk("w", state, None)], Some(&coord()));
    assert_eq!(row(&g, DeskState::Idle)[0].stage, 0, "Delegado");
    assert_eq!(row(&g, DeskState::Thinking)[0].stage, 1, "Trabajando");
    assert_eq!(row(&g, DeskState::ToolCall)[0].stage, 1);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "1", "phase": "completed", "taskRef": "t1", "actorId": "w"}),
        None,
        0.0,
    );
    let r = &row(&g, DeskState::Idle)[0];
    assert_eq!((r.stage, r.failed), (3, false), "Entregado");
    assert_eq!(
        (r.source.as_str(), r.target.as_str(), r.task.as_str()),
        ("Pepito", "W", "Buscar")
    );
}

#[test]
fn a_review_is_at_least_in_the_reviewing_stage_and_failures_are_flagged() {
    let mut g = Graph::default();
    g.apply("canvas:edge_add", &json!({"id": "e", "source": "w", "target": "r", "edgeType": "reviews", "data": {"taskId": "t2"}}), None, 0.0);
    let desks = [
        desk("w", DeskState::Idle, None),
        desk("r", DeskState::Idle, None),
    ];
    assert_eq!(flow(&g, &desks, None)[0].stage, 2);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "x", "phase": "review_failed", "taskRef": "t2", "actorId": "r"}),
        None,
        0.0,
    );
    let r = &flow(&g, &desks, None)[0];
    assert!(r.failed && r.stage == 2);
    g.apply(
        "canvas:work_event",
        &json!({"eventId": "y", "phase": "blocked", "taskRef": "t2", "actorId": "r"}),
        None,
        0.0,
    );
    assert!(
        flow(&g, &desks, None)[0].failed,
        "bloqueado también cuenta como atención"
    );
}

#[test]
fn an_edge_without_a_task_id_matches_no_event() {
    let mut g = graph_with(&[event("e", "completed")]);
    g.apply(
        "canvas:edge_add",
        &json!({"id": "e", "source": "a", "target": "b", "edgeType": "delegates"}),
        None,
        0.0,
    );
    let r = &flow(&g, &[desk("b", DeskState::Idle, None)], None)[0];
    assert_eq!((r.stage, r.failed), (0, false));
}

// ── selección ──

fn office(workers: usize) -> OfficeState {
    let mut o = OfficeState::default();
    let a = |id: &str, role: &str| Agent {
        id: id.into(),
        name: id.into(),
        role: role.into(),
        status: "idle".into(),
        enabled: true,
        ..Default::default()
    };
    o.agents.push(a("boss", "coordinator"));
    for i in 0..workers {
        o.agents.push(a(&format!("w{i}"), "worker"));
    }
    o.step(0.0);
    o
}

const VIEW: (f32, f32) = (900.0, 600.0);

#[test]
fn clicking_where_the_queen_is_picks_her_and_empty_space_picks_nothing() {
    let o = office(0);
    let (qx, qy) = screen_pos(&o.camera, QUEEN_CENTER, VIEW).unwrap();
    assert_eq!(pick(&o, (qx, qy), VIEW), Pick::Queen);
    assert_eq!(
        pick(&o, (qx + 2.0, qy - 3.0), VIEW),
        Pick::Queen,
        "con algo de margen"
    );
    assert_eq!(pick(&o, (20.0, 20.0), VIEW), Pick::Nothing);
}

#[test]
fn clicking_a_bee_picks_it_and_the_nearest_to_the_camera_wins() {
    let o = office(3);
    for (id, sim) in &o.bees {
        let (x, y) = screen_pos(&o.camera, sim.bee.pos, VIEW).unwrap();
        // Si dos coinciden en pantalla, gana la más cercana a la cámara; en cualquier caso se elige una abeja.
        assert!(
            matches!(pick(&o, (x, y), VIEW), Pick::Bee(_) | Pick::Queen),
            "{id}"
        );
    }
    let mut o = office(1);
    o.bees.get_mut("w0").unwrap().bee.pos = [0.0, 3.1, 8.0]; // delante de la reina, hacia la cámara
    let (x, y) = screen_pos(&o.camera, [0.0, 3.1, 8.0], VIEW).unwrap();
    assert_eq!(
        pick(&o, (x, y), VIEW),
        Pick::Bee("w0".into()),
        "la abeja más cercana a la cámara tapa a la reina"
    );
}

#[test]
fn a_degenerate_viewport_picks_nothing() {
    assert_eq!(pick(&office(2), (10.0, 10.0), (0.0, 0.0)), Pick::Nothing);
    assert_eq!(screen_pos(&Camera::default(), [0.0; 3], (0.0, 100.0)), None);
}

#[test]
fn picking_the_same_thing_twice_deselects_it() {
    let mut sel = None;
    apply_pick(&mut sel, Some("boss"), &Pick::Bee("w1".into()));
    assert_eq!(sel.as_deref(), Some("w1"));
    apply_pick(&mut sel, Some("boss"), &Pick::Bee("w1".into()));
    assert_eq!(sel, None, "tocarla otra vez la suelta");
    apply_pick(&mut sel, Some("boss"), &Pick::Queen);
    assert_eq!(sel.as_deref(), Some("boss"));
    apply_pick(&mut sel, Some("boss"), &Pick::Bee("w2".into()));
    assert_eq!(sel.as_deref(), Some("w2"), "pasar de una a otra");
    apply_pick(&mut sel, Some("boss"), &Pick::Nothing);
    assert_eq!(sel, None, "tocar el vacío suelta");
    apply_pick(&mut sel, None, &Pick::Queen);
    assert_eq!(sel, None, "sin coordinador no hay reina que elegir");
}
