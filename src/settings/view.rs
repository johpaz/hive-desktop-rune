//! Vista de Rune de los ajustes, con el diseño de las páginas de la web: Providers, Canales, Herramientas, Skills y
//! Servidores MCP (cada una con su cabecera, sus filtros y su rejilla de tarjetas con interruptores).

use crate::ui::text;
use crate::ui::Tap;
use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, text_input, Color, El, ScrollSpec, TextEdit};

use super::{api::Provider, Action, Tab};
use crate::ui::{icons, theme as th};
use crate::State;

/// Filas que se pintan como máximo en las listas largas (hay cientos de herramientas).
const MAX_ROWS: usize = 60;
/// Ancho mínimo de una tarjeta de la rejilla: tres por fila en una ventana normal.
const CARD_W: f32 = 340.0;

pub(crate) fn button(
    label: &str,
    key: String,
    fill: Color,
    on: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .px(12.0)
        .py(7.0)
        .radius(radius::SM)
        .fill(fill)
        .center()
        .child(
            text(label.to_string())
                .font(type_::LABEL)
                .color(th::foreground()),
        )
        .tap(on)
}

/// Cambia de página de Ajustes; herramientas, skills y MCP se leen del gateway la primera vez.
pub fn select_tab(s: &mut State, tab: Tab) {
    s.settings.tab = tab;
    s.settings.config_open = None;
    s.settings.filter = TextEdit::new("");
    if matches!(tab, Tab::Tools | Tab::Skills | Tab::Mcp)
        && !s.settings.shown.extras_loaded
        && s.settings.shown.busy == 0
    {
        s.settings.dispatch(Action::RefreshExtras);
    }
    let conn = s.settings.conn.clone();
    match (tab, conn) {
        (Tab::Ethics, Some(c)) if !s.sys.shown.ethics_loaded => {
            s.sys.refresh(&c, crate::system::Part::Ethics)
        }
        (Tab::Profile, Some(c)) if !s.sys.shown.profile_loaded => {
            s.sys.refresh(&c, crate::system::Part::Profile)
        }
        (Tab::Updates, Some(c)) if s.sys.shown.version.is_none() => {
            s.sys.refresh(&c, crate::system::Part::Dashboard)
        }
        (Tab::Ocr, Some(c)) if !s.ocr.shown.loaded => s.ocr.refresh(&c),
        (Tab::Computer, Some(c)) if !s.computer.remote.shown.loaded => s.computer.load(&c),
        (Tab::Oracle, Some(c)) if !s.oracle.remote.shown.loaded => s.oracle.load(&c),
        _ => {}
    }
    if tab == Tab::Display {
        s.audio.load();
    }
}

pub(crate) fn field(
    edit: &TextEdit,
    key: String,
    hint: &str,
    pick: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
) -> El {
    row()
        .w_full()
        .px(10.0)
        .py(7.0)
        .radius(radius::SM)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(
            text_input(edit, type_::BODY.lh(24.0))
                .key(key)
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .color(th::foreground())
                .placeholder(hint, th::text_muted())
                .on_edit(pick),
        )
}

// ─── piezas comunes ──────────────────────────────────────────────────────────

pub(crate) fn mono(size: f32, tracking: f32) -> TextStyle {
    TextStyle::new(size, 600.0).mono().tracking(tracking)
}

pub(crate) fn cardx() -> El {
    col()
        .radius(14.0)
        .fill(th::card())
        .border(1.0, th::border_subtle())
}

pub(crate) fn grid() -> El {
    row().w_full().gap(16.0).flex_wrap()
}

/// Cabecera de página: etiqueta con punto azul, título con una palabra en azul y subtítulo.
pub(crate) fn header(eyebrow: &str, pre: &str, accent: &str, sub: &str, right: Option<El>) -> El {
    let left = col()
        .shrink()
        .min_w(280.0)
        .grow(1.0)
        .gap(8.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(rune::el().size(6.0, 6.0).radius(3.0).fill(th::blue()))
                .child(
                    text(eyebrow.to_uppercase())
                        .font(mono(10.0, 2.0))
                        .color(th::blue()),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(10.0)
                .flex_wrap()
                .child(
                    text(pre.to_string())
                        .font(TextStyle::new(32.0, 800.0))
                        .color(th::foreground()),
                )
                .child(
                    text(accent.to_string())
                        .font(TextStyle::new(32.0, 800.0).italic())
                        .color(th::blue()),
                ),
        )
        .child(
            text(sub.to_string())
                .font(type_::BODY.lh(20.0))
                .color(th::muted_foreground()),
        );
    row()
        .w_full()
        .gap(16.0)
        .flex_wrap()
        .justify_between()
        .items_end()
        .child(left)
        .children(right)
}

/// El interruptor de la web: «ON»/«OFF» y una píldora azul cuando está encendido.
pub(crate) fn switch(key: String, on: bool, press: impl Fn(&mut State) + 'static) -> El {
    let knob = rune::el()
        .size(18.0, 18.0)
        .radius(9.0)
        .fill(Color::WHITE)
        .absolute(if on { 23.0 } else { 3.0 }, 3.0);
    row()
        .key(key)
        .gap(8.0)
        .center_y()
        .child(
            text(if on { "ON" } else { "OFF" })
                .font(mono(9.5, 1.2))
                .color(if on { th::blue() } else { th::text_muted() }),
        )
        .child(
            row()
                .size(44.0, 24.0)
                .radius(12.0)
                .fill(if on { th::blue_dark() } else { th::muted() })
                .child(knob),
        )
        .tap(press)
}

/// Un filtro en píldora (TODOS · ACTIVOS · …).
pub(crate) fn pill(
    label: String,
    key: String,
    active: bool,
    press: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .px(14.0)
        .py(7.0)
        .radius(16.0)
        .fill(if active {
            th::blue_dark()
        } else {
            th::glass_strong()
        })
        .border(
            1.0,
            if active {
                th::blue().alpha(0.6)
            } else {
                th::border_base()
            },
        )
        .center()
        .child(
            text(label.to_uppercase())
                .font(mono(10.0, 1.2))
                .color(if active {
                    Color::WHITE
                } else {
                    th::muted_foreground()
                }),
        )
        .tap(press)
}

pub(crate) fn chip(label: &str, tone: Color) -> El {
    row()
        .px(9.0)
        .py(2.0)
        .radius(9.0)
        .fill(tone.alpha(0.12))
        .border(1.0, tone.alpha(0.3))
        .child(text(label.to_string()).font(mono(9.0, 0.8)).color(tone))
}

pub(crate) fn icon_box(icon: &'static rune::Icon, tone: Color, size: f32) -> El {
    row()
        .size(size, size)
        .radius(size * 0.28)
        .center()
        .fill(tone.alpha(0.1))
        .border(1.0, tone.alpha(0.25))
        .child(rune::icon(icon, size * 0.46).color(tone))
}

pub(crate) fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>().trim_end())
    }
}

