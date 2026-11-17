//! Agentes: el mapa de la colmena y la configuración de cada nodo. Mismo diseño que `AgentsPage` de la web.

use crate::ui::text;
use crate::ui::Tap;
use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, text_input, Color, El, LineCap, ScrollSpec, Shape};

use super::icons;
use super::theme as th;
use crate::settings::api::{core_id, extra_coordinators, Agent};
use crate::settings::{Action, AgentForm, NewAgent};
use crate::State;

/// Separación entre hexágonos vecinos del mapa (puntos).
const DX: f32 = 123.5;
const DY: f32 = 138.0;
const HEX_W: f32 = 126.0;
const HEX_H: f32 = 108.0;
/// Lo que se separa cada vecino de la coordinadora (su hexágono es más grande): la web deja 19 puntos más en vertical.
const CORE_PUSH: f32 = 19.0;
const CORE_W: f32 = 170.0;
const CORE_H: f32 = 147.0;
/// Hexágonos que caben: el coordinador, el primer anillo (6) y el segundo (12).
pub const MAX_NODES: usize = 18;

/// Posiciones de los especialistas alrededor del centro, del primer anillo hacia fuera y en el sentido
/// de las agujas del reloj empezando por arriba (la malla es la de la web: vecinos a (0, ±130) y (±123, ±65)).
pub fn ring_positions(n: usize) -> Vec<(f32, f32)> {
    let dirs: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let at = |q: i32, r: i32| {
        let y = r as f32 * DY + q as f32 * DY / 2.0;
        (
            q as f32 * DX,
            y + y.signum() * CORE_PUSH * (y.abs() / DY).min(1.0),
        )
    };
    let mut out: Vec<(f32, f32)> = vec![];
    for ring in 1..=3i32 {
        let (mut q, mut r) = (dirs[4].0 * ring, dirs[4].1 * ring); // se empieza en una esquina y se rodea
        let mut cells = vec![];
        for d in dirs {
            for _ in 0..ring {
                cells.push(at(q, r));
                q += d.0;
                r += d.1;
            }
        }
        // De arriba en sentido horario: el ángulo desde el norte.
        cells.sort_by(|a, b| {
            let ang =
                |p: &(f32, f32)| (p.0.atan2(-p.1) + std::f32::consts::TAU) % std::f32::consts::TAU;
            ang(a).partial_cmp(&ang(b)).unwrap()
        });
        let missing = n.saturating_sub(out.len());
        if missing > 0 && missing < cells.len() {
            // Un anillo a medias se reparte parejo (con el primero arriba) en vez de apilarse en un lado.
            let c = cells.len();
            let picked: Vec<(f32, f32)> = (0..missing).map(|i| cells[i * c / missing]).collect();
            out.extend(picked);
        } else {
            out.extend(cells);
        }
        if out.len() >= n {
            break;
        }
    }
    out.truncate(n);
    out
}

fn hex(w: f32, h: f32, fill: Color, stroke: Color, lw: f32, glow: bool) -> Shape {
    let pad = 5.0;
    let pts = vec![
        [w * 0.25, pad],
        [w * 0.75, pad],
        [w - pad, h / 2.0],
        [w * 0.75, h - pad],
        [w * 0.25, h - pad],
        [pad, h / 2.0],
    ];
    let mut sh = Shape::new();
    if glow {
        // El resplandor del nodo elegido: el mismo contorno, ancho y casi transparente.
        sh = sh.stroke_with(pts.clone(), true, 9.0, LineCap::Round, stroke.alpha(0.22));
    }
    sh.fill(pts.clone(), fill)
        .stroke_with(pts, true, lw, LineCap::Round, stroke)
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>().trim_end())
    }
}

/// El agente que se está mirando: el elegido, y si no, el coordinador, y si no, el primero.
pub fn current<'a>(agents: &'a [Agent], selected: Option<&str>) -> Option<&'a Agent> {
    selected
        .and_then(|id| agents.iter().find(|a| a.id == id))
        .or_else(|| {
            agents
                .iter()
                .find(|a| Some(a.id.as_str()) == core_id(agents))
        })
        .or_else(|| agents.first())
}

fn select(s: &mut State, id: &str) {
    s.agents_sel = Some(id.to_string());
    if s.agents_editing {
        if let Some(a) = s.settings.shown.agents.iter().find(|a| a.id == id).cloned() {
            s.settings.agent_form = Some(AgentForm::from(&a));
        }
    }
}

fn start_edit(s: &mut State) {
    let Some(a) = current(&s.settings.shown.agents, s.agents_sel.as_deref()).cloned() else {
        return;
    };
    s.settings.agent_form = Some(AgentForm::from(&a));
    s.agents_editing = true;
}

fn card() -> El {
    col()
        .radius(16.0)
        .fill(th::card())
        .border(1.0, th::border_subtle())
}

fn caption(t: &str, color: Color) -> El {
    text(t.to_uppercase())
        .font(TextStyle::new(9.5, 700.0).mono().tracking(1.8))
        .color(color)
}

