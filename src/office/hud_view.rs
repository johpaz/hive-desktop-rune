//! Vista de Rune del HUD de la oficina: placas de nombre ancladas a las abejas y los paneles de la
//! interfaz (barra superior, «Ahora ocurre», agentes, inspector, ticker y Jev). Va como hijos
//! absolutos del hueco de la escena.

use crate::ui::text;
use crate::ui::Tap;
use rune::tok::{radius, type_};
use rune::{col, el, oklcha, row, Color, El, ScrollSpec};

use super::effects::{self, ORACLE_POSITION};
use super::graph::{phase_label, phase_tone, InteractionKind, Tone};
use super::hud::{self, FeedKind, STAGES};
use super::model::{Coordinator, Desk, DeskState};
use super::scene::{point_of, COORDINATOR_POINT};
use super::state::OfficeState;
use crate::ui::theme as th;
use crate::State;

fn color_from(c: [f32; 3], a: f32) -> Color {
    // sRGB 0..1 a Color de Rune mediante su hex (los canales ya vienen en sRGB).
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        (c[0] * 255.0).round() as u8,
        (c[1] * 255.0).round() as u8,
        (c[2] * 255.0).round() as u8
    );
    rune::color::hex(&hex).unwrap_or(Color::WHITE).alpha(a)
}

fn glass() -> El {
    col()
        .radius(radius::SM)
        .fill(ok(0.16, 0.03, 0.78))
        .border(1.0, oklcha(0.7, 0.15, 230.0, 0.22))
        .px(12.0)
        .py(8.0)
}

/// Color de los textos del HUD: blanco sobre el cristal oscuro, tinta sobre el claro.
fn fg() -> Color {
    if th::is_light() {
        th::foreground()
    } else {
        Color::WHITE
    }
}

/// Fondo de los paneles del HUD: cristal azul noche (tema oscuro) o crema casi opaco (tema claro; la escena de detrás es de tono medio).
fn ok(l: f32, c: f32, a: f32) -> Color {
    if th::is_light() {
        oklcha(1.0 - l * 0.25, c * 0.3, 85.0, (a + 0.2).min(0.94))
    } else {
        oklcha(l, c, 264.0, a)
    }
}

fn dim() -> Color {
    if th::is_light() {
        th::muted_foreground()
    } else {
        oklcha(0.75, 0.02, 264.0, 1.0)
    }
}

fn state_color(s: DeskState) -> Color {
    match s {
        DeskState::Idle => oklcha(0.6, 0.04, 264.0, 1.0),
        DeskState::Disabled => oklcha(0.4, 0.02, 264.0, 1.0),
        DeskState::Archived => oklcha(0.55, 0.08, 70.0, 1.0),
        DeskState::Thinking => color_from([0.4, 0.8, 1.0], 1.0),
        DeskState::ToolCall => color_from([0.96, 0.62, 0.04], 1.0),
        DeskState::Stuck => color_from([0.98, 0.44, 0.52], 1.0),
    }
}

fn kind_label(k: InteractionKind) -> &'static str {
    if k == InteractionKind::Reviews {
        "Revisión"
    } else {
        "Delegación"
    }
}

fn dot(c: Color) -> El {
    el().size(8.0, 8.0).radius(4.0).fill(c)
}

/// Un contenedor sin ancho anclado en (x, y) cuyos hijos quedan centrados sobre ese punto.
fn anchored(x: f32, y: f32, viewport: f32) -> El {
    let w = 200.0_f32.min((viewport - 24.0).max(0.0));
    col()
        .absolute(
            (x - w / 2.0).clamp(12.0, (viewport - w - 12.0).max(12.0)),
            y,
        )
        .w(w)
        .items_center()
}