fn banners(s: &State) -> El {
    let st = &s.settings;
    let mut b = col().w_full().gap(6.0);
    if let Some(e) = &st.shown.error {
        b = b.child(text(e.clone()).font(type_::LABEL).color(th::red()).wrap());
    }
    if let Some(n) = &st.shown.notice {
        b = b.child(text(n.clone()).font(type_::LABEL).color(th::green()).wrap());
    }
    if st.shown.unauthorized {
        b = b.child(
            text("La sesión venció: pulsa «Cerrar sesión» y vuelve a entrar.")
                .font(type_::LABEL)
                .color(th::red())
                .wrap(),
        );
    }
    if !st.shown.loaded && st.shown.busy > 0 {
        b = b.child(
            text("Cargando…")
                .font(type_::LABEL)
                .color(th::muted_foreground()),
        );
    }
    b
}

pub(crate) fn refresh_button(s: &State, label: &str) -> El {
    row()
        .key("set/refresh")
        .gap(8.0)
        .center_y()
        .px(14.0)
        .py(9.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(rune::icon(&icons::REFRESH, 13.0).color(th::muted_foreground()))
        .child(
            text(if s.settings.shown.busy > 0 {
                "TRABAJANDO…".to_string()
            } else {
                label.to_uppercase()
            })
            .font(mono(10.5, 1.2))
            .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            let conn = s.settings.conn.clone();
            match (s.settings.tab, conn) {
                (Tab::Ethics, Some(c)) => s.sys.refresh(&c, crate::system::Part::Ethics),
                (Tab::Updates, Some(c)) => s.sys.refresh(&c, crate::system::Part::Dashboard),
                (Tab::Tools | Tab::Skills | Tab::Mcp, _) => {
                    s.settings.dispatch(Action::RefreshExtras)
                }
                _ => s.settings.dispatch(Action::Refresh),
            }
        })
}

pub fn settings_view(s: &State) -> El {
    // Las páginas del sistema traen su propio marco y su desplazamiento.
    match s.settings.tab {
        Tab::Ocr => return crate::ocr::page(s),
        Tab::Computer => return crate::computer::page(s),
        Tab::Oracle => return crate::oracle::page(s),
        Tab::Ethics => return crate::ui::system_pages::ethics(s),
        Tab::Profile => return crate::ui::system_pages::profile(s),
        Tab::Display => return crate::ui::system_pages::display(s),
        Tab::Updates => return crate::ui::system_pages::updates(s),
        _ => {}
    }
    let page = match s.settings.tab {
        Tab::Providers => providers(s),
        Tab::Channels => channels(s),
        Tab::Tools => tools(s),
        Tab::Skills => skills(s),
        Tab::Mcp => mcp(s),
        _ => col(),
    };
    col().w_full().grow(1.0).min_h(0.0).child(
        col()
            .w_full()
            .padding(28.0)
            .gap(20.0)
            .child(banners(s))
            .child(page)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
    )
}

// ─── Providers ───────────────────────────────────────────────────────────────

fn provider_in_cat(p: &Provider, cat: u8) -> bool {
    p.models.iter().any(|m| m.in_category(cat))
        || match cat {
            0 => p.category.is_empty() || p.category == "llm",
            1 => matches!(p.category.as_str(), "vision" | "image" | "ocr"),
            _ => matches!(p.category.as_str(), "stt" | "tts" | "voice" | "audio"),
        }
}

fn providers(s: &State) -> El {
    let cat = s.settings.prov_cat;
    let all = &s.settings.shown.providers;
    let list: Vec<&Provider> = all.iter().filter(|p| provider_in_cat(p, cat)).collect();
    let active = list.iter().filter(|p| p.enabled && p.active).count();
    let count = |c: u8| all.iter().filter(|p| provider_in_cat(p, c)).count();
    let mut tabs = row().w_full().gap(8.0).flex_wrap();
    for (i, name) in ["Providers", "Imagen", "Voz"].iter().enumerate() {
        let i = i as u8;
        tabs = tabs.child(pill(
            format!("{name} ({})", count(i)),
            format!("prov/cat/{i}"),
            cat == i,
            move |s: &mut State| s.settings.prov_cat = i,
        ));
    }
    let mut g = grid();
    for p in &list {
        g = g.child(provider_card(s, p));
    }
    let empty = list.is_empty() && s.settings.shown.loaded;
    col().w_full().gap(20.0)
        .child(header("Infraestructura", "Providers", "de IA", "Conecta los modelos que usan tus agentes y activa los que quieras ofrecerles.", Some(refresh_button(s, "Sincronizar"))))
        .child(tabs)
        .children((cat>0).then(||text(if cat==1 {"Selecciona el modelo que analizará imágenes para visión. Los generadores de imágenes se muestran con su función."}else{"Configura transcripción y síntesis de voz. El modelo de conversación en vivo se elige en HiveLive."}).font(type_::LABEL).color(th::muted_foreground()).wrap()))
        .child(row().w_full().justify_between().center_y()
            .child(text("PROVIDERS DE IA").font(mono(11.5, 2.2)).color(th::blue()))
            .child(chip(&format!("{active} activos / {} total", list.len()), th::blue())))
        .children(empty.then(|| text("No hay proveedores en esta categoría").font(type_::LABEL).color(th::muted_foreground())))
        .child(g)
}