fn hero(s: &State) -> El {
    let agents = &s.settings.shown.agents;
    let active = agents.iter().filter(|a| a.enabled).count();
    // Un solo coordinador: todo lo demás es especialista.
    let specialists = agents.len() - usize::from(core_id(agents).is_some());
    let extras: Vec<String> = extra_coordinators(agents)
        .iter()
        .map(|a| a.name.clone())
        .collect();
    let stat = |n: usize, label: &str, tone: Color| {
        row()
            .gap(6.0)
            .items_end()
            .child(
                text(n.to_string())
                    .font(TextStyle::new(22.0, 700.0))
                    .color(tone),
            )
            .child(
                text(label.to_uppercase())
                    .font(TextStyle::new(9.0, 600.0).tracking(1.2))
                    .color(th::text_muted()),
            )
    };
    let sync = row()
        .key("agents/sync")
        .gap(8.0)
        .center_y()
        .px(14.0)
        .py(10.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(rune::icon(&icons::REFRESH, 14.0).color(th::muted_foreground()))
        .child(
            text(if s.settings.shown.busy > 0 {
                "Sincronizando…"
            } else {
                "Sincronizar"
            })
            .font(type_::LABEL)
            .color(th::foreground()),
        )
        .tap(|s: &mut State| s.settings.dispatch(Action::Refresh));
    let deploy = row()
        .key("agents/deploy")
        .gap(8.0)
        .center_y()
        .px(16.0)
        .py(10.0)
        .radius(10.0)
        .fill(th::amber())
        .child(rune::icon(&icons::PLUS, 14.0).color(th::on_amber()))
        .child(
            text("Desplegar nodo")
                .font(TextStyle::new(13.0, 700.0))
                .color(th::on_amber()),
        )
        .tap(|s: &mut State| {
            s.settings.new_agent = Some(NewAgent::default());
            s.agents_editing = false;
            s.agents_detail = false;
        });
    // Aviso si el servidor tiene más de un coordinador (p. ej. por crear agentes con el `POST` de REST): se corrige a mano.
    let warning = (!extras.is_empty()).then(|| {
        row().key("agents/extra-coordinators").w_full().px(14.0).py(10.0).gap(12.0).center_y().radius(10.0).fill(th::amber().alpha(0.1)).border(1.0, th::amber().alpha(0.35))
            .child(text(format!("Solo puede haber un coordinador: {} figura{} también como coordinador{}. Se pasarán a especialista.", extras.join(", "), if extras.len() == 1 { "" } else { "n" }, if extras.len() == 1 { "" } else { "es" })).font(type_::LABEL).color(th::foreground()).wrap().grow(1.0))
            .child(
                row().key("agents/fix-coordinators").px(14.0).py(7.0).radius(8.0).fill(th::amber()).center()
                    .child(text("Corregir").font(TextStyle::new(12.0, 700.0)).color(th::on_amber()))
                    .tap(|s: &mut State| s.settings.dispatch(Action::DemoteExtraCoordinators)),
            )
    });
    card().key("agents/hero").w_full().px(24.0).py(20.0).gap(8.0)
        .child(
            (if s.content_width()<920.0 {col()}else{row()}).w_full().gap(16.0).flex_wrap().justify_between()
                .child(
                    col().shrink().grow(1.0).gap(8.0)
                        .child(row().gap(8.0).center_y().child(rune::el().size(6.0, 6.0).radius(3.0).fill(th::blue())).child(text("NEXO CENTRAL · CONFIGURACIÓN DE AGENTES").font(TextStyle::new(9.5, 700.0).mono().tracking(1.8)).color(th::blue())))
                        .child(row().w_full().gap(10.0).flex_wrap().items_end()
                            .child(text("Tu colmena").font(TextStyle::new(34.0, 800.0)).color(th::foreground()))
                            .child(text("inteligente").font(TextStyle::new(34.0, 800.0)).color(th::amber())))
                        .child(text("Selecciona un nodo para revisar su configuración, ajustar su propósito o entrar a su espacio operativo.").font(type_::BODY.lh(20.0)).color(th::muted_foreground())),
                )
                .child(
                    (if s.content_width()<920.0 {row().w_full().flex_wrap().justify_between().center_y()}else{col().items_end()}).gap(14.0)
                        .child(row().gap(20.0).child(stat(active, "activos", th::green())).child(stat(specialists, "especialistas", th::blue())))
                        .child(row().gap(10.0).child(sync).child(deploy)),
                ),
        )
        .children(warning)
}

fn info_box(icon: &'static rune::Icon, label: &str, value: String, mono: bool) -> El {
    let style = if mono {
        TextStyle::new(11.5, 400.0).mono()
    } else {
        TextStyle::new(13.0, 500.0)
    };
    row()
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(12.0)
        .center_y()
        .radius(10.0)
        .fill(th::glass())
        .border(1.0, th::border_base())
        .child(rune::icon(icon, 15.0).color(th::muted_foreground()))
        .child(
            col()
                .gap(2.0)
                .grow(1.0)
                .min_w(0.0)
                .child(caption(label, th::text_muted()))
                .child(text(value).font(style).color(th::foreground())),
        )
}

fn stat_box(n: String, label: &str, tone: Color) -> El {
    col()
        .grow(1.0)
        .w(0.0)
        .py(10.0)
        .center()
        .gap(2.0)
        .radius(10.0)
        .fill(th::glass())
        .border(1.0, th::border_base())
        .child(text(n).font(TextStyle::new(18.0, 700.0)).color(tone))
        .child(
            text(label.to_uppercase())
                .font(TextStyle::new(8.5, 600.0).tracking(1.0))
                .color(th::text_muted()),
        )
}

fn node_card(s: &State, a: &Agent) -> El {
    let coord = core_id(&s.settings.shown.agents) == Some(a.id.as_str());
    let accent = if coord { th::amber() } else { th::blue() };
    let provider = s
        .settings
        .shown
        .providers
        .iter()
        .find(|p| p.id == a.provider_id)
        .map(|p| {
            if p.name.is_empty() {
                p.id.clone()
            } else {
                p.name.clone()
            }
        })
        .unwrap_or_else(|| {
            if a.provider_id.is_empty() {
                "—".into()
            } else {
                a.provider_id.clone()
            }
        });
    let status = if a.enabled {
        ("ACTIVO", th::green())
    } else {
        ("INACTIVO", th::muted_foreground())
    };
    let desc = if a.description.is_empty() {
        "Sin descripción.".to_string()
    } else {
        short(&a.description, 150)
    };
    card()
        .key("agents/node")
        .w(if s.content_width() < 920.0 {
            (s.content_width() - 32.0).max(0.0)
        } else {
            300.0
        })
        .padding(0.0)
        .clip_rounded(16.0)
        .child(col().w_full().h(3.0).fill(accent))
        .child(
            col()
                .w_full()
                .padding(18.0)
                .gap(12.0)
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(
                            row()
                                .size(44.0, 44.0)
                                .radius(12.0)
                                .center()
                                .fill(accent.alpha(0.12))
                                .border(1.0, accent.alpha(0.3))
                                .child(
                                    rune::icon(
                                        if coord { &icons::SHIELD } else { &icons::CPU },
                                        20.0,
                                    )
                                    .color(accent),
                                ),
                        )
                        .child(
                            row()
                                .gap(6.0)
                                .center_y()
                                .px(10.0)
                                .py(3.0)
                                .radius(10.0)
                                .fill(status.1.alpha(0.12))
                                .border(1.0, status.1.alpha(0.35))
                                .child(rune::el().size(5.0, 5.0).radius(2.5).fill(status.1))
                                .child(
                                    text(status.0)
                                        .font(TextStyle::new(9.0, 700.0).tracking(1.0))
                                        .color(status.1),
                                ),
                        ),
                )
                .child(caption(
                    if coord {
                        "Nodo coordinador"
                    } else {
                        "Nodo especialista"
                    },
                    accent,
                ))
                .child(
                    text(a.name.clone())
                        .font(TextStyle::new(22.0, 700.0))
                        .color(th::foreground())
                        .wrap(),
                )
                .child(
                    text(desc)
                        .font(type_::BODY.lh(19.0))
                        .color(th::muted_foreground())
                        .wrap(),
                )
                .child(info_box(&icons::BRAIN, "Motor", provider, false))
                .child(info_box(
                    &icons::SPARKLES,
                    "Modelo",
                    if a.model_id.is_empty() {
                        "—".into()
                    } else {
                        a.model_id.clone()
                    },
                    false,
                ))
                .child(info_box(
                    &icons::FOLDER,
                    "Workspace",
                    if a.workspace.is_empty() {
                        "—".into()
                    } else {
                        a.workspace.clone()
                    },
                    true,
                ))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(stat_box(a.tools().len().to_string(), "Tools", th::green()))
                        .child(stat_box(
                            a.skills().len().to_string(),
                            "Skills",
                            th::amber(),
                        ))
                        .child(stat_box(
                            a.max_iterations.map_or("—".into(), |n| n.to_string()),
                            "Iters",
                            th::purple(),
                        )),
                )
                .child(
                    row()
                        .key("agents/configure")
                        .w_full()
                        .py(11.0)
                        .gap(8.0)
                        .center()
                        .radius(10.0)
                        .fill(th::amber())
                        .child(rune::icon(&icons::PENCIL, 14.0).color(th::on_amber()))
                        .child(
                            text(if s.agents_editing {
                                "Editando…"
                            } else {
                                "Configurar nodo"
                            })
                            .font(TextStyle::new(13.0, 700.0))
                            .color(th::on_amber()),
                        )
                        .tap(|s: &mut State| start_edit(s)),
                )
                .child(
                    row()
                        .key("agents/profile")
                        .w_full()
                        .py(10.0)
                        .gap(8.0)
                        .center()
                        .radius(10.0)
                        .fill(th::glass_strong())
                        .border(1.0, th::border_base())
                        .child(
                            text("Abrir perfil")
                                .font(TextStyle::new(13.0, 500.0))
                                .color(th::foreground()),
                        )
                        .child(
                            rune::icon(&icons::EXTERNAL_LINK, 13.0).color(th::muted_foreground()),
                        )
                        .tap(|s: &mut State| s.agents_detail = true),
                ),
        )
}

