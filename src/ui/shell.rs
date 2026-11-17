//! Marco común de la app, igual que `AppLayout` de la web: barra lateral con la navegación y cabecera
//! con el logo y el estado de la conexión.

use crate::ui::text;
use crate::ui::Tap;
use rune::tok::{radius, type_};
use rune::{col, row, Color, El, Icon};

use super::icons;
use super::theme as th;
use crate::settings::Tab;
use crate::{open_screen, Screen, State};

/// A dónde lleva una entrada de la barra lateral.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Screen(Screen),
    /// Una pestaña de Ajustes (en la web, Agentes, Canales y Providers son páginas propias).
    Settings(Tab),
}

struct Item {
    key: &'static str,
    label: &'static str,
    icon: &'static Icon,
    target: Target,
}

const ITEMS: &[Item] = &[
    Item {
        key: "nav/dashboard",
        label: "Dashboard",
        icon: &icons::DASHBOARD,
        target: Target::Screen(Screen::Dashboard),
    },
    Item {
        key: "nav/chat",
        label: "Web Chat",
        icon: &icons::CHAT,
        target: Target::Screen(Screen::Chat),
    },
    Item {
        key: "nav/live",
        label: "HiveLive",
        icon: &icons::LIVE,
        target: Target::Screen(Screen::Live),
    },
    Item {
        key: "nav/agents",
        label: "Agentes",
        icon: &icons::BOT,
        target: Target::Screen(Screen::Agents),
    },
    Item {
        key: "nav/channels",
        label: "Canales",
        icon: &icons::CHAT,
        target: Target::Settings(Tab::Channels),
    },
    Item {
        key: "nav/providers",
        label: "Providers",
        icon: &icons::BRAIN,
        target: Target::Settings(Tab::Providers),
    },
    Item {
        key: "nav/office",
        label: "Oficina 3D",
        icon: &icons::HEXAGON,
        target: Target::Screen(Screen::Office),
    },
    Item {
        key: "nav/panel",
        label: "Panel interactivo",
        icon: &icons::LAYERS,
        target: Target::Screen(Screen::Panel),
    },
    Item {
        key: "nav/meeting",
        label: "Reuniones",
        icon: &icons::VIDEO,
        target: Target::Screen(Screen::Meeting),
    },
    Item {
        key: "nav/api",
        label: "API Client",
        icon: &icons::GLOBE,
        target: Target::Screen(Screen::ApiClient),
    },
];

/// El grupo «Cognitivo» de la web.
const COGNITIVE: &[Item] = &[
    Item {
        key: "nav/notes",
        label: "Notas",
        icon: &icons::FILE,
        target: Target::Screen(Screen::Notes),
    },
    Item {
        key: "nav/cron",
        label: "Cron Jobs",
        icon: &icons::CLOCK,
        target: Target::Screen(Screen::Cron),
    },
];

/// «Ajustes» abre las herramientas; queda activo con cualquiera de sus pestañas propias.
const SETTINGS_ITEM: Item = Item {
    key: "nav/settings",
    label: "Ajustes",
    icon: &icons::SLIDERS,
    target: Target::Settings(Tab::Tools),
};

fn is_active(s: &State, item: &Item) -> bool {
    match item.target {
        Target::Screen(sc) => s.screen == sc,
        Target::Settings(tab) if item.key == "nav/settings" => {
            s.screen == Screen::Settings
                && matches!(
                    s.settings.tab,
                    Tab::Tools
                        | Tab::Skills
                        | Tab::Mcp
                        | Tab::Ocr
                        | Tab::Computer
                        | Tab::Oracle
                        | Tab::Ethics
                        | Tab::Profile
                        | Tab::Display
                        | Tab::Updates
                )
                && tab == Tab::Tools
        }
        Target::Settings(tab) => s.screen == Screen::Settings && s.settings.tab == tab,
    }
}

fn go(s: &mut State, target: Target) {
    match target {
        Target::Screen(sc) => open_screen(s, sc),
        Target::Settings(tab) => {
            open_screen(s, Screen::Settings);
            crate::settings::view::select_tab(s, tab);
        }
    }
}