fn provider_card(s: &State, p: &Provider) -> El {
    let id = p.id.clone();
    let on = p.enabled && p.active;
    let name = if p.name.is_empty() {
        id.clone()
    } else {
        p.name.clone()
    };
    let fg = if on {
        th::foreground()
    } else {
        th::muted_foreground()
    };
    let toggle_id = id.clone();
    let key_line = if p.id == "piper" {
        ("Síntesis local · sin clave de API".to_string(), true)
    } else {
        match (&p.masked_api_key, p.has_api_key) {
            (Some(m), true) => (m.clone(), true),
            (None, true) => ("Clave guardada".to_string(), true),
            _ => ("Sin API key".to_string(), false),
        }
    };
    let models: Vec<_> = p
        .models
        .iter()
        .filter(|m| m.in_category(s.settings.prov_cat))
        .collect();
    let enabled: Vec<_> = models.iter().filter(|m| m.is_active()).collect();
    let mut chips = row().w_full().gap(6.0).flex_wrap();
    for m in enabled.iter().take(3) {
        let label = if m.name.is_empty() { &m.id } else { &m.name };
        chips = chips.child(
            row()
                .px(8.0)
                .py(3.0)
                .radius(8.0)
                .border(1.0, th::border_base())
                .child(
                    text(short(label, 22).to_uppercase())
                        .font(mono(8.5, 0.4))
                        .color(fg),
                ),
        );
    }
    if enabled.len() > 3 {
        chips = chips.child(
            row()
                .px(8.0)
                .py(3.0)
                .radius(8.0)
                .border(1.0, th::border_subtle())
                .child(
                    text(format!("+{}", enabled.len() - 3))
                        .font(mono(8.5, 0.4))
                        .color(th::text_muted()),
                ),
        );
    }
    let open = s.settings.config_open.as_deref() == Some(id.as_str());
    let cfg_id = id.clone();
    let mut c = cardx()
        .key(format!("prov/{id}"))
        .w(CARD_W.min(s.content_width() - 56.0))
        .min_w(CARD_W.min(s.content_width() - 56.0))
        .grow(1.0)
        .padding(18.0)
        .gap(12.0)
        .border(
            1.0,
            if on {
                th::blue().alpha(0.25)
            } else {
                th::border_subtle()
            },
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .shrink()
                        .grow(1.0)
                        .gap(12.0)
                        .center_y()
                        .child(icon_box(
                            &icons::CPU,
                            if on {
                                th::blue()
                            } else {
                                th::muted_foreground()
                            },
                            40.0,
                        ))
                        .child(
                            col()
                                .shrink()
                                .gap(1.0)
                                .child(
                                    text(name.to_uppercase())
                                        .font(TextStyle::new(15.0, 800.0))
                                        .color(fg),
                                )
                                .child(
                                    text(id.clone())
                                        .font(TextStyle::new(10.0, 400.0).mono())
                                        .color(th::text_muted()),
                                ),
                        ),
                )
                .child(switch(
                    format!("prov/{id}/toggle"),
                    on,
                    move |s: &mut State| {
                        s.settings.dispatch(Action::ToggleProvider {
                            id: toggle_id.clone(),
                            active: !on,
                        })
                    },
                )),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(rune::icon(&icons::SHIELD, 13.0).color(if key_line.1 {
                    th::green()
                } else {
                    th::text_muted()
                }))
                .child(
                    text(key_line.0)
                        .font(TextStyle::new(11.5, 400.0).mono())
                        .color(if key_line.1 {
                            th::foreground()
                        } else {
                            th::text_muted()
                        }),
                ),
        )
        .child(col().w_full().h(1.0).fill(th::border_subtle()))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    text("MODELOS")
                        .font(mono(9.5, 1.6))
                        .color(th::muted_foreground()),
                )
                .child(chip(
                    &format!("{}/{}", enabled.len(), models.len()),
                    th::muted_foreground(),
                )),
        )
        .child(chips)
        .child(
            row().w_full().justify_end().child(
                row()
                    .key(format!("prov/{id}/config"))
                    .gap(7.0)
                    .center_y()
                    .px(14.0)
                    .py(7.0)
                    .radius(9.0)
                    .border(
                        1.0,
                        if open {
                            th::blue().alpha(0.6)
                        } else {
                            th::border_base()
                        },
                    )
                    .child(rune::icon(&icons::SETTINGS, 13.0).color(th::muted_foreground()))
                    .child(text("CONFIG").font(mono(10.0, 1.4)).color(th::foreground()))
                    .tap(move |s: &mut State| {
                        s.settings.config_open =
                            if s.settings.config_open.as_deref() == Some(cfg_id.as_str()) {
                                None
                            } else {
                                Some(cfg_id.clone())
                            }
                    }),
            ),
        );
    if open {
        c = c.child(provider_config(s, p));
    } else if on && s.settings.prov_cat > 0 && p.id != "piper" {
        c = c.child(provider_models(s, p));
    }
    c
}