/// El nombre del agente en una o dos líneas centradas (se parte por el espacio más cercano a la mitad; los guiones bajos son espacios).
pub fn split_name(name: &str) -> Vec<String> {
    let n: String = name.replace('_', " ").trim().to_uppercase();
    if n.chars().count() <= 14 {
        return vec![n];
    }
    let mid = n.chars().count() / 2;
    let cut = n
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .min_by_key(|(i, _)| (n[..*i].chars().count() as i32 - mid as i32).abs())
        .map(|(i, _)| i);
    match cut {
        Some(i) => vec![short(n[..i].trim(), 16), short(n[i..].trim(), 16)],
        None => vec![short(&n, 16)],
    }
}

fn name_lines(name: &str, coord: bool) -> El {
    let style = TextStyle::new(if coord { 12.0 } else { 8.5 }, 700.0).tracking(0.5);
    let tone = if coord {
        th::amber()
    } else {
        th::blue().alpha(0.95)
    };
    split_name(name)
        .into_iter()
        .fold(col().items_center().gap(0.0), |c, l| {
            c.child(text(l).font(style).color(tone))
        })
}

fn node_hex(a: &Agent, coord: bool, selected: bool, x: f32, y: f32) -> El {
    let (w, h) = if coord {
        (CORE_W, CORE_H)
    } else {
        (HEX_W, HEX_H)
    };
    let accent = if coord { th::amber() } else { th::blue() };
    let fill = if th::is_light() {
        if coord {
            th::hsla(40.0, 85.0, 93.0, 1.0)
        } else {
            th::hsla(220.0, 60.0, 95.0, 1.0)
        }
    } else if coord {
        th::hsla(40.0, 60.0, 9.0, 1.0)
    } else {
        th::hsla(220.0, 45.0, 7.0, 1.0)
    };
    let (tools, skills) = (a.tools().len(), a.skills().len());
    let key = format!("agents/hex/{}", a.id);
    let inner = col()
        .w_full()
        .h_full()
        .center()
        .gap(3.0)
        .px(18.0)
        .child(
            rune::icon(
                if coord { &icons::SHIELD } else { &icons::CPU },
                if coord { 26.0 } else { 20.0 },
            )
            .color(accent),
        )
        .child(name_lines(&a.name, coord))
        .child(if coord {
            text("COORDINADORA")
                .font(TextStyle::new(8.0, 600.0).tracking(1.4))
                .color(th::amber().alpha(0.7))
        } else {
            text(format!("{tools} tools · {skills} skills"))
                .font(TextStyle::new(8.0, 500.0))
                .color(th::text_muted())
        });
    col()
        .key(key)
        .absolute(x - w / 2.0, y - h / 2.0)
        .size(w, h)
        .shape(hex(
            w,
            h,
            fill,
            accent.alpha(if selected { 1.0 } else { 0.55 }),
            if selected { 2.5 } else { 1.5 },
            selected,
        ))
        .child(inner)
        .tap({
            let id = a.id.clone();
            move |s: &mut State| select(s, &id)
        })
}