/// Placas, tarea y herramienta de cada abeja, proyectadas a pantalla.
fn plates(o: &OfficeState, desks: &[Desk]) -> Vec<El> {
    let (vw, vh) = o.viewport;
    let mut out = Vec::new();
    for d in desks {
        let Some(sim) = o.bees.get(&d.id) else {
            continue;
        };
        let p = sim.bee.pos;
        let selected = o.selected.as_deref() == Some(d.id.as_str());
        let Some((x, y)) = hud::screen_pos(&o.camera, [p[0], p[1] - 0.55, p[2]], o.viewport) else {
            continue;
        };
        if x < -60.0 || x > vw + 60.0 || y < -20.0 || y > vh + 20.0 {
            continue;
        }
        let border = if selected {
            color_from(d.color, 0.9)
        } else {
            oklcha(0.7, 0.15, 230.0, 0.18)
        };
        let plate = row()
            .gap(5.0)
            .px(8.0)
            .py(3.0)
            .center_y()
            .radius(10.0)
            .fill(ok(0.14, 0.03, if selected { 0.92 } else { 0.7 }))
            .border(1.0, border)
            .child(dot(color_from(d.color, 1.0)))
            .child(text(d.name.clone()).font(type_::LABEL).color(fg()));
        let mut stack = anchored(x, y, vw).child(plate);
        if selected {
            let desc = if d.description.is_empty() {
                "Agente especialista".to_string()
            } else {
                hud::short_description(&d.description)
            };
            stack = stack.child(text(desc).font(type_::LABEL).color(dim()));
        }
        out.push(stack);

        // Tarea (arriba) y herramienta (encima, solo si está ejecutando una).
        if let Some((tx, ty)) = hud::screen_pos(&o.camera, [p[0], p[1] + 0.96, p[2]], o.viewport) {
            let mut chips = col().gap(3.0).items_center();
            if let (DeskState::ToolCall, Some(tool)) = (d.state, &d.current_tool) {
                chips = chips.child(
                    row()
                        .px(7.0)
                        .py(2.0)
                        .radius(8.0)
                        .fill(color_from(d.color, 0.28))
                        .child(
                            text(hud::humanize_tool(tool))
                                .font(type_::LABEL)
                                .color(fg()),
                        ),
                );
            }
            if let Some(task) = &d.current_task {
                chips = chips.child(
                    row()
                        .px(8.0)
                        .py(3.0)
                        .radius(10.0)
                        .fill(ok(0.14, 0.03, 0.78))
                        .child(text(format!("◈ {task}")).font(type_::LABEL).color(fg())),
                );
            }
            if d.current_task.is_some() || d.current_tool.is_some() {
                out.push(anchored(tx, ty - 34.0, vw).child(chips));
            }
        }
    }
    out
}