/// Lo que se configura de un proveedor: su clave (nunca se muestra la guardada) y sus modelos.
fn provider_config(s: &State, p: &Provider) -> El {
    if p.id == "piper" {
        return piper_config(s);
    }
    let id = p.id.clone();
    let accent = th::amber();
    let (edit_id, set_id, verify_id, ask_id, yes_id) =
        (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let temp = TextEdit::new("").secure();
    let edit = s.settings.key_edit(&id).unwrap_or(&temp);
    let mut c = col()
        .w_full()
        .gap(10.0)
        .py(6.0)
        .border(1.0, th::border_subtle())
        .child(field(
            edit,
            format!("prov/{id}/key"),
            "Nueva clave de API",
            move |s: &mut State| {
                s.settings
                    .keys
                    .entry(edit_id.clone())
                    .or_insert_with(|| TextEdit::new("").secure())
            },
        ));
    let confirming = s.settings.confirm_clear.as_deref() == Some(id.as_str());
    let mut actions = row()
        .gap(6.0)
        .flex_wrap()
        .child(button(
            "Guardar clave",
            format!("prov/{id}/save"),
            accent.alpha(0.5),
            move |s: &mut State| s.settings.dispatch(Action::SetKey { id: set_id.clone() }),
        ))
        .child(button(
            "Verificar",
            format!("prov/{id}/verify"),
            th::glass_strong(),
            move |s: &mut State| {
                s.settings.dispatch(Action::VerifyKey {
                    id: verify_id.clone(),
                })
            },
        ));
    if p.has_api_key {
        actions = if confirming {
            actions
                .child(
                    text("¿Eliminar la clave?")
                        .font(type_::LABEL)
                        .color(th::red()),
                )
                .child(button(
                    "Sí, eliminar",
                    format!("prov/{id}/clear-yes"),
                    th::red().alpha(0.6),
                    move |s: &mut State| {
                        s.settings.dispatch(Action::ClearKey { id: yes_id.clone() })
                    },
                ))
                .child(button(
                    "No",
                    format!("prov/{id}/clear-no"),
                    th::glass_strong(),
                    |s: &mut State| s.settings.confirm_clear = None,
                ))
        } else {
            actions.child(button(
                "Quitar clave",
                format!("prov/{id}/clear"),
                th::glass_strong(),
                move |s: &mut State| s.settings.confirm_clear = Some(ask_id.clone()),
            ))
        };
    }
    c = c.child(actions);
    if p.enabled && p.active && p.id != "piper" {
        let sync_id = p.id.clone();
        c = c.child(button(
            "Actualizar catálogo",
            format!("prov/{id}/sync-models"),
            th::glass_strong(),
            move |s: &mut State| {
                s.settings.dispatch(Action::SyncModels {
                    id: sync_id.clone(),
                })
            },
        ));
    }
    if s.settings.prov_cat > 0 {
        c = c.child(provider_models(s, p));
    } else if !p.models.is_empty() {
        let open = s.settings.models_open.as_deref() == Some(id.as_str());
        let open_id = id.clone();
        c = c.child(button(
            if open {
                "Ocultar modelos"
            } else {
                "Ver modelos"
            },
            format!("prov/{id}/models"),
            th::glass_strong(),
            move |s: &mut State| {
                s.settings.models_open = if open { None } else { Some(open_id.clone()) }
            },
        ));
        if open {
            c = c.child(provider_models(s, p));
        }
    }
    c
}

fn piper_action(label: &str, key: &str, operation: &str, payload: serde_json::Value) -> El {
    let operation = operation.to_string();
    button(
        label,
        format!("piper/{key}"),
        th::glass_strong(),
        move |s: &mut State| {
            s.settings.dispatch(Action::Piper {
                operation: operation.clone(),
                payload: payload.clone(),
            })
        },
    )
}
fn piper_config(s: &State) -> El {
    use serde_json::json;
    let mut c = col()
        .w_full()
        .gap(10.0)
        .child(
            text("Piper · voz local")
                .font(type_::LABEL)
                .color(th::foreground()),
        )
        .child(piper_action(
            "Consultar estado",
            "refresh",
            "status",
            json!({}),
        ));
    let Some(status) = &s.settings.shown.piper else {
        return c.child(
            text("Consulta el estado para instalar Piper y administrar sus voces.")
                .font(type_::LABEL)
                .color(th::muted_foreground())
                .wrap(),
        );
    };
    let installed = status["installed"] == true;
    let running = status["running"] == true;
    c = c.child(
        text(if running {
            "Servicio iniciado"
        } else if installed {
            "Binario instalado · servicio detenido"
        } else {
            "Piper pendiente de instalar"
        })
        .font(type_::LABEL)
        .color(if running {
            th::green()
        } else {
            th::muted_foreground()
        }),
    );
    if !installed {
        c = c.child(piper_action(
            "Instalar binario nativo",
            "install",
            "install",
            json!({}),
        ));
    } else {
        c = c.child(piper_action(
            if running {
                "Detener Piper"
            } else {
                "Iniciar Piper"
            },
            "service",
            if running { "stop" } else { "start" },
            json!({}),
        ));
    }
    for voice in status["models"].as_array().into_iter().flatten() {
        let id = voice["id"].as_str().unwrap_or_default();
        let ready = voice["installed"] == true;
        let mut line = col().w_full().gap(5.0).child(
            text(voice["name"].as_str().unwrap_or(id))
                .font(type_::LABEL)
                .color(th::foreground()),
        );
        line = line.child(if ready {
            piper_action(
                if voice["selected"] == true {
                    "Voz seleccionada"
                } else {
                    "Seleccionar voz"
                },
                &format!("voice/{id}"),
                "config",
                json!({"voice":id}),
            )
        } else {
            piper_action(
                "Descargar voz",
                &format!("download/{id}"),
                "models/download",
                json!({"voice":id}),
            )
        });
        c = c.child(line);
    }
    c = c
        .child(
            text(format!(
                "Velocidad · escala {}",
                status["settings"]["length_scale"]
            ))
            .font(type_::LABEL)
            .color(th::muted_foreground()),
        )
        .child(
            row()
                .gap(5.0)
                .flex_wrap()
                .child(piper_action(
                    "Pausada",
                    "speed/slow",
                    "config",
                    json!({"length_scale":1.2}),
                ))
                .child(piper_action(
                    "Normal",
                    "speed/normal",
                    "config",
                    json!({"length_scale":1.0}),
                ))
                .child(piper_action(
                    "Rápida",
                    "speed/fast",
                    "config",
                    json!({"length_scale":0.8}),
                )),
        )
        .child(field(
            &s.settings.piper_text,
            "piper/text".into(),
            "Texto de prueba",
            |s: &mut State| &mut s.settings.piper_text,
        ));
    if running {
        c = c.child(piper_action("Probar voz", "speak", "speak", json!({})));
    }
    for line in status["logs"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .take(5)
    {
        c = c.child(
            text(line.as_str().unwrap_or_default())
                .font(type_::LABEL)
                .color(th::muted_foreground())
                .wrap(),
        );
    }
    c
}

fn provider_models(s: &State, p: &Provider) -> El {
    let mut c = col().w_full().gap(10.0);
    if !p.enabled || !p.active {
        return c.child(
            text("Activa el proveedor para configurar sus modelos")
                .font(type_::LABEL)
                .color(th::muted_foreground()),
        );
    }
    let models: Vec<_> = p
        .models
        .iter()
        .filter(|m| m.in_category(s.settings.prov_cat))
        .collect();
    for m in models.iter().take(MAX_ROWS) {
        let (mid, on) = (m.id.clone(), m.is_active());
        let label = if m.name.is_empty() { &m.id } else { &m.name };
        let mut item = col().w_full().gap(6.0).child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .gap(3.0)
                        .child(
                            text(label.clone())
                                .font(type_::LABEL)
                                .color(th::foreground())
                                .wrap(),
                        )
                        .child(
                            text(m.kind_label())
                                .font(type_::LABEL)
                                .color(th::muted_foreground()),
                        ),
                )
                .child(switch(
                    format!("model/{}/toggle", m.id),
                    on,
                    move |s: &mut State| {
                        s.settings.dispatch(Action::ToggleModel {
                            id: mid.clone(),
                            active: !on,
                        })
                    },
                )),
        );
        if !m.availability_note.is_empty() {
            item = item.child(
                text(m.availability_note.clone())
                    .font(type_::LABEL)
                    .color(th::muted_foreground())
                    .wrap(),
            );
        }
        if on {
            if let Some(purpose) = m.purpose(s.settings.prov_cat) {
                let chosen = m.default_for.iter().any(|p| p == purpose);
                let id = m.id.clone();
                let name = match purpose {
                    "vision" => "Usar para visión",
                    "stt" => "Usar para transcribir",
                    _ => "Usar para voz",
                };
                item = item.child(button(
                    if chosen { "Predeterminado" } else { name },
                    format!("model/{}/{purpose}/default", m.id),
                    if chosen {
                        th::blue().alpha(0.3)
                    } else {
                        th::glass_strong()
                    },
                    move |s: &mut State| {
                        s.settings.dispatch(Action::DefaultModel {
                            id: id.clone(),
                            purpose: purpose.into(),
                        })
                    },
                ));
            }
        }
        c = c.child(item);
    }
    if models.len() > MAX_ROWS {
        c = c.child(
            text(format!(
                "Se muestran {MAX_ROWS} de {} modelos",
                models.len()
            ))
            .font(type_::LABEL)
            .color(th::muted_foreground()),
        );
    }
    c
}

// ─── Canales ─────────────────────────────────────────────────────────────────