/// Cuánto se amplía el mapa para llenar el hueco de la ventana (a pasos de 0,05 para que no tiemble al redimensionar).
pub fn map_scale(s: &State, w: f32, h: f32) -> f32 {
    // Hueco de la tarjeta del mapa: la ventana menos la barra, el panel de la izquierda (300 + 16), márgenes y relleno;
    // y de alto, menos la cabecera de la app, la tarjeta de presentación (~190), la cabecera del mapa (~70) y márgenes.
    let avail_w = s.content_width()
        - 32.0
        - if s.content_width() < 920.0 {
            24.0
        } else {
            316.0 + 24.0
        };
    // Solo manda el ancho: la página se desplaza en vertical como la de la web, así que el mapa no se encoge para caber a lo alto.
    let _ = h;
    let k = (avail_w / w).clamp(0.2, 1.05);
    (k * 20.0).floor() / 20.0
}

fn hive_map(s: &State, current_id: Option<&str>) -> El {
    let agents = &s.settings.shown.agents;
    let core_id = core_id(agents);
    let core = agents
        .iter()
        .find(|a| Some(a.id.as_str()) == core_id)
        .or_else(|| agents.first());
    let others: Vec<&Agent> = agents
        .iter()
        .filter(|a| Some(a.id.as_str()) != core.map(|c| c.id.as_str()))
        .take(MAX_NODES - 1)
        .collect();
    let pos = ring_positions(others.len());
    // El lienzo se ajusta a lo que hay dibujado (no a anillos enteros), con el centro donde le toca.
    let (mut x0, mut x1, mut y0, mut y1) =
        (-CORE_W / 2.0, CORE_W / 2.0, -CORE_H / 2.0, CORE_H / 2.0);
    for (x, y) in &pos {
        x0 = x0.min(x - HEX_W / 2.0);
        x1 = x1.max(x + HEX_W / 2.0);
        y0 = y0.min(y - HEX_H / 2.0);
        y1 = y1.max(y + HEX_H / 2.0);
    }
    let pad = 16.0;
    let (w, h) = (x1 - x0 + 2.0 * pad, y1 - y0 + 2.0 * pad);
    let (cx, cy) = (pad - x0, pad - y0);
    let mut map = col().key("agents/map").size(w, h);
    for (a, (x, y)) in others.iter().zip(pos) {
        map = map.child(node_hex(
            a,
            false,
            current_id == Some(a.id.as_str()),
            cx + x,
            cy + y,
        ));
    }
    if let Some(c) = core {
        map = map.child(node_hex(c, true, current_id == Some(c.id.as_str()), cx, cy));
    }
    let legend = |tone: Color, label: &str| {
        row()
            .gap(6.0)
            .center_y()
            .child(rune::el().size(6.0, 6.0).radius(3.0).fill(tone))
            .child(
                text(label.to_uppercase())
                    .font(TextStyle::new(9.0, 600.0).tracking(1.2))
                    .color(th::text_muted()),
            )
    };
    let head = row()
        .w_full()
        .px(18.0)
        .py(14.0)
        .gap(10.0)
        .flex_wrap()
        .justify_between()
        .center_y()
        .border(1.0, th::border_subtle())
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(
                    row()
                        .size(34.0, 34.0)
                        .radius(9.0)
                        .center()
                        .fill(th::blue().alpha(0.1))
                        .border(1.0, th::blue().alpha(0.2))
                        .child(rune::icon(&icons::NETWORK, 16.0).color(th::blue())),
                )
                .child(
                    col()
                        .gap(2.0)
                        .child(
                            text("MAPA DE LA COLMENA")
                                .font(TextStyle::new(12.0, 700.0).tracking(2.0))
                                .color(th::foreground()),
                        )
                        .child(
                            text("Haz clic en un hexágono para inspeccionarlo")
                                .font(TextStyle::new(11.0, 400.0))
                                .color(th::text_muted()),
                        ),
                ),
        )
        .child(
            row()
                .gap(16.0)
                .child(legend(th::amber(), "Coordinación"))
                .child(legend(th::blue(), "Especialistas")),
        );
    // El mapa crece o se encoge con la ventana (hasta 0,7× y 1,7×): `scale` no cambia el hueco que ocupa en el diseño, así que
    // se reserva el tamaño ya escalado y se coloca el mapa centrado dentro.
    let k = map_scale(s, w, h);
    let fitted = col()
        .size(w * k, h * k)
        .child(map.absolute((w * k - w) / 2.0, (h * k - h) / 2.0).scale(k));
    card()
        .key("agents/map-card")
        .w_full()
        .grow(1.0)
        .min_w(0.0)
        .min_h(540.0)
        .child(head)
        .child(col().w_full().padding(12.0).items_center().child(fitted))
}