/// Etiquetas ancladas a puntos del mundo: el paquete en vuelo, el rayo de Jev, el oráculo y las delegaciones.
fn world_labels(o: &OfficeState, desks: &[Desk], coordinator: Option<&Coordinator>) -> Vec<El> {
    let (vw, vh) = o.viewport;
    let cid = coordinator.map(|c| c.id.as_str());
    let name = |id: &str| {
        desks
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .or_else(|| coordinator.filter(|c| c.id == id).map(|c| c.name.clone()))
            .unwrap_or_else(|| id.to_string())
    };
    let at = |p: [f32; 3]| {
        hud::screen_pos(&o.camera, p, o.viewport)
            .filter(|(x, y)| *x > -80.0 && *x < vw + 80.0 && *y > -30.0 && *y < vh + 30.0)
    };
    let chip = |bg: Color, lines: Vec<(String, bool)>| {
        lines.into_iter().fold(
            col().px(8.0).py(3.0).radius(10.0).fill(bg).items_center(),
            |c, (t, strong)| {
                c.child(
                    text(t)
                        .font(type_::LABEL)
                        .color(if strong { fg() } else { dim() }),
                )
            },
        )
    };
    let mut out = Vec::new();

    // Paquetes de trabajo: «quién hace qué a quién» y la fase, sobre el paquete.
    for f in &o.transfers.flights {
        let e = &f.event;
        let (Some(from), Some(to)) = (
            point_of(o, &e.actor_id, cid),
            point_of(o, e.target_id.as_deref().or(cid).unwrap_or(""), cid),
        ) else {
            continue;
        };
        let (pos, _) = effects::packet_pose(from, to, effects::progress(o.t, f.started));
        let Some((x, y)) = at([pos[0], pos[1] + 0.75, pos[2]]) else {
            continue;
        };
        let target = e
            .target_id
            .as_deref()
            .map(&name)
            .unwrap_or_else(|| coordinator.map_or("—".into(), |c| c.name.clone()));
        let (line, phase) = effects::transfer_label(&e.phase, &name(&e.actor_id), &target);
        out.push(
            anchored(x, y - 30.0, vw)
                .key(format!("office/transfer-label/{}", e.event_id))
                .child(chip(
                    ok(0.14, 0.03, 0.85),
                    vec![(line, true), (phase, false)],
                )),
        );
    }

    // Oráculo de Jev y el texto de cada uno de sus rayos.
    if let Some(st) = o.graph.jev_status.as_ref().filter(|j| j.state != "off") {
        if let Some((x, y)) = at([
            ORACLE_POSITION[0],
            ORACLE_POSITION[1] + 1.55,
            ORACLE_POSITION[2],
        ]) {
            let tag = if st.backend.as_deref() == Some("kev") {
                "KEV"
            } else {
                "JEV"
            };
            out.push(anchored(x, y - 12.0, vw).key("office/jev-tag").child(chip(
                color_from(hud::VIOLET, 0.3),
                vec![(tag.to_string(), true)],
            )));
        }
        for b in &o.jev_fx.beams {
            let agent = b.decision.agent_id.as_str();
            let end = if Some(agent) == cid {
                COORDINATOR_POINT
            } else {
                o.bee_pos(agent).unwrap_or(COORDINATOR_POINT)
            };
            let Some((x, y)) = at([end[0], end[1] + 1.35, end[2]]) else {
                continue;
            };
            let mut lines = vec![(
                effects::jev_beam_label(st.backend.as_deref(), &b.decision.kind),
                true,
            )];
            if let Some(c) = hud::jev_saving_chip(b.decision.saved_tokens) {
                lines.push((c, false));
            }
            lines.push((
                format!("{} ms", b.decision.latency_ms.round() as i64),
                false,
            ));
            out.push(
                anchored(x, y - 30.0, vw)
                    .key(format!("office/jev-beam-label/{}", b.decision.event_id))
                    .child(chip(color_from(hud::VIOLET, 0.3), lines)),
            );
        }
    }

    // Delegaciones y revisiones en curso: un rótulo a mitad del rayo.
    for i in o.graph.interactions() {
        let (Some(from), Some(to)) = (
            point_of(o, &i.source_id, cid),
            point_of(o, &i.target_id, cid),
        ) else {
            continue;
        };
        let len =
            ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2) + (to[2] - from[2]).powi(2))
                .sqrt();
        if len <= 0.5 {
            continue;
        }
        let mid = [
            (from[0] + to[0]) / 2.0,
            (from[1] + to[1]) / 2.0 + 0.55,
            (from[2] + to[2]) / 2.0,
        ];
        let Some((x, y)) = at(mid) else { continue };
        let head = if i.kind == InteractionKind::Reviews {
            "REVISIÓN EN CURSO"
        } else {
            "TRABAJO DELEGADO"
        };
        let mut lines = vec![(head.to_string(), true)];
        if let Some(t) = i.task_name.clone().filter(|t| !t.is_empty()) {
            lines.push((t, false));
        }
        out.push(
            anchored(x, y - 14.0, vw)
                .key(format!("office/beam-label/{}", i.id))
                .child(chip(ok(0.14, 0.03, 0.8), lines)),
        );
    }
    out
}