fn channels(s: &State) -> El {
    let all = &s.settings.shown.channels;
    let is_active = |c: &super::api::Channel| c.enabled && c.active;
    let (n_all, n_act) = (all.len(), all.iter().filter(|c| is_active(c)).count());
    let filter = s.settings.chan_filter;
    let shown: Vec<&super::api::Channel> = all
        .iter()
        .filter(|c| match filter {
            1 => is_active(c),
            2 => !is_active(c),
            _ => true,
        })
        .collect();
    let mut tabs = row().gap(6.0);
    for (i, (name, n)) in [
        ("Todos", n_all),
        ("Activos", n_act),
        ("Inactivos", n_all - n_act),
    ]
    .iter()
    .enumerate()
    {
        let i = i as u8;
        tabs = tabs.child(pill(
            format!("{name} ({n})"),
            format!("chan/filter/{i}"),
            filter == i,
            move |s: &mut State| s.settings.chan_filter = i,
        ));
    }
    let new_btn = row()
        .key("chan/new")
        .gap(8.0)
        .center_y()
        .px(18.0)
        .py(10.0)
        .radius(10.0)
        .fill(th::blue_dark())
        .shadow(0.0, 14.0, th::blue().alpha(0.3))
        .child(rune::icon(&icons::PLUS, 14.0).color(Color::WHITE))
        .child(
            text("NUEVO CANAL")
                .font(mono(10.5, 1.4))
                .color(Color::WHITE),
        )
        .tap(|s: &mut State| s.settings.add_open = !s.settings.add_open);
    let top = row().w_full().gap(16.0).flex_wrap().justify_between().items_end()
        .child(row().shrink().grow(1.0).gap(14.0).items_end()
            .child(icon_box(&icons::LAYERS, th::blue(), 44.0))
            .child(col().shrink().gap(6.0).child(text("CANALES").font(TextStyle::new(32.0, 800.0)).color(th::foreground())).child(text("Gestiona tus puntos de entrada y salida de comunicación en la red Hive.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()))))
        .child(row().gap(12.0).child(refresh_button(s, "Sincronizar")).child(new_btn));
    let status = cardx().w_full().px(20.0).py(14.0).child(
        row()
            .w_full()
            .justify_between()
            .center_y()
            .child(
                row()
                    .gap(14.0)
                    .center_y()
                    .child(icon_box(&icons::ZAP, th::blue(), 36.0))
                    .child(
                        col()
                            .gap(3.0)
                            .child(
                                text("ESTADO OPERATIVO")
                                    .font(mono(9.5, 1.8))
                                    .color(th::blue()),
                            )
                            .child(
                                row()
                                    .gap(0.0)
                                    .child(
                                        text("Actualmente tienes ")
                                            .font(type_::BODY)
                                            .color(th::foreground()),
                                    )
                                    .child(
                                        text(n_act.to_string())
                                            .font(TextStyle::new(13.0, 700.0))
                                            .color(th::blue()),
                                    )
                                    .child(
                                        text(" canales procesando datos.")
                                            .font(type_::BODY)
                                            .color(th::foreground()),
                                    ),
                            ),
                    ),
            )
            .child(
                text("HIVE NODE SECURE")
                    .font(mono(9.5, 1.6))
                    .color(th::text_muted()),
            ),
    );
    let mut g = grid();
    for ch in shown {
        g = g.child(channel_card(s, ch));
    }
    col()
        .w_full()
        .gap(20.0)
        .child(top)
        .child(status)
        .child(tabs)
        .children(s.settings.add_open.then(|| add_channel(s)))
        .children((all.is_empty() && s.settings.shown.loaded).then(|| {
            text("No hay canales")
                .font(type_::LABEL)
                .color(th::muted_foreground())
        }))
        .child(g)
}

fn channel_card(s: &State, ch: &super::api::Channel) -> El {
    let id = ch.id.clone();
    let active = ch.enabled && ch.active;
    let connected = ch.status == "connected";
    let configured = ch.is_configured || connected;
    let name = if ch.kind.is_empty() {
        ch.id.clone()
    } else {
        ch.kind.clone()
    };
    let tone = if connected { th::green() } else { th::blue() };
    let open = s.settings.config_open.as_deref() == Some(id.as_str());
    let open_id = id.clone();
    let mut tags = row().gap(10.0);
    if connected {
        tags = tags.child(text("CONECTADO").font(mono(8.5, 1.4)).color(th::green()));
    }
    tags = tags.child(
        text(if configured {
            "CONFIGURADO"
        } else {
            "DISPONIBLE"
        })
        .font(mono(8.5, 1.4))
        .color(th::blue().alpha(0.7)),
    );
    let mut c = cardx()
        .key(format!("chan/card/{id}"))
        .w(CARD_W.min(s.content_width() - 56.0))
        .min_w(CARD_W.min(s.content_width() - 56.0))
        .grow(1.0)
        .padding(20.0)
        .gap(14.0)
        .border(
            1.0,
            if configured {
                th::border_base()
            } else {
                th::border_subtle()
            },
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .items_start()
                .child(icon_box(&icons::CHAT, tone, 46.0))
                .child(tags),
        )
        .child(
            col()
                .gap(4.0)
                .child(
                    text(name.to_uppercase())
                        .font(TextStyle::new(22.0, 800.0))
                        .color(if configured {
                            th::foreground()
                        } else {
                            th::muted_foreground()
                        }),
                )
                .child(
                    text(if configured {
                        if active {
                            "ACTIVO"
                        } else {
                            "DESACTIVADO"
                        }
                    } else {
                        "CLICK PARA CONFIGURAR"
                    })
                    .font(mono(9.5, 1.6))
                    .color(th::text_muted()),
                ),
        )
        .tap(move |s: &mut State| {
            s.settings.config_open = if s.settings.config_open.as_deref() == Some(open_id.as_str())
            {
                None
            } else {
                Some(open_id.clone())
            }
        });
    if open {
        c = c.child(channel_config(s, ch));
    }
    c
}

/// La configuración de un canal: activarlo, su token de bot y, en WhatsApp, vincular con QR.
fn channel_config(s: &State, ch: &super::api::Channel) -> El {
    let accent = th::amber();
    let id = ch.id.clone();
    let active = ch.enabled && ch.active;
    let tid = id.clone();
    let toggle_id = id.clone();
    let mut c = col()
        .w_full()
        .gap(10.0)
        .py(8.0)
        .border(1.0, th::border_subtle())
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(if ch.status.is_empty() {
                        "sin estado".to_string()
                    } else {
                        ch.status.clone()
                    })
                    .font(TextStyle::new(11.0, 400.0).mono())
                    .color(th::muted_foreground())
                    .grow(1.0),
                )
                .child(switch(
                    format!("chan/{id}/toggle"),
                    active,
                    move |s: &mut State| {
                        s.settings.dispatch(Action::ToggleChannel {
                            id: toggle_id.clone(),
                            active: !active,
                        })
                    },
                )),
        );
    if matches!(ch.kind.as_str(), "telegram" | "discord" | "slack") {
        let (edit_id, save_id) = (tid.clone(), tid.clone());
        let temp = TextEdit::new("").secure();
        let edit = s.settings.channel_tokens.get(&tid).unwrap_or(&temp);
        c = c
            .child(field(
                edit,
                format!("chan/{tid}/token"),
                "Nuevo token del bot",
                move |s: &mut State| {
                    s.settings
                        .channel_tokens
                        .entry(edit_id.clone())
                        .or_insert_with(|| TextEdit::new("").secure())
                },
            ))
            .child(button(
                "Guardar token",
                format!("chan/{tid}/token-save"),
                accent.alpha(0.5),
                move |s: &mut State| {
                    s.settings.dispatch(Action::SetChannelToken {
                        id: save_id.clone(),
                    })
                },
            ));
    }
    if ch.kind == "whatsapp" {
        let wa = s
            .settings
            .shown
            .whatsapp
            .as_ref()
            .filter(|w| w.channel == ch.id);
        let waiting = wa.is_some_and(|w| matches!(w.status.as_str(), "starting" | "qr"));
        let wid = ch.id.clone();
        c = c.child(button(
            if waiting {
                "Esperando el escaneo…"
            } else {
                "Vincular con QR"
            },
            format!("chan/{}/link", ch.id),
            accent.alpha(0.5),
            move |s: &mut State| {
                s.settings
                    .dispatch(Action::LinkWhatsApp { id: wid.clone() })
            },
        ));
        if let Some(w) = wa {
            let dim = th::muted_foreground();
            c = match (w.status.as_str(), &w.qr) {
                ("qr", Some(_)) => c
                    .child(rune::el().key(format!("chan/{}/qr", ch.id)).size(240.0, 240.0).radius(6.0).pixels_fit(&s.settings.qr_pixels, rune::SurfaceFit::Fit))
                    .child(text("En el teléfono: WhatsApp → Dispositivos vinculados → Vincular un dispositivo. El código se renueva solo.").font(type_::LABEL).color(dim).wrap()),
                ("starting", _) => c.child(text("Generando el código QR…").font(type_::LABEL).color(dim)),
                ("connected", _) => c.child(text("WhatsApp vinculado").font(type_::LABEL).color(th::green())),
                ("error", _) => c.child(text("No se pudo vincular. Inténtalo de nuevo.").font(type_::LABEL).color(dim)),
                ("timeout", _) => c.child(text("El código caducó sin escanearse. Vuelve a pedirlo.").font(type_::LABEL).color(dim)),
                _ => c,
            };
        }
    }
    c
}

/// Formulario «Nuevo canal»: tipo, token del bot y el segundo dato que pide cada tipo.
fn add_channel(s: &State) -> El {
    let accent = th::amber();
    let chosen = s.settings.add_kind.as_deref();
    let mut kinds = row().gap(6.0).flex_wrap();
    for k in ["telegram", "discord", "slack", "whatsapp"] {
        kinds = kinds.child(pill(
            k.to_string(),
            format!("chan/add/{k}"),
            chosen == Some(k),
            move |s: &mut State| {
                s.settings.add_kind = if s.settings.add_kind.as_deref() == Some(k) {
                    None
                } else {
                    Some(k.to_string())
                };
                // Token y signing secret son secretos; el Application ID de Discord no.
                s.settings.add_token = TextEdit::new("").secure();
                s.settings.add_extra = if k == "slack" {
                    TextEdit::new("").secure()
                } else {
                    TextEdit::new("")
                };
            },
        ));
    }
    let mut c = cardx()
        .w_full()
        .padding(18.0)
        .gap(10.0)
        .child(text("NUEVO CANAL").font(mono(10.5, 2.0)).color(th::blue()))
        .child(kinds);
    if let Some(kind) = chosen {
        if kind != "whatsapp" {
            c = c.child(field(
                &s.settings.add_token,
                "chan/add/token".into(),
                "Token del bot",
                |s: &mut State| &mut s.settings.add_token,
            ));
        }
        match kind {
            "discord" => {
                c = c.child(field(
                    &s.settings.add_extra,
                    "chan/add/extra".into(),
                    "Application ID",
                    |s: &mut State| &mut s.settings.add_extra,
                ))
            }
            "slack" => {
                c = c.child(field(
                    &s.settings.add_extra,
                    "chan/add/extra".into(),
                    "Signing secret",
                    |s: &mut State| &mut s.settings.add_extra,
                ))
            }
            _ => {}
        }
        c = c.child(button(
            "Crear canal",
            "chan/add/create".into(),
            accent.alpha(0.5),
            |s: &mut State| s.settings.dispatch(Action::CreateChannel),
        ));
    }
    c
}

// ─── Herramientas, Skills y MCP ──────────────────────────────────────────────

/// Texto del buscador en minúsculas; vacío = todo.
fn needle(s: &State) -> String {
    s.settings.filter.text.trim().to_lowercase()
}

fn search_box(s: &State, hint: &str) -> El {
    row()
        .w(260.0)
        .max_w((s.content_width() - 56.0).max(0.0))
        .shrink()
        .child(field(
            &s.settings.filter,
            "set/filter".into(),
            hint,
            |s: &mut State| &mut s.settings.filter,
        ))
}

/// Las categorías distintas (en orden alfabético) con cuántos elementos tiene cada una.
fn categories<'a>(items: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut map: std::collections::BTreeMap<String, usize> = Default::default();
    for c in items.filter(|c| !c.is_empty()) {
        *map.entry(c.to_string()).or_default() += 1;
    }
    map.into_iter().collect()
}

fn category_pills(
    prefix: &'static str,
    total: usize,
    cats: &[(String, usize)],
    current: Option<&str>,
    set: fn(&mut State, Option<String>),
) -> El {
    let mut r = row().w_full().gap(8.0).flex_wrap().child(pill(
        format!("Todos ({total})"),
        format!("{prefix}/cat/all"),
        current.is_none(),
        move |s: &mut State| set(s, None),
    ));
    for (c, n) in cats.iter().take(14) {
        let cc = c.clone();
        r = r.child(pill(
            format!("{c} ({n})"),
            format!("{prefix}/cat/{c}"),
            current == Some(c.as_str()),
            move |s: &mut State| set(s, Some(cc.clone())),
        ));
    }
    r
}

fn tools(s: &State) -> El {
    let q = needle(s);
    let all = &s.settings.shown.tools;
    let cats = categories(all.iter().map(|t| t.category.as_str()));
    let cat = s.settings.tool_cat.as_deref();
    let hits: Vec<_> = all
        .iter()
        .filter(|x| cat.is_none_or(|c| x.category == c))
        .filter(|x| {
            q.is_empty()
                || [&x.name, &x.description, &x.category]
                    .iter()
                    .any(|f| f.to_lowercase().contains(&q))
        })
        .collect();
    let mut g = grid();
    for x in hits.iter().take(MAX_ROWS) {
        let id = x.id.clone();
        let active = x.active && x.enabled;
        let title = if x.name.is_empty() { &x.id } else { &x.name };
        g = g.child(
            cardx()
                .key(format!("tool/{}", x.id))
                .w(CARD_W.min(s.content_width() - 56.0))
                .min_w(CARD_W.min(s.content_width() - 56.0))
                .grow(1.0)
                .padding(18.0)
                .gap(12.0)
                .border(
                    1.0,
                    if active {
                        th::blue().alpha(0.22)
                    } else {
                        th::border_subtle()
                    },
                )
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(
                            row()
                                .size(34.0, 34.0)
                                .radius(17.0)
                                .center()
                                .fill(th::blue().alpha(if active { 0.15 } else { 0.05 }))
                                .border(1.0, th::blue().alpha(0.25))
                                .child(rune::icon(&icons::WRENCH, 15.0).color(if active {
                                    th::blue()
                                } else {
                                    th::muted_foreground()
                                })),
                        )
                        .child(switch(
                            format!("tool/{}/toggle", x.id),
                            active,
                            move |s: &mut State| {
                                s.settings.dispatch(Action::ToggleTool {
                                    id: id.clone(),
                                    active: !active,
                                })
                            },
                        )),
                )
                .child(
                    text(title.to_uppercase())
                        .font(TextStyle::new(15.0, 800.0))
                        .color(if active {
                            th::foreground()
                        } else {
                            th::muted_foreground()
                        })
                        .wrap(),
                )
                .child(
                    text(short(&x.description, 96))
                        .font(TextStyle::new(11.5, 400.0).lh(17.0))
                        .color(th::muted_foreground())
                        .wrap(),
                )
                .child(col().w_full().h(1.0).fill(th::border_subtle()))
                .child(
                    row()
                        .gap(22.0)
                        .child(
                            row()
                                .gap(6.0)
                                .items_end()
                                .child(text("TIPO").font(mono(8.5, 1.2)).color(th::text_muted()))
                                .child(
                                    text(if x.category.is_empty() {
                                        "—".to_string()
                                    } else {
                                        x.category.clone()
                                    })
                                    .font(TextStyle::new(12.0, 600.0).mono())
                                    .color(th::foreground()),
                                ),
                        )
                        .child(
                            row()
                                .gap(6.0)
                                .items_end()
                                .child(text("ESTADO").font(mono(8.5, 1.2)).color(th::text_muted()))
                                .child(
                                    text(if !active {
                                        "INACTIVE"
                                    } else if x.runtime_available == Some(false) {
                                        "CONFIGURADA"
                                    } else {
                                        "ACTIVE"
                                    })
                                    .font(mono(10.0, 0.8))
                                    .color(if active {
                                        th::blue()
                                    } else {
                                        th::text_muted()
                                    }),
                                ),
                        ),
                ),
        );
    }
    col()
        .w_full()
        .gap(20.0)
        .child(header(
            "Capacidades dinámicas",
            "Gestión",
            "de Herramientas",
            "Habilita y configura las funciones que tus agentes pueden ejecutar.",
            Some(refresh_button(s, "Actualizar")),
        ))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .child(
                            text("HERRAMIENTAS")
                                .font(TextStyle::new(15.0, 800.0).tracking(2.4))
                                .color(th::foreground()),
                        )
                        .child(chip(
                            &format!("{} en catálogo", all.len()),
                            th::muted_foreground(),
                        )),
                )
                .child(search_box(s, "Buscar herramienta...")),
        )
        .child(category_pills("tool", all.len(), &cats, cat, |s, c| {
            s.settings.tool_cat = c
        }))
        .child(
            text(format!(
                "{} de {} herramientas{}",
                hits.len().min(MAX_ROWS),
                hits.len(),
                if hits.len() > MAX_ROWS {
                    " · afina la búsqueda para ver el resto"
                } else {
                    ""
                }
            ))
            .font(type_::LABEL)
            .color(th::muted_foreground()),
        )
        .child(g)
}