fn area(edit: &rune::TextEdit, key: &str, pick: fn(&mut State) -> &mut rune::TextEdit) -> El {
    row()
        .key(key.to_string())
        .w_full()
        .h(190.0)
        .px(10.0)
        .py(8.0)
        .radius(radius::SM)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(
            text_input(edit, TextStyle::new(12.0, 400.0).mono().lh(18.0))
                .key(format!("{key}/input"))
                .grow(1.0)
                .color(th::foreground())
                .on_edit(pick),
        )
}

fn editor(s: &State) -> El {
    let Some(f) = &s.settings.agent_form else {
        return col();
    };
    use crate::settings::view::field;
    let labelled = |name: &str, el: El| {
        col()
            .w_full()
            .gap(5.0)
            .child(caption(name, th::text_muted()))
            .child(el)
    };
    let on = f.enabled;
    let save = row()
        .key("agents/save")
        .px(18.0)
        .py(10.0)
        .radius(10.0)
        .fill(th::amber())
        .center()
        .child(
            text("Guardar")
                .font(TextStyle::new(13.0, 700.0))
                .color(th::on_amber()),
        )
        .tap(|s: &mut State| s.settings.dispatch(Action::SaveAgentForm));
    let cancel = row()
        .key("agents/cancel")
        .px(18.0)
        .py(10.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .center()
        .child(
            text("Volver al mapa")
                .font(TextStyle::new(13.0, 500.0))
                .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            s.agents_editing = false;
            s.settings.agent_form = None;
        });
    let toggle = row()
        .key("agents/enabled")
        .gap(10.0)
        .center_y()
        .child(
            row()
                .size(38.0, 22.0)
                .radius(11.0)
                .fill(if on { th::green() } else { th::muted() })
                .child(
                    rune::el()
                        .size(18.0, 18.0)
                        .radius(9.0)
                        .fill(Color::WHITE)
                        .absolute(if on { 18.0 } else { 2.0 }, 2.0),
                ),
        )
        .child(
            text(if on {
                "Agente activo"
            } else {
                "Agente apagado"
            })
            .font(type_::BODY)
            .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            if let Some(f) = s.settings.agent_form.as_mut() {
                f.enabled = !f.enabled;
            }
        });
    let mut c = card().key("agents/editor").grow(1.0).min_w(0.0).padding(22.0).gap(14.0)
        .child(row().gap(12.0).center_y().child(row().size(34.0, 34.0).radius(9.0).center().fill(th::amber().alpha(0.12)).border(1.0, th::amber().alpha(0.3)).child(rune::icon(&icons::PENCIL, 16.0).color(th::amber())))
            .child(col().shrink().gap(2.0).child(text("CONFIGURAR NODO").font(TextStyle::new(12.0, 700.0).tracking(2.0)).color(th::foreground())).child(text("Los cambios se guardan en el gateway y valen para las próximas respuestas").font(TextStyle::new(11.0, 400.0)).color(th::text_muted()))))
        .child(labelled("Nombre", field(&f.name, "agents/f/name".into(), "Nombre del agente", |s: &mut State| &mut s.settings.agent_form.as_mut().expect("formulario").name)))
        .child(labelled("Descripción", field(&f.desc, "agents/f/desc".into(), "Qué hace este agente", |s: &mut State| &mut s.settings.agent_form.as_mut().expect("formulario").desc)))
        .child(labelled("Prompt del sistema", area(&f.prompt, "agents/f/prompt", |s: &mut State| &mut s.settings.agent_form.as_mut().expect("formulario").prompt)))
        .child(row().w_full().gap(14.0)
            .child(col().grow(1.0).w(0.0).child(labelled("Tono", field(&f.tone, "agents/f/tone".into(), "friendly, professional…", |s: &mut State| &mut s.settings.agent_form.as_mut().expect("formulario").tone))))
            .child(col().grow(1.0).w(0.0).child(labelled("Máx. iteraciones", field(&f.iters, "agents/f/iters".into(), "1 – 200", |s: &mut State| &mut s.settings.agent_form.as_mut().expect("formulario").iters)))))
        .child(toggle)
        .child(row().gap(10.0).child(save).child(cancel));
    if let Some(e) = &s.settings.shown.error {
        c = c.child(text(e.clone()).font(type_::LABEL).color(th::red()).wrap());
    }
    if let Some(n) = &s.settings.shown.notice {
        c = c.child(text(n.clone()).font(type_::LABEL).color(th::green()).wrap());
    }
    c.scroll(ScrollSpec {
        stick_bottom: false,
    })
}

/// Una lista desplegable en línea (como la de HiveLive): título, valor actual y, abierta, las opciones.
fn pick(
    s_open: bool,
    key: &'static str,
    name: &str,
    value: String,
    options: Vec<String>,
    toggle: fn(&mut State),
    choose: fn(&mut State, usize),
) -> El {
    let mut c = col()
        .w_full()
        .gap(5.0)
        .child(caption(name, th::text_muted()));
    let head = row()
        .key(key)
        .w_full()
        .px(12.0)
        .py(10.0)
        .radius(radius::SM)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .center_y()
        .child(
            text(value)
                .font(type_::BODY)
                .color(th::foreground())
                .grow(1.0),
        )
        .child(rune::icon(&icons::CHEVRON_DOWN, 14.0).color(th::muted_foreground()))
        .tap(toggle);
    c = c.child(head);
    if s_open {
        let mut list = col()
            .w_full()
            .radius(radius::SM)
            .fill(th::card())
            .border(1.0, th::border_base())
            .py(4.0);
        for (i, o) in options.into_iter().enumerate() {
            list = list.child(
                row()
                    .key(format!("{key}/{i}"))
                    .w_full()
                    .px(12.0)
                    .py(7.0)
                    .child(text(o).font(type_::BODY).color(th::foreground()))
                    .tap(move |s: &mut State| choose(s, i)),
            );
        }
        c = c.child(list);
    }
    c
}