fn top_bar(s: &State, desks: &[Desk]) -> El {
    let o = &s.office;
    let working = desks
        .iter()
        .filter(|d| matches!(d.state, DeskState::Thinking | DeskState::ToolCall))
        .count();
    let amber = color_from([0.96, 0.62, 0.04], 1.0);
    let toggle = |label: &'static str, key: &'static str, on: bool, f: fn(&mut State)| {
        row()
            .key(key)
            .px(10.0)
            .py(4.0)
            .radius(8.0)
            .fill(if on {
                oklcha(0.7, 0.15, 230.0, 0.3)
            } else {
                ok(0.3, 0.02, 0.6)
            })
            .child(text(label).font(type_::LABEL).color(fg()))
            .tap(f)
    };
    let live = s.chat_ready;
    let mut right = row().gap(8.0).center_y().child(
        row()
            .gap(5.0)
            .px(8.0)
            .py(4.0)
            .center_y()
            .radius(8.0)
            .fill(ok(0.2, 0.03, 0.7))
            .child(dot(if live {
                color_from([0.53, 0.94, 0.67], 1.0)
            } else {
                color_from([0.98, 0.44, 0.52], 1.0)
            }))
            .child(
                text(if live { "LIVE" } else { "OFFLINE" })
                    .font(type_::LABEL)
                    .color(fg()),
            ),
    );
    if let Some(j) = o.graph.jev_status.as_ref().filter(|j| j.state != "off") {
        let saved = j.totals.as_ref().map_or(0.0, |t| t.saved_tokens);
        let label = format!(
            "{}{}",
            j.backend.as_deref().unwrap_or("jev").to_uppercase(),
            if saved > 0.0 {
                format!(" −{}", hud::format_token_count(saved))
            } else {
                String::new()
            }
        );
        right = right.child(
            row()
                .px(8.0)
                .py(4.0)
                .radius(8.0)
                .fill(color_from(hud::VIOLET, 0.25))
                .child(text(label).font(type_::LABEL).color(fg())),
        );
    }
    right = right
        .child(toggle(
            if o.high_quality { "Alta" } else { "Eco" },
            "office/quality",
            o.high_quality,
            |s| s.office.high_quality = !s.office.high_quality,
        ))
        .child(toggle(
            if o.sound_on { "Audio" } else { "Silencio" },
            "office/sound",
            o.sound_on,
            |s| {
                s.office.sound_on = !s.office.sound_on;
                s.office.pending_sounds.clear();
            },
        ))
        .child(toggle(
            if o.calm { "Enfoque" } else { "Fijo" },
            "office/motion",
            o.calm,
            |s| s.office.calm = !s.office.calm,
        ));
    row()
        .absolute(12.0, 12.0)
        .w(o.viewport.0.max(240.0) - 24.0)
        .flex_wrap()
        .gap(8.0)
        .justify_between()
        .center_y()
        .px(14.0)
        .py(8.0)
        .radius(radius::SM)
        .fill(ok(0.16, 0.03, 0.78))
        .border(1.0, oklcha(0.7, 0.15, 230.0, 0.22))
        .child(
            row()
                .gap(10.0)
                .center_y()
                .child(text("⬡").font(type_::TITLE).color(amber))
                .child(
                    col()
                        .child(text("HoloHive · Oficina 3D").font(type_::BODY).color(fg()))
                        .child(
                            text(format!(
                                "{} especialistas · {} trabajando",
                                desks.len(),
                                working
                            ))
                            .font(type_::LABEL)
                            .color(dim()),
                        ),
                ),
        )
        .child(right)
}