fn skills(s: &State) -> El {
    let q = needle(s);
    let all = &s.settings.shown.skills;
    let cats = categories(all.iter().map(|k| k.category.as_str()));
    let cat = s.settings.skill_cat.as_deref();
    let hits: Vec<_> = all
        .iter()
        .filter(|x| cat.is_none_or(|c| x.category == c))
        .filter(|x| {
            q.is_empty()
                || x.name.to_lowercase().contains(&q)
                || x.description.to_lowercase().contains(&q)
        })
        .collect();
    let on = all.iter().filter(|k| k.active).count();
    let mut g = grid();
    for x in hits.iter().take(MAX_ROWS) {
        let (id, active) = (x.id.clone(), x.active);
        let title = if x.name.is_empty() { &x.id } else { &x.name };
        let mut tags = row().gap(6.0).flex_wrap();
        if !x.version.is_empty() {
            tags = tags.child(chip(&format!("v{}", x.version), th::amber()));
        }
        if x.tool_count() > 0 {
            tags = tags.child(chip(&format!("{} tools", x.tool_count()), th::cyan()));
        }
        if x.trigger_count() > 0 {
            tags = tags.child(chip(
                &format!("{} triggers", x.trigger_count()),
                th::purple(),
            ));
        }
        g = g.child(
            cardx()
                .key(format!("skill/{}", x.id))
                .w(CARD_W.min(s.content_width() - 56.0))
                .min_w(CARD_W.min(s.content_width() - 56.0))
                .grow(1.0)
                .padding(18.0)
                .gap(12.0)
                .border(
                    1.0,
                    if active {
                        th::blue().alpha(0.22)
                    } else {
                        th::border_subtle()
                    },
                )
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(
                            row()
                                .gap(6.0)
                                .center_y()
                                .px(10.0)
                                .py(3.0)
                                .radius(10.0)
                                .border(1.0, th::border_base())
                                .child(rune::el().size(5.0, 5.0).radius(2.5).fill(th::blue()))
                                .child(
                                    text(if x.category.is_empty() {
                                        "SKILL".to_string()
                                    } else {
                                        x.category.to_uppercase()
                                    })
                                    .font(mono(9.0, 1.0))
                                    .color(th::muted_foreground()),
                                ),
                        )
                        .child(switch(
                            format!("skill/{}/toggle", x.id),
                            active,
                            move |s: &mut State| {
                                s.settings.dispatch(Action::ToggleSkill {
                                    id: id.clone(),
                                    active: !active,
                                })
                            },
                        )),
                )
                .child(
                    row()
                        .gap(12.0)
                        .child(icon_box(&icons::ZAP, th::blue(), 44.0))
                        .child(
                            col()
                                .gap(4.0)
                                .grow(1.0)
                                .min_w(0.0)
                                .child(
                                    text(title.to_uppercase())
                                        .font(TextStyle::new(14.0, 800.0))
                                        .color(if active {
                                            th::foreground()
                                        } else {
                                            th::muted_foreground()
                                        })
                                        .wrap(),
                                )
                                .child(
                                    text(short(&x.description, 90))
                                        .font(TextStyle::new(11.0, 400.0).lh(16.0))
                                        .color(th::muted_foreground())
                                        .wrap(),
                                ),
                        ),
                )
                .child(col().w_full().h(1.0).fill(th::border_subtle()))
                .child(tags),
        );
    }
    col()
        .w_full()
        .gap(20.0)
        .child(header(
            "Habilidades especializadas",
            "Galería",
            "de Skills",
            "Nodos cognitivos preconfigurados para tareas específicas de alto nivel.",
            Some(refresh_button(s, "Actualizar")),
        ))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .child(
                            text("HABILIDADES DEL SISTEMA")
                                .font(TextStyle::new(15.0, 800.0).tracking(2.4))
                                .color(th::blue()),
                        )
                        .child(chip(&format!("{on} activas"), th::blue()))
                        .child(chip(
                            &format!("{} total", all.len()),
                            th::muted_foreground(),
                        )),
                )
                .child(search_box(s, "Buscar habilidad...")),
        )
        .child(category_pills("skill", all.len(), &cats, cat, |s, c| {
            s.settings.skill_cat = c
        }))
        .child(
            text(format!(
                "{} de {} skills{}",
                hits.len().min(MAX_ROWS),
                hits.len(),
                if hits.len() > MAX_ROWS {
                    " · afina la búsqueda para ver el resto"
                } else {
                    ""
                }
            ))
            .font(type_::LABEL)
            .color(th::muted_foreground()),
        )
        .child(g)
}