fn create_form(s: &State) -> El {
    use crate::settings::view::field;
    let Some(f) = &s.settings.new_agent else {
        return col();
    };
    let providers = &s.settings.shown.providers;
    let usable = NewAgent::usable(providers);
    let prov = usable.get(f.provider);
    let models: Vec<&crate::settings::api::Model> = prov
        .map(|p| p.models.iter().filter(|m| m.for_agent()).collect())
        .unwrap_or_default();
    let prov_name = prov
        .map(|p| {
            if p.name.is_empty() {
                p.id.clone()
            } else {
                p.name.clone()
            }
        })
        .unwrap_or_else(|| "Selecciona un proveedor".into());
    let model_name = models
        .get(f.model)
        .map(|m| {
            if m.name.is_empty() {
                m.id.clone()
            } else {
                m.name.clone()
            }
        })
        .unwrap_or_else(|| "Selecciona un modelo".into());
    let labelled = |name: &str, el: El| {
        col()
            .w_full()
            .gap(5.0)
            .child(caption(name, th::text_muted()))
            .child(el)
    };
    let create = row()
        .key("agents/create")
        .px(20.0)
        .py(11.0)
        .radius(10.0)
        .fill(th::amber())
        .center()
        .child(
            text("Desplegar nodo")
                .font(TextStyle::new(13.0, 700.0))
                .color(th::on_amber()),
        )
        .tap(|s: &mut State| s.settings.dispatch(Action::CreateAgent));
    let cancel = row()
        .key("agents/new-cancel")
        .px(20.0)
        .py(11.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .center()
        .child(
            text("Cancelar")
                .font(TextStyle::new(13.0, 500.0))
                .color(th::foreground()),
        )
        .tap(|s: &mut State| s.settings.new_agent = None);
    let mut c = card()
        .key("agents/new")
        .w(640.0_f32.min((s.content_width() - 32.0).max(0.0)))
        .padding(26.0)
        .gap(16.0)
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(
                    row()
                        .size(40.0, 40.0)
                        .radius(10.0)
                        .center()
                        .fill(th::blue().alpha(0.1))
                        .border(1.0, th::blue().alpha(0.25))
                        .child(rune::icon(&icons::SPARKLES, 18.0).color(th::blue())),
                )
                .child(
                    col()
                        .gap(2.0)
                        .child(
                            text("Desplegar Nuevo Nodo")
                                .font(TextStyle::new(18.0, 700.0))
                                .color(th::foreground()),
                        )
                        .child(
                            text("Crea una nueva entidad inteligente para tu colmena.")
                                .font(type_::BODY)
                                .color(th::muted_foreground()),
                        ),
                ),
        )
        .child(labelled(
            "Nombre del agente",
            field(
                &f.name,
                "agents/n/name".into(),
                "Ej: Analista de Mercado",
                |s: &mut State| &mut s.settings.new_agent.as_mut().expect("formulario").name,
            ),
        ))
        .child(labelled(
            "Descripción / Rol",
            field(
                &f.desc,
                "agents/n/desc".into(),
                "Describe qué hace este agente y cuáles son sus objetivos...",
                |s: &mut State| &mut s.settings.new_agent.as_mut().expect("formulario").desc,
            ),
        ))
        .child(labelled(
            "System prompt",
            area(&f.prompt, "agents/n/prompt", |s: &mut State| {
                &mut s.settings.new_agent.as_mut().expect("formulario").prompt
            }),
        ))
        .child(labelled(
            "Directorio de trabajo del agente",
            field(
                &f.workspace,
                "agents/n/workspace".into(),
                "Ej: /home/user/proyectos",
                |s: &mut State| &mut s.settings.new_agent.as_mut().expect("formulario").workspace,
            ),
        ))
        .child(caption("Motor de inteligencia", th::blue()))
        .child(pick(
            f.open == 1,
            "agents/n/provider",
            "Proveedor de IA",
            prov_name,
            usable
                .iter()
                .map(|p| {
                    if p.name.is_empty() {
                        p.id.clone()
                    } else {
                        p.name.clone()
                    }
                })
                .collect(),
            |s| {
                if let Some(f) = s.settings.new_agent.as_mut() {
                    f.open = if f.open == 1 { 0 } else { 1 }
                }
            },
            |s, i| {
                if let Some(f) = s.settings.new_agent.as_mut() {
                    f.provider = i;
                    f.model = 0;
                    f.open = 0
                }
            },
        ))
        .child(pick(
            f.open == 2,
            "agents/n/model",
            "Modelo",
            model_name,
            models
                .iter()
                .map(|m| {
                    if m.name.is_empty() {
                        m.id.clone()
                    } else {
                        m.name.clone()
                    }
                })
                .collect(),
            |s| {
                if let Some(f) = s.settings.new_agent.as_mut() {
                    f.open = if f.open == 2 { 0 } else { 2 }
                }
            },
            |s, i| {
                if let Some(f) = s.settings.new_agent.as_mut() {
                    f.model = i;
                    f.open = 0
                }
            },
        ));
    if usable.is_empty() {
        c = c.child(text("No hay proveedores con clave activa: el agente usará el modelo predeterminado del servidor.").font(type_::LABEL).color(th::muted_foreground()).wrap());
    }
    if let Some(e) = &s.settings.shown.error {
        c = c.child(text(e.clone()).font(type_::LABEL).color(th::red()).wrap());
    }
    c.child(row().gap(10.0).child(create).child(cancel))
}