fn now_happening(o: &OfficeState, desks: &[Desk], coordinator: Option<&Coordinator>) -> Option<El> {
    let cue = o.director.active()?;
    let name = |id: &str| {
        desks
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .or_else(|| coordinator.filter(|c| c.id == id).map(|c| c.name.clone()))
            .unwrap_or_else(|| id.to_string())
    };
    let e = &cue.event;
    let target = e
        .target_id
        .as_deref()
        .map(&name)
        .unwrap_or_else(|| coordinator.map_or("—".into(), |c| c.name.clone()));
    let accent = match phase_tone(&e.phase) {
        Tone::Alert => color_from(hud::ROSE, 1.0),
        Tone::Done => color_from(hud::GREEN, 1.0),
        Tone::Work => color_from([0.4, 0.8, 1.0], 1.0),
    };
    let w = (o.viewport.0 - 40.0).clamp(220.0, 400.0);
    let x = (o.viewport.0 - w) / 2.0;
    let mut c = glass()
        .absolute(x.max(12.0), 76.0)
        .w(w)
        .gap(2.0)
        .border(1.0, accent.alpha(0.5))
        .child(text("AHORA OCURRE").font(type_::LABEL).color(dim()))
        .child(text(phase_label(&e.phase)).font(type_::BODY).color(accent))
        .child(
            text(format!("{} → {}", name(&e.actor_id), target))
                .font(type_::LABEL)
                .color(fg()),
        )
        .child(
            text(e.task_name.clone())
                .font(type_::LABEL)
                .color(fg())
                .wrap(),
        );
    if let Some(d) = &e.detail {
        c = c.child(text(d.clone()).font(type_::LABEL).color(dim()).wrap());
    }
    Some(c)
}