/// Servidores MCP: solo lectura (el estado cambia solo mientras conectan; «Actualizar» lo vuelve a leer).
fn mcp(s: &State) -> El {
    let q = needle(s);
    let hits: Vec<_> = s
        .settings
        .shown
        .mcp
        .iter()
        .filter(|m| {
            q.is_empty() || m.name.to_lowercase().contains(&q) || m.id.to_lowercase().contains(&q)
        })
        .collect();
    let mut g = grid();
    for m in &hits {
        let (tone, label) = match m.status.as_str() {
            "connected" => (th::cyan(), "LIVE"),
            "error" | "failed" => (th::red(), "ERROR"),
            "" => (th::muted_foreground(), "SIN ESTADO"),
            _ => (th::amber(), "CONECTANDO"),
        };
        let tools = m.tool_count().map_or("—".to_string(), |n| {
            format!("{n} {}", if n == 1 { "expuesta" } else { "expuestas" })
        });
        let transport = m.transport();
        g = g.child(
            cardx()
                .key(format!("mcp/{}", m.id))
                .w((CARD_W + 40.0).min(s.content_width() - 56.0))
                .min_w((CARD_W + 40.0).min(s.content_width() - 56.0))
                .grow(1.0)
                .clip_rounded(14.0)
                .border(1.0, tone.alpha(0.25))
                .child(
                    row()
                        .w_full()
                        .child(col().w(3.0).h_full().fill(tone))
                        .child(
                            col()
                                .grow(1.0)
                                .padding(18.0)
                                .gap(12.0)
                                .child(
                                    row()
                                        .w_full()
                                        .justify_between()
                                        .center_y()
                                        .child(icon_box(&icons::GLOBE, tone, 36.0))
                                        .child(
                                            row()
                                                .gap(6.0)
                                                .center_y()
                                                .px(10.0)
                                                .py(3.0)
                                                .radius(10.0)
                                                .border(1.0, tone.alpha(0.4))
                                                .child(
                                                    rune::el()
                                                        .size(5.0, 5.0)
                                                        .radius(2.5)
                                                        .fill(tone),
                                                )
                                                .child(
                                                    text(label).font(mono(9.0, 1.0)).color(tone),
                                                ),
                                        ),
                                )
                                .child(
                                    text(if m.name.is_empty() {
                                        m.id.clone()
                                    } else {
                                        m.name.clone()
                                    })
                                    .font(TextStyle::new(15.0, 700.0))
                                    .color(if m.enabled {
                                        th::foreground()
                                    } else {
                                        th::muted_foreground()
                                    }),
                                )
                                .child(
                                    text(short(&m.url(), 46))
                                        .font(TextStyle::new(10.0, 400.0).mono())
                                        .color(th::text_muted()),
                                )
                                .child(
                                    row()
                                        .gap(0.0)
                                        .child(
                                            col()
                                                .gap(2.0)
                                                .pr(14.0)
                                                .child(
                                                    text("TRANSPORT")
                                                        .font(mono(8.0, 1.0))
                                                        .color(th::text_muted()),
                                                )
                                                .child(
                                                    text(if transport.is_empty() {
                                                        "—".into()
                                                    } else {
                                                        transport
                                                    })
                                                    .font(TextStyle::new(12.0, 600.0))
                                                    .color(th::foreground()),
                                                ),
                                        )
                                        .child(col().w(1.0).h(28.0).fill(th::border_subtle()))
                                        .child(
                                            col()
                                                .gap(2.0)
                                                .pl(14.0)
                                                .child(
                                                    text("TOOLS")
                                                        .font(mono(8.0, 1.0))
                                                        .color(th::text_muted()),
                                                )
                                                .child(
                                                    text(tools)
                                                        .font(TextStyle::new(12.0, 600.0))
                                                        .color(th::foreground()),
                                                ),
                                        ),
                                ),
                        ),
                ),
        );
    }
    col()
        .w_full()
        .gap(20.0)
        .child(header(
            "Conectividad externa",
            "Servidores",
            "MCP",
            "Conecta Hive con fuentes de datos externas y herramientas de terceros.",
            Some(refresh_button(s, "Actualizar")),
        ))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .gap(12.0)
                        .center_y()
                        .child(icon_box(&icons::NETWORK, th::blue(), 36.0))
                        .child(
                            col()
                                .gap(2.0)
                                .child(
                                    text("MCP ECOSYSTEM")
                                        .font(TextStyle::new(14.0, 800.0).tracking(1.2))
                                        .color(th::foreground()),
                                )
                                .child(
                                    text("Model Context Protocol Servers")
                                        .font(TextStyle::new(10.0, 400.0).mono())
                                        .color(th::text_muted()),
                                ),
                        ),
                )
                .child(search_box(s, "Buscar servidor...")),
        )
        .children(
            (hits.is_empty() && s.settings.shown.extras_loaded).then(|| {
                text("No hay servidores MCP")
                    .font(type_::LABEL)
                    .color(th::muted_foreground())
            }),
        )
        .child(g)
}