/// El perfil de un agente: identidad y lo que tiene asignado (la «loadout» de la web).
fn detail(s: &State, a: &Agent) -> El {
    let coord = core_id(&s.settings.shown.agents) == Some(a.id.as_str());
    let accent = if coord { th::amber() } else { th::blue() };
    let provider = s
        .settings
        .shown
        .providers
        .iter()
        .find(|p| p.id == a.provider_id)
        .map(|p| {
            if p.name.is_empty() {
                p.id.clone()
            } else {
                p.name.clone()
            }
        })
        .unwrap_or_else(|| a.provider_id.clone());
    let status = if a.enabled {
        ("Activo", th::green())
    } else {
        ("Apagado", th::muted_foreground())
    };
    let (tools, skills) = (a.tools(), a.skills());
    // La categoría de cada herramienta, si ya se leyó la lista de herramientas de Ajustes.
    let category = |name: &str| {
        s.settings
            .shown
            .tools
            .iter()
            .find(|t| t.id == name || t.name == name)
            .map(|t| t.category.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "herramienta".into())
    };
    let info = |icon: &'static rune::Icon, label: &str, value: String, sub: Option<String>| {
        col()
            .grow(1.0)
            .w(0.0)
            .px(16.0)
            .py(12.0)
            .gap(5.0)
            .radius(12.0)
            .fill(th::glass())
            .border(1.0, th::border_base())
            .child(
                row()
                    .gap(7.0)
                    .center_y()
                    .child(rune::icon(icon, 12.0).color(th::muted_foreground()))
                    .child(caption(label, th::text_muted())),
            )
            .child(
                text(value)
                    .font(TextStyle::new(14.0, 600.0))
                    .color(th::foreground())
                    .wrap(),
            )
            .child(match sub {
                Some(t) => text(t)
                    .font(TextStyle::new(10.5, 400.0).mono())
                    .color(th::text_muted())
                    .wrap(),
                None => col(),
            })
    };
    let back = row()
        .key("agents/back")
        .gap(8.0)
        .center_y()
        .child(rune::icon(&icons::CHEVRON_LEFT, 14.0).color(th::muted_foreground()))
        .child(
            text("VOLVER A LA COLMENA")
                .font(TextStyle::new(11.0, 600.0).tracking(1.8))
                .color(th::muted_foreground()),
        )
        .tap(|s: &mut State| s.agents_detail = false);
    let identity = card()
        .key("agents/identity")
        .w_full()
        .padding(24.0)
        .gap(14.0)
        .child(
            row()
                .w_full()
                .gap(14.0)
                .flex_wrap()
                .justify_between()
                .child(
                    row()
                        .shrink()
                        .grow(1.0)
                        .gap(14.0)
                        .center_y()
                        .child(
                            row()
                                .size(46.0, 46.0)
                                .radius(12.0)
                                .center()
                                .fill(accent.alpha(0.12))
                                .border(1.0, accent.alpha(0.3))
                                .child(rune::icon(&icons::FINGERPRINT, 22.0).color(accent)),
                        )
                        .child(
                            col()
                                .shrink()
                                .gap(3.0)
                                .child(caption("Identidad de nodo", accent))
                                .child(
                                    text(a.name.clone())
                                        .font(TextStyle::new(28.0, 800.0))
                                        .color(th::foreground())
                                        .wrap(),
                                ),
                        ),
                )
                .child(
                    col()
                        .gap(6.0)
                        .items_end()
                        .child(caption("Estado operativo", th::text_muted()))
                        .child(
                            row()
                                .px(18.0)
                                .py(7.0)
                                .radius(18.0)
                                .fill(status.1.alpha(0.12))
                                .border(1.0, status.1.alpha(0.4))
                                .child(
                                    text(status.0)
                                        .font(TextStyle::new(13.0, 600.0))
                                        .color(status.1),
                                ),
                        ),
                ),
        )
        .child(
            text(if a.description.is_empty() {
                "Sin descripción.".to_string()
            } else {
                format!("«{}»", a.description)
            })
            .font(TextStyle::new(14.0, 400.0).italic().lh(22.0))
            .color(th::muted_foreground())
            .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(14.0)
                .child(info(
                    &icons::CPU,
                    "Infraestructura",
                    provider,
                    Some(a.model_id.clone()).filter(|m| !m.is_empty()),
                ))
                .child(info(
                    &icons::SPARKLES,
                    "Personalidad",
                    if a.tone.is_empty() {
                        "—".into()
                    } else {
                        a.tone.clone()
                    },
                    None,
                ))
                .child(info(
                    &icons::ACTIVITY,
                    "Iteraciones máximas",
                    a.max_iterations.map_or("—".into(), |n| n.to_string()),
                    Some(a.role.clone()).filter(|r| !r.is_empty()),
                )),
        );
    let count = |n: usize, label: &str, tone: Color| {
        col()
            .gap(1.0)
            .items_end()
            .child(
                text(n.to_string())
                    .font(TextStyle::new(20.0, 700.0))
                    .color(tone),
            )
            .child(
                text(label.to_uppercase())
                    .font(TextStyle::new(8.5, 600.0).tracking(1.0))
                    .color(th::text_muted()),
            )
    };
    let mut grid = row().w_full().gap(12.0).flex_wrap();
    for t in &tools {
        grid = grid.child(
            row()
                .key(format!("agents/tool/{t}"))
                .w(250.0)
                .grow(1.0)
                .px(14.0)
                .py(10.0)
                .gap(10.0)
                .center_y()
                .radius(10.0)
                .fill(th::glass())
                .border(1.0, th::border_base())
                .child(rune::icon(&icons::WRENCH, 14.0).color(th::green()))
                .child(
                    col()
                        .gap(2.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(t.clone())
                                .font(TextStyle::new(12.5, 600.0).mono())
                                .color(th::foreground()),
                        )
                        .child(
                            text(category(t).to_uppercase())
                                .font(TextStyle::new(8.5, 600.0).tracking(1.0))
                                .color(th::green()),
                        ),
                ),
        );
    }
    let mut sk = row().w_full().gap(8.0).flex_wrap();
    for k in &skills {
        sk = sk.child(
            row()
                .key(format!("agents/skill/{k}"))
                .px(12.0)
                .py(6.0)
                .radius(14.0)
                .fill(th::amber().alpha(0.12))
                .border(1.0, th::amber().alpha(0.3))
                .child(
                    text(k.clone())
                        .font(TextStyle::new(11.5, 500.0).mono())
                        .color(th::amber()),
                ),
        );
    }
    let loadout = card()
        .key("agents/loadout")
        .w_full()
        .padding(22.0)
        .gap(14.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .gap(12.0)
                        .center_y()
                        .child(
                            row()
                                .size(34.0, 34.0)
                                .radius(9.0)
                                .center()
                                .fill(th::green().alpha(0.12))
                                .border(1.0, th::green().alpha(0.3))
                                .child(rune::icon(&icons::WRENCH, 16.0).color(th::green())),
                        )
                        .child(
                            col()
                                .gap(2.0)
                                .child(
                                    text("LOADOUT OPERATIVO")
                                        .font(TextStyle::new(13.0, 700.0).tracking(2.0))
                                        .color(th::foreground()),
                                )
                                .child(
                                    text("Tools y skills asignadas para ejecutar tareas")
                                        .font(TextStyle::new(11.0, 400.0))
                                        .color(th::text_muted()),
                                ),
                        ),
                )
                .child(
                    row()
                        .gap(22.0)
                        .child(count(tools.len(), "Tools", th::green()))
                        .child(count(skills.len(), "Skills", th::amber())),
                ),
        )
        .child(caption("Tools disponibles", th::green()))
        .children(tools.is_empty().then(|| {
            text("Este agente no tiene herramientas asignadas.")
                .font(type_::BODY)
                .color(th::muted_foreground())
        }))
        .child(grid)
        .child(caption("Skills", th::amber()))
        .children(skills.is_empty().then(|| {
            text("Este agente no tiene skills asignadas.")
                .font(type_::BODY)
                .color(th::muted_foreground())
        }))
        .child(sk);
    col()
        .w_full()
        .padding(0.0)
        .gap(16.0)
        .child(back)
        .child(identity)
        .child(loadout)
}