fn roster(s: &State, desks: &[Desk], coordinator: Option<&Coordinator>) -> El {
    let o = &s.office;
    let w = (o.viewport.0 * 0.28).clamp(210.0, 330.0);
    let x = (o.viewport.0 - w - 12.0).max(12.0);
    let collapsed = if is_narrow(o) {
        !o.roster_open_narrow
    } else {
        o.roster_collapsed
    };
    let head = row()
        .justify_between()
        .center_y()
        .child(
            col()
                .child(
                    text("CENTRO DE OPERACIONES")
                        .font(type_::LABEL)
                        .color(dim()),
                )
                .child(text("Quién hace qué").font(type_::BODY).color(fg())),
        )
        .child(
            text(if collapsed { "▸" } else { "▾" })
                .key("office/roster-toggle")
                .font(type_::BODY)
                .color(fg())
                .tap(|s: &mut State| {
                    if is_narrow(&s.office) {
                        s.office.roster_open_narrow = !s.office.roster_open_narrow
                    } else {
                        s.office.roster_collapsed = !s.office.roster_collapsed
                    }
                }),
        );
    if collapsed {
        return glass().absolute(x, 76.0).w(w).child(head);
    }
    let mut body = col().gap(6.0).w_full();
    if let Some(c) = coordinator {
        let id = c.id.clone();
        let sel = o.selected.as_deref() == Some(c.id.as_str());
        body = body.child(
            row()
                .key("office/coord")
                .gap(8.0)
                .px(8.0)
                .py(6.0)
                .center_y()
                .radius(8.0)
                .fill(color_from(
                    [0.96, 0.62, 0.04],
                    if sel { 0.32 } else { 0.14 },
                ))
                .child(
                    text("♛")
                        .font(type_::BODY)
                        .color(color_from([0.96, 0.62, 0.04], 1.0)),
                )
                .child(
                    col()
                        .grow(1.0)
                        .child(
                            text("ABEJA REINA · COORDINADOR")
                                .font(type_::LABEL)
                                .color(dim()),
                        )
                        .child(text(c.name.clone()).font(type_::BODY).color(fg()))
                        .child(
                            text("Asigna · supervisa · recibe")
                                .font(type_::LABEL)
                                .color(dim()),
                        ),
                )
                .tap(move |s: &mut State| {
                    let mut sel = s.office.selected.take();
                    hud::apply_pick(&mut sel, Some(&id), &hud::Pick::Queen);
                    s.office.selected = sel;
                }),
        );
    }
    let flows = hud::flow(&o.graph, desks, coordinator);
    body = body.child(
        text(format!("Flujo activo  {}", flows.len()))
            .font(type_::LABEL)
            .color(dim()),
    );
    if flows.is_empty() {
        body = body.child(
            text("El equipo está disponible. Las delegaciones aparecerán aquí.")
                .font(type_::LABEL)
                .color(dim())
                .wrap(),
        );
    }
    for f in &flows {
        let last_label = if f.failed { "Atención" } else { STAGES[3] };
        let bar = row().gap(3.0).children((0..4).map(|i| {
            let on = i as u8 <= f.stage;
            let c = if f.failed && i == 3 {
                color_from(hud::ROSE, 1.0)
            } else if on {
                color_from([0.4, 0.8, 1.0], 1.0)
            } else {
                ok(0.3, 0.02, 0.8)
            };
            el().grow(1.0).h(4.0).radius(2.0).fill(c)
        }));
        body = body.child(
            col()
                .gap(3.0)
                .px(8.0)
                .py(6.0)
                .radius(8.0)
                .fill(ok(0.2, 0.03, 0.6))
                .child(
                    text(format!("{} → {}", f.source, f.target))
                        .font(type_::LABEL)
                        .color(fg()),
                )
                .child(
                    text(if f.task.is_empty() {
                        kind_label(f.kind).to_string()
                    } else {
                        format!("{} · {}", kind_label(f.kind), f.task)
                    })
                    .font(type_::LABEL)
                    .color(dim())
                    .wrap(),
                )
                .child(bar)
                .child(
                    text(if f.stage == 3 {
                        last_label
                    } else {
                        STAGES[f.stage as usize]
                    })
                    .font(type_::LABEL)
                    .color(dim()),
                ),
        );
    }
    body = body.child(
        text(format!("Especialistas  {}", desks.len()))
            .font(type_::LABEL)
            .color(dim()),
    );
    for d in desks {
        let id = d.id.clone();
        let sel = o.selected.as_deref() == Some(d.id.as_str());
        let line = d.current_task.clone().unwrap_or_else(|| {
            if d.description.is_empty() {
                String::new()
            } else {
                hud::short_description(&d.description)
            }
        });
        body = body.child(
            row()
                .key(format!("office/desk/{}", d.id))
                .gap(8.0)
                .px(8.0)
                .py(5.0)
                .center_y()
                .radius(8.0)
                .fill(if sel {
                    color_from(d.color, 0.25)
                } else {
                    ok(0.2, 0.03, 0.45)
                })
                .child(dot(state_color(d.state)))
                .child(
                    col()
                        .grow(1.0)
                        .child(text(d.name.clone()).font(type_::BODY).color(fg()))
                        .child(text(line).font(type_::LABEL).color(dim()).wrap()),
                )
                .child(
                    text(d.state.label())
                        .font(type_::LABEL)
                        .color(state_color(d.state)),
                )
                .tap(move |s: &mut State| {
                    let mut sel = s.office.selected.take();
                    hud::apply_pick(&mut sel, None, &hud::Pick::Bee(id.clone()));
                    s.office.selected = sel;
                }),
        );
    }
    let max_h = (o.viewport.1 - 76.0 - 12.0).max(120.0);
    glass()
        .absolute(x, 76.0)
        .w(w)
        .h(max_h.min(520.0))
        .gap(8.0)
        .child(head)
        .child(
            body.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .shrink()
            .min_h(0.0)
            .grow(1.0),
        )
}

