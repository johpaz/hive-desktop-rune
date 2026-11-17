//! Panel interactivo: las superficies A2UI que crean los agentes, en tarjetas con su rejilla y su papelera.
//! Mismo diseño que `A2UIPage` y `A2UISurfacePanel` de la web.

use crate::ui::text;
use crate::ui::Tap;
use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, El, ScrollSpec};
use serde_json::Value;

use super::icons;
use super::theme as th;
use crate::State;

/// Texto de la cabecera de una superficie: el nombre del agente que la creó o, si no hay, su id.
pub fn surface_title(theme: &Value, id: &str) -> (String, bool) {
    match theme
        .get("agentDisplayName")
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
    {
        Some(name) => (name.to_string(), true),
        None => (id.to_string(), false),
    }
}

/// `1 superficie A2UI activa` / `2 superficies A2UI activas` / el texto de panel vacío.
pub fn footer_label(n: usize) -> String {
    match n {
        0 => "Panel interactivo · sin superficies".into(),
        1 => "1 superficie A2UI activa".into(),
        n => format!("{n} superficies A2UI activas"),
    }
}

/// Borra la superficie en el servidor (como la papelera de la web) y la quita de la vista al instante.
pub fn delete_surface(s: &mut State, id: &str) {
    s.a2.apply("a2ui:deleteSurface", &serde_json::json!({"surfaceId": id}));
    let Some(conn) = s.settings.conn.clone() else {
        return;
    };
    let (id, data) = (id.to_string(), s.settings.data.clone());
    std::thread::spawn(move || {
        if let Err(e) = crate::settings::api::delete_surface(&conn, &id) {
            data.lock().unwrap().error = Some(format!(
                "No se pudo borrar la superficie en el servidor: {e}"
            ));
        }
    });
}

fn live_chip(connected: bool) -> El {
    let (tone, label, icon) = if connected {
        (th::green(), "LIVE", &icons::RADIO)
    } else {
        (th::red(), "OFFLINE", &icons::RADIO)
    };
    row()
        .gap(6.0)
        .center_y()
        .px(12.0)
        .py(4.0)
        .radius(14.0)
        .fill(tone.alpha(0.15))
        .border(1.0, tone.alpha(0.25))
        .child(rune::icon(icon, 13.0).color(tone))
        .child(
            text(label)
                .font(TextStyle::new(10.0, 700.0).tracking(1.2))
                .color(tone),
        )
}

pub fn view(s: &State) -> El {
    let surfaces = &s.a2.store.surfaces;
    let session = crate::session_id().unwrap_or_default();
    let header = row()
        .w_full()
        .px(24.0)
        .py(16.0)
        .justify_between()
        .center_y()
        .fill(Color_white(0.03))
        .border(1.0, th::border_subtle())
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(
                    row()
                        .size(38.0, 38.0)
                        .radius(10.0)
                        .center()
                        .fill(th::blue().alpha(0.12))
                        .border(1.0, th::blue().alpha(0.2))
                        .child(rune::icon(&icons::LAYERS, 19.0).color(th::blue())),
                )
                .child(
                    col()
                        .gap(3.0)
                        .child(
                            text("Panel interactivo")
                                .font(TextStyle::new(17.0, 700.0))
                                .color(th::foreground()),
                        )
                        .child(
                            text(session)
                                .font(TextStyle::new(11.0, 400.0).mono())
                                .color(th::text_muted()),
                        ),
                ),
        )
        .child(live_chip(s.chat_ready));

    let mut grid = row().w_full().gap(24.0).flex_wrap();
    for sf in surfaces {
        let (title, named) = surface_title(&sf.theme, &sf.id);
        let id = sf.id.clone();
        let head = row()
            .w_full()
            .justify_between()
            .center_y()
            .pb(10.0)
            .border(1.0, th::border_subtle())
            .child(if named {
                text(title.to_uppercase())
                    .font(TextStyle::new(11.5, 700.0).tracking(1.0))
                    .color(th::blue())
            } else {
                text(title)
                    .font(TextStyle::new(10.0, 400.0).mono())
                    .color(th::text_muted())
            })
            .child(
                row()
                    .key(format!("panel/delete/{id}"))
                    .size(28.0, 28.0)
                    .radius(7.0)
                    .center()
                    .child(rune::icon(&icons::TRASH, 14.0).color(th::muted_foreground()))
                    .tap(move |s: &mut State| delete_surface(s, &id)),
            );
        grid = grid.child(
            col()
                .key(format!("panel/surface/{}", sf.id))
                .w(480.0)
                .grow(1.0)
                .padding(16.0)
                .gap(10.0)
                .radius(radius::LG)
                .border(1.0, th::blue().alpha(0.2))
                .fill(th::glass())
                .child(head)
                .child(crate::a2ui::view::surface_view(&s.a2, sf)),
        );
    }
    let body = if surfaces.is_empty() {
        col()
            .w_full()
            .grow(1.0)
            .center()
            .gap(8.0)
            .child(rune::icon(&icons::LAYERS, 30.0).color(th::muted_foreground()))
            .child(
                text("Aún no hay superficies")
                    .font(TextStyle::new(14.0, 600.0))
                    .color(th::foreground()),
            )
            .child(
                text("Cuando un agente cree un formulario o un panel, aparecerá aquí.")
                    .font(type_::BODY.lh(20.0))
                    .color(th::muted_foreground()),
            )
    } else {
        col()
            .w_full()
            .padding(24.0)
            .child(grid)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0)
    };
    let footer = row()
        .w_full()
        .px(24.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .border(1.0, th::border_subtle())
        .child(rune::icon(&icons::LAYERS, 12.0).color(th::blue().alpha(0.6)))
        .child(
            text(footer_label(surfaces.len()))
                .font(TextStyle::new(10.5, 500.0).mono().tracking(0.8))
                .color(th::text_muted()),
        );
    col().w_full().grow(1.0).min_h(0.0).padding(16.0).child(
        col()
            .key("panel/card")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .radius(radius::LG)
            .fill(th::card())
            .border(1.0, th::border_subtle())
            .child(header)
            .child(body)
            .child(footer),
    )
}

#[allow(non_snake_case)]
fn Color_white(a: f32) -> rune::Color {
    if th::is_light() {
        rune::Color::BLACK.alpha(a)
    } else {
        rune::Color::WHITE.alpha(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_title_is_the_agent_or_the_surface_id() {
        assert_eq!(
            surface_title(&json!({"agentDisplayName": "Asistente IA"}), "s1"),
            ("Asistente IA".to_string(), true)
        );
        assert_eq!(
            surface_title(&json!({"agentDisplayName": "  "}), "s1"),
            ("s1".to_string(), false)
        );
        assert_eq!(surface_title(&json!({}), "s1"), ("s1".to_string(), false));
        assert_eq!(surface_title(&Value::Null, "s1"), ("s1".to_string(), false));
    }

    #[test]
    fn the_footer_counts_like_the_web() {
        assert_eq!(footer_label(0), "Panel interactivo · sin superficies");
        assert_eq!(footer_label(1), "1 superficie A2UI activa");
        assert_eq!(footer_label(3), "3 superficies A2UI activas");
    }
}