pub fn view(s: &State) -> El {
    let agents = &s.settings.shown.agents;
    let cur = current(agents, s.agents_sel.as_deref());
    // «Desplegar nodo» y el perfil ocupan la pantalla entera; el mapa y el editor, no.
    if s.settings.new_agent.is_some() {
        return col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(16.0)
            .items_center()
            .child(create_form(s))
            .scroll(ScrollSpec {
                stick_bottom: false,
            });
    }
    if s.agents_detail {
        if let Some(a) = cur {
            return col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .padding(16.0)
                .child(detail(s, a))
                .scroll(ScrollSpec {
                    stick_bottom: false,
                });
        }
    }
    let body = match cur {
        None => col()
            .w_full()
            .grow(1.0)
            .center()
            .gap(8.0)
            .child(rune::icon(&icons::BOT, 30.0).color(th::muted_foreground()))
            .child(
                text(if s.settings.shown.loaded {
                    "No hay agentes"
                } else {
                    "Cargando agentes…"
                })
                .font(type_::BODY)
                .color(th::muted_foreground()),
            ),
        Some(a) => (if s.content_width() < 920.0 {
            col()
        } else {
            row()
        })
        .w_full()
        .gap(16.0)
        .child(node_card(s, a))
        .child(if s.agents_editing {
            editor(s)
        } else {
            hive_map(s, Some(a.id.as_str()))
        }),
    };
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .padding(16.0)
        .gap(16.0)
        .child(hero(s))
        .child(body)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_ring_has_six_neighbours_at_the_webs_distances() {
        let p = ring_positions(6);
        assert_eq!(p.len(), 6);
        // Arriba del todo (0, -157), y los demás a (±123,5, ±78) y (0, +157).
        assert!(
            (p[0].0).abs() < 0.01 && (p[0].1 + DY + CORE_PUSH).abs() < 0.01,
            "el primero va arriba: {:?}",
            p[0]
        );
        for (x, y) in &p {
            let d = (x * x + y * y).sqrt();
            assert!(
                d > 120.0 && d < 165.0,
                "todos a una distancia parecida del centro ({d})"
            );
        }
        let mut xs: Vec<i32> = p.iter().map(|q| (q.0 * 2.0).round() as i32).collect();
        xs.sort();
        assert_eq!(xs, [-247, -247, 0, 0, 247, 247]);
    }

    #[test]
    fn the_second_ring_continues_without_overlapping_the_first() {
        let p = ring_positions(18);
        assert_eq!(p.len(), 18);
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                let d = ((p[i].0 - p[j].0).powi(2) + (p[i].1 - p[j].1).powi(2)).sqrt();
                assert!(d > 118.0, "los hexágonos {i} y {j} se tocan ({d})");
            }
        }
        assert!(ring_positions(0).is_empty());
        assert_eq!(ring_positions(3).len(), 3);
    }

    #[test]
    fn the_current_agent_is_the_selected_one_then_the_coordinator_then_the_first() {
        let mk = |id: &str, role: &str| Agent {
            id: id.into(),
            role: role.into(),
            ..Default::default()
        };
        let list = vec![
            mk("w1", "worker"),
            mk("boss", "coordinator"),
            mk("w2", "worker"),
        ];
        assert_eq!(current(&list, Some("w2")).unwrap().id, "w2");
        assert_eq!(current(&list, None).unwrap().id, "boss");
        assert_eq!(current(&list, Some("ya no existe")).unwrap().id, "boss");
        assert_eq!(current(&[mk("solo", "worker")], None).unwrap().id, "solo");
        assert!(current(&[], None).is_none());
    }

    #[test]
    fn long_descriptions_are_shortened_with_an_ellipsis() {
        assert_eq!(short("corto", 10), "corto");
        assert_eq!(short("abcdefghij", 5), "abcde…");
    }
}