fn inspector(o: &OfficeState, desks: &[Desk], coordinator: Option<&Coordinator>) -> Option<El> {
    let id = o.selected.as_deref()?;
    let row_of = |k: &str, v: String| {
        row()
            .gap(8.0)
            .justify_between()
            .child(text(k.to_string()).font(type_::LABEL).color(dim()))
            .child(text(v).font(type_::LABEL).color(fg()).wrap())
    };
    let name_of = |id: &str| {
        desks
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .or_else(|| coordinator.filter(|c| c.id == id).map(|c| c.name.clone()))
            .unwrap_or_else(|| id.to_string())
    };
    let jev = o.graph.jev_by_agent.get(id).map(|t| {
        format!(
            "{} dec. · {}",
            t.decisions as i64,
            hud::jev_saving_chip(t.saved_tokens).unwrap_or_else(|| "±0 tok".into())
        )
    });
    let close = text("✕")
        .key("office/inspector-close")
        .font(type_::BODY)
        .color(dim())
        .tap(|s: &mut State| s.office.selected = None);
    let w = 250.0;
    if let Some(d) = desks.iter().find(|d| d.id == id) {
        let mut c = glass()
            .absolute(12.0, 76.0)
            .w(w)
            .gap(4.0)
            .border(1.0, color_from(d.color, 0.6))
            .child(
                row()
                    .justify_between()
                    .child(
                        col()
                            .child(text(d.name.clone()).font(type_::BODY).color(fg()))
                            .child(text(d.id.clone()).font(type_::LABEL).color(dim())),
                    )
                    .child(close),
            )
            .child(row_of("Rol", "Especialista".into()))
            .child(row_of("Estado", d.state.label().into()))
            .child(row_of(
                "Tarea",
                d.current_task.clone().unwrap_or_else(|| "—".into()),
            ))
            .child(row_of(
                "Delegado por",
                d.delegated_by.as_deref().map_or("—".into(), name_of),
            ))
            .child(row_of(
                "Herramienta",
                d.current_tool
                    .as_deref()
                    .map_or("—".into(), hud::humanize_tool),
            ))
            .child(row_of("Workers", d.worker_count.to_string()));
        if let Some(j) = jev {
            c = c.child(row_of("Jev", j));
        }
        if !d.description.is_empty() {
            c = c.child(
                text(d.description.clone())
                    .font(type_::LABEL)
                    .color(dim())
                    .wrap(),
            );
        }
        return Some(c);
    }
    let c = coordinator.filter(|c| c.id == id)?;
    let mut card = glass()
        .absolute(12.0, 76.0)
        .w(w)
        .gap(4.0)
        .border(1.0, color_from([0.96, 0.62, 0.04], 0.6))
        .child(
            row()
                .justify_between()
                .child(
                    col()
                        .child(
                            text(c.name.clone())
                                .font(type_::BODY)
                                .color(color_from([0.96, 0.62, 0.04], 1.0)),
                        )
                        .child(
                            text("Coordinador principal")
                                .font(type_::LABEL)
                                .color(dim()),
                        ),
                )
                .child(close),
        )
        .child(row_of("Función", "Asigna y supervisa".into()))
        .child(row_of("Estado", c.state.label().into()));
    if let Some(j) = jev {
        card = card.child(row_of("Jev", j));
    }
    if !c.description.is_empty() {
        card = card.child(
            text(c.description.clone())
                .font(type_::LABEL)
                .color(dim())
                .wrap(),
        );
    }
    Some(card)
}

fn ticker(o: &OfficeState) -> Option<El> {
    if o.feed.items.is_empty() {
        return None;
    }
    let mut c = col()
        .absolute(
            12.0,
            (o.viewport.1 - 12.0 - 20.0 - o.feed.shown().count() as f32 * 22.0).max(80.0),
        )
        .w((o.viewport.0 * 0.4).clamp(220.0, 340.0))
        .gap(3.0)
        .child(text("ACTIVIDAD").font(type_::LABEL).color(dim()));
    for (n, it) in o.feed.shown().enumerate() {
        let color = match it.kind {
            FeedKind::Warn => color_from(hud::ROSE, 1.0),
            FeedKind::Done => color_from(hud::GREEN, 1.0),
            _ => fg(),
        };
        let stamp = it.stamp.clone();
        c = c.child(
            row()
                .key(format!("office/feed/{n}"))
                .gap(6.0)
                .center_y()
                .px(8.0)
                .py(3.0)
                .radius(8.0)
                .fill(ok(0.14, 0.03, 0.6))
                .child(dot(color_from(it.color, 1.0)))
                .child(
                    text(it.text.clone())
                        .font(type_::LABEL)
                        .color(color)
                        .grow(1.0),
                )
                .child(text(stamp).font(type_::LABEL).color(dim())),
        );
    }
    Some(c)
}