fn nav_item(s: &State, item: &Item, collapsed: bool) -> El {
    let available = match item.target {
        Target::Screen(target) => crate::screen_available(s, target),
        Target::Settings(_) => !crate::chat_only(s),
    };
    let active = is_active(s, item);
    let fg = if active {
        th::blue()
    } else {
        th::sidebar_foreground()
    };
    let target = item.target;
    let mut r = row()
        .key(item.key)
        .gap(12.0)
        .center_y()
        .px(if collapsed { 0.0 } else { 16.0 })
        .h(36.0)
        .radius(10.0)
        .child(rune::icon(item.icon, 16.0).color(fg));
    if collapsed {
        r = r.w(40.0).center();
    } else {
        r = r.w_full().child(
            text(item.label)
                .font(type_::BODY.lh(20.0))
                .color(fg)
                .grow(1.0),
        );
        // La insignia del Panel interactivo: cuántas superficies hay activas.
        let n = s.a2.store.surfaces.len();
        if item.key == "nav/panel" && n > 0 {
            r = r.child(
                row()
                    .size(20.0, 20.0)
                    .radius(10.0)
                    .center()
                    .fill(th::blue_dark())
                    .child(
                        text(n.to_string())
                            .font(type_::LABEL)
                            .color(rune::Color::WHITE),
                    ),
            );
        }
    }
    if active {
        // Resalte de la web: fondo azul muy oscuro, borde azul y un brillo suave.
        r = r
            .fill(th::nav_active_fill())
            .border(1.5, th::blue().alpha(0.6))
            .shadow(0.0, 10.0, th::blue().alpha(0.28));
    }
    if available {
        r.tap(move |s: &mut State| go(s, target))
    } else {
        r.child(text("—").font(type_::LABEL).color(th::muted_foreground()))
    }
}

fn section(label: &str) -> El {
    text(label.to_string())
        .font(type_::LABEL)
        .color(th::muted_foreground())
        .px(16.0)
        .py(4.0)
}

pub fn sidebar(s: &State) -> El {
    let collapsed = s.sidebar_is_collapsed();
    // Todo lo navegable va en una columna con desplazamiento (en ventanas bajas no cabe); «Cerrar sesión» queda fijo al pie.
    let mut c = col().w_full().gap(4.0);
    if !collapsed {
        c = c.child(section("Navegación"));
    }
    for item in ITEMS {
        c = c.child(nav_item(s, item, collapsed));
    }
    c = c.child(col().h(12.0));
    if !collapsed {
        c = c.child(section("COGNITIVO"));
    }
    for item in COGNITIVE {
        c = c.child(nav_item(s, item, collapsed));
    }
    c = c.child(col().h(12.0));
    if !collapsed {
        c = c.child(section("SISTEMA"));
    }
    c = c.child(nav_item(s, &SETTINGS_ITEM, collapsed));
    // Las páginas de Ajustes cuelgan de «Ajustes», como en la web, mientras se está en una de ellas.
    let in_env =
        s.screen == Screen::Settings && !matches!(s.settings.tab, Tab::Providers | Tab::Channels);
    if in_env && !collapsed {
        type NavItem<'a> = (&'a str, &'a str, &'static Icon, Tab);
        type Group<'a> = (&'a str, &'a [NavItem<'a>]);
        let groups: [Group; 3] = [
            (
                "ENTORNO",
                &[
                    ("nav/tools", "Herramientas", &icons::WRENCH, Tab::Tools),
                    ("nav/mcp", "MCP Servers", &icons::NETWORK, Tab::Mcp),
                    ("nav/skills", "Skills", &icons::ZAP, Tab::Skills),
                    ("nav/ocr", "OCR", &icons::FILE_TEXT, Tab::Ocr),
                    (
                        "nav/computer",
                        "Control del equipo",
                        &icons::MONITOR_COG,
                        Tab::Computer,
                    ),
                    ("nav/oracle", "Oráculo", &icons::SHIELD, Tab::Oracle),
                    ("nav/ethics", "Ética", &icons::SHIELD_ALERT, Tab::Ethics),
                ],
            ),
            (
                "USUARIO",
                &[("nav/profile", "Perfil", &icons::USER, Tab::Profile)],
            ),
            (
                "SISTEMA",
                &[
                    (
                        "nav/display",
                        "Pantalla y audio",
                        &icons::MONITOR_COG,
                        Tab::Display,
                    ),
                    (
                        "nav/updates",
                        "Actualizaciones",
                        &icons::UPDATE,
                        Tab::Updates,
                    ),
                ],
            ),
        ];
        for (title, items) in groups {
            c = c.child(section(title));
            for (key, label, icon, tab) in items.iter().copied() {
                let on = s.settings.tab == tab;
                let fg = if on {
                    th::blue()
                } else {
                    th::sidebar_foreground()
                };
                c = c.child(
                    row()
                        .key(key)
                        .gap(10.0)
                        .center_y()
                        .px(28.0)
                        .h(32.0)
                        .radius(8.0)
                        .fill(if on {
                            th::sidebar_accent()
                        } else {
                            th::sidebar().alpha(0.0)
                        })
                        .child(rune::icon(icon, 14.0).color(fg))
                        .child(text(label).font(type_::BODY.lh(18.0)).color(fg))
                        .tap(move |s: &mut State| crate::settings::view::select_tab(s, tab)),
                );
            }
        }
    }
    // Cerrar sesión, al pie, solo si hay una sesión con token.
    col()
        .key("sidebar")
        .w(if collapsed {
            th::SIDEBAR_COLLAPSED_W
        } else {
            th::SIDEBAR_W
        })
        .h_full()
        .fill(th::sidebar())
        .border(1.0, th::sidebar_border())
        .padding(8.0)
        .gap(4.0)
        .child(col().h(th::TOPBAR_H - 8.0))
        .child(
            c.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
        )
        .children(crate::has_session().then(|| {
            row()
                .key("set/sign-out")
                .gap(12.0)
                .center_y()
                .px(if collapsed { 0.0 } else { 16.0 })
                .h(36.0)
                .radius(10.0)
                .w_full()
                .child(rune::icon(&icons::PHONE_OFF, 16.0).color(th::sidebar_foreground()))
                .children((!collapsed).then(|| {
                    text("Cerrar sesión")
                        .font(type_::BODY.lh(20.0))
                        .color(th::sidebar_foreground())
                }))
                .tap(|s: &mut State| crate::sign_out(s))
        }))
}