fn jev_panel(o: &OfficeState) -> Option<El> {
    let st = o.graph.jev_status.as_ref().filter(|j| j.state != "off")?;
    let t = st.totals.clone().unwrap_or_default();
    let state_label = match st.state.as_str() {
        "ready" => "activo",
        "fallback" => "flujo clásico",
        _ => "desactivado",
    };
    let saved_label = if t.saved_tokens < 0.0 {
        "Tokens añadidos"
    } else {
        "Tokens ahorrados"
    };
    let metric = |k: &str, v: String| {
        row()
            .justify_between()
            .child(text(k.to_string()).font(type_::LABEL).color(dim()))
            .child(text(v).font(type_::LABEL).color(fg()))
    };
    let backend = st.backend.as_deref().unwrap_or("jev");
    let mut c = glass()
        .absolute(12.0, if o.selected.is_some() { 330.0 } else { 76.0 })
        .w(220.0)
        .gap(3.0)
        .border(1.0, color_from(hud::VIOLET, 0.4))
        .child(
            row()
                .justify_between()
                .child(
                    col()
                        .child(text("PLANO DE DECISIÓN").font(type_::LABEL).color(dim()))
                        .child(
                            text(format!(
                                "{} · {}",
                                if backend == "kev" { "Kev" } else { "Jev" },
                                state_label
                            ))
                            .font(type_::BODY)
                            .color(color_from(hud::VIOLET, 1.0)),
                        ),
                )
                .child(
                    text(if o.jev_collapsed { "▸" } else { "▾" })
                        .key("office/jev-toggle")
                        .font(type_::BODY)
                        .color(fg())
                        .tap(|s: &mut State| s.office.jev_collapsed = !s.office.jev_collapsed),
                ),
        )
        .child(metric("Decisiones", format!("{}", t.decisions as i64)))
        .child(metric(
            saved_label,
            format!("~{}", hud::format_token_count(t.saved_tokens.abs())),
        ))
        .child(metric("Costo", hud::format_jev_cost(t.cost_usd)));
    if !o.jev_collapsed {
        if st.state == "fallback" {
            c = c.child(
                text("Jev no responde: se usa el flujo clásico.")
                    .font(type_::LABEL)
                    .color(dim())
                    .wrap(),
            );
        }
        if let Some(d) = o.graph.jev_decisions.last() {
            c = c.child(
                text(format!("Última: {}", d.summary))
                    .font(type_::LABEL)
                    .color(fg())
                    .wrap(),
            );
        }
    }
    Some(c)
}

/// Todos los elementos del HUD como hijos absolutos del hueco.
/// Por debajo de este ancho de escenario el HUD se simplifica: sin «Ahora ocurre», plano de Jev ni actividad, y el Centro de operaciones plegado.
pub const NARROW_STAGE: f32 = 980.0;

fn is_narrow(o: &crate::office::state::OfficeState) -> bool {
    o.viewport.0 < NARROW_STAGE
}

pub fn overlay(s: &State) -> Vec<El> {
    let o = &s.office;
    let (coordinator, desks) = o.desks();
    let mut out = plates(o, &desks);
    out.extend(world_labels(o, &desks, coordinator.as_ref()));
    out.push(top_bar(s, &desks));
    let narrow = is_narrow(o);
    if !narrow {
        out.extend(now_happening(o, &desks, coordinator.as_ref()));
    }
    out.push(roster(s, &desks, coordinator.as_ref()));
    out.extend(inspector(o, &desks, coordinator.as_ref()));
    if !narrow {
        out.extend(jev_panel(o));
        out.extend(ticker(o));
    }
    out
}