pub fn topbar(s: &State) -> El {
    let (label, dot) = if s.chat_ready {
        ("WS: connected".to_string(), th::green())
    } else {
        (
            crate::header_line(s),
            if s.shown.chat_error.is_some() {
                th::red()
            } else {
                th::amber()
            },
        )
    };
    row()
        .key("topbar")
        .h(th::TOPBAR_H)
        .w_full()
        .px(16.0)
        .gap(12.0)
        .center_y()
        .fill(th::sidebar().alpha(0.0))
        .border(1.0, th::border_subtle())
        .child(
            row()
                .key("sidebar/toggle")
                .size(32.0, 32.0)
                .radius(radius::SM)
                .center()
                .child(rune::icon(&icons::PANEL_LEFT, 16.0).color(th::sidebar_foreground()))
                .tap(|s: &mut State| {
                    if s.win.0 < crate::NARROW {
                        s.sidebar_open_narrow = !s.sidebar_open_narrow
                    } else {
                        s.sidebar_collapsed = !s.sidebar_collapsed
                    }
                }),
        )
        .child(
            rune::el()
                .size(24.0, 24.0)
                .pixels_fit(&s.logo, rune::SurfaceFit::Fit),
        )
        .child(
            text("Hive")
                .font(type_::BODY.lh(20.0))
                .color(th::foreground())
                .w(30.0)
                .min_w(30.0)
                .truncate(),
        )
        // El logo de Rust (CC-BY), porque la app está hecha con Rune: negro sobre crema, blanco sobre oscuro.
        .child(rune::el().key("topbar/rust").size(20.0, 20.0).pixels_fit(
            if th::is_light() {
                &s.rust_dark
            } else {
                &s.rust_light
            },
            rune::SurfaceFit::Fit,
        ))
        .child(col().grow(1.0))
        .child(rune::el().size(8.0, 8.0).radius(4.0).fill(dot))
        .child(
            text(label.clone())
                .font(type_::LABEL)
                .color(th::muted_foreground())
                .truncate()
                .help(label),
        )
        .child(
            row()
                .key("theme/toggle")
                .size(32.0, 32.0)
                .radius(radius::SM)
                .center()
                .child(
                    rune::icon(
                        if th::is_light() {
                            &icons::MOON
                        } else {
                            &icons::SUN
                        },
                        16.0,
                    )
                    .color(th::sidebar_foreground()),
                )
                .tap(|_: &mut State| {
                    th::set_light(!th::is_light());
                    th::save();
                }),
        )
}

/// Pantalla completa: barra lateral a la izquierda y, a la derecha, la cabecera sobre la página.
pub fn frame(s: &State, page: El) -> El {
    let overlay = s.win.0 < crate::NARROW && s.sidebar_open_narrow;
    let nav = if overlay {
        col()
            .w(th::SIDEBAR_COLLAPSED_W)
            .h_full()
            .fill(th::sidebar())
    } else {
        sidebar(s)
    };
    let frame = row()
        .w_full()
        .h_full()
        .fill(th::background())
        .child(nav)
        .child(
            col()
                .grow(1.0)
                .h_full()
                .min_w(0.0)
                .child(topbar(s))
                .child(page),
        );
    if overlay {
        frame
            .child(
                col()
                    .key("sidebar/dismiss")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .fill(Color::BLACK.alpha(0.4))
                    .tap(|s: &mut State| s.sidebar_open_narrow = false),
            )
            .child(
                sidebar(s).absolute(0.0, 0.0).child(
                    row()
                        .key("sidebar/close-narrow")
                        .absolute(12.0, 8.0)
                        .size(30.0, 30.0)
                        .radius(6.0)
                        .center()
                        .child(rune::icon(&icons::PANEL_LEFT, 16.0).color(th::foreground()))
                        .tap(|s: &mut State| s.sidebar_open_narrow = false),
                ),
            )
    } else {
        frame
    }
}
