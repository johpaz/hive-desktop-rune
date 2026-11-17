//! La pestaña «Control del equipo» de Ajustes: dejar que el coordinador vea la pantalla y, aparte, que mueva el
//! ratón y teclee; con un botón de PARAR siempre a la vista. Todo vive en el gateway (`/api/computer`).
use crate::remote::Remote;
use crate::settings::api::Conn;
use crate::settings::view::{cardx, chip, header, icon_box, mono, pill, switch};
use crate::ui::{icons, text, theme as th, Tap};
use crate::State;
use rune::tok::{type_, TextStyle};
use rune::{col, row, El};
use serde_json::{json, Value};

#[derive(Default)]
pub struct Computer {
    pub remote: Remote,
    pub enabled: bool,
    pub allow_input: bool,
    pub backend: String,
    pub max_dimension: u64,
    pub max_actions: u64,
    pub minutes: u64,
}

impl Computer {
    pub fn sync(&mut self) -> bool {
        let (changed, fresh) = self.remote.sync();
        if fresh && self.remote.shown.error.is_none() {
            let s = self.remote.shown.value["settings"].clone();
            self.enabled = s["enabled"].as_bool().unwrap_or(false);
            self.allow_input = s["allow_input"].as_bool().unwrap_or(false);
            self.backend = s["backend"].as_str().unwrap_or("auto").to_string();
            self.max_dimension = s["max_dimension"].as_u64().unwrap_or(1280);
            self.max_actions = s["max_actions"].as_u64().unwrap_or(200);
            self.minutes = s["session_minutes"].as_u64().unwrap_or(10);
        }
        changed
    }
    pub fn load(&self, conn: &Conn) {
        self.remote.call(conn, "GET", "/api/computer", None, None);
    }
    pub fn body(&self) -> Value {
        json!({"enabled":self.enabled,"allow_input":self.allow_input,"backend":self.backend,"max_dimension":self.max_dimension,"max_actions":self.max_actions,"session_minutes":self.minutes})
    }
    pub fn dirty(&self) -> bool {
        let s = &self.remote.shown.value["settings"];
        self.remote.shown.loaded
            && (self.enabled != s["enabled"].as_bool().unwrap_or(false)
                || self.allow_input != s["allow_input"].as_bool().unwrap_or(false)
                || self.backend != s["backend"].as_str().unwrap_or("auto")
                || self.max_dimension != s["max_dimension"].as_u64().unwrap_or(1280)
                || self.max_actions != s["max_actions"].as_u64().unwrap_or(200)
                || self.minutes != s["session_minutes"].as_u64().unwrap_or(10))
    }
    pub fn stopped(&self) -> bool {
        self.remote.shown.value["stopped"]
            .as_bool()
            .unwrap_or(false)
    }
}

fn section_title(t: &str) -> El {
    row()
        .gap(8.0)
        .center_y()
        .child(
            text("⬢")
                .font(TextStyle::new(10.0, 700.0))
                .color(th::amber()),
        )
        .child(
            text(t.to_uppercase())
                .font(mono(11.0, 2.0))
                .color(th::amber()),
        )
}
fn hint(t: &str) -> El {
    text(t.to_string())
        .font(TextStyle::new(11.0, 400.0))
        .color(th::text_muted())
        .wrap()
}
fn card(key: &'static str, title: &str, body: El) -> El {
    cardx()
        .key(key)
        .w_full()
        .padding(20.0)
        .gap(12.0)
        .child(section_title(title))
        .child(body)
}
fn choices(
    prefix: &'static str,
    current: u64,
    values: &[u64],
    unit: &'static str,
    set: fn(&mut State, u64),
) -> El {
    let mut r = row().gap(8.0).flex_wrap();
    for v in values.iter().copied() {
        r = r.child(pill(
            format!("{v}{unit}"),
            format!("{prefix}/{v}"),
            current == v,
            move |s: &mut State| set(s, v),
        ));
    }
    r
}
fn button(
    key: &'static str,
    label: &str,
    fill: rune::Color,
    fg: rune::Color,
    press: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .px(18.0)
        .py(10.0)
        .radius(10.0)
        .fill(fill)
        .center()
        .child(
            text(label.to_string())
                .font(TextStyle::new(13.0, 700.0))
                .color(fg),
        )
        .tap(press)
}

pub fn page(s: &State) -> El {
    let c = &s.computer;
    let shown = &c.remote.shown;
    let actions = shown.value["actions"].as_u64().unwrap_or(0);
    let (tone, label) = if !shown.loaded {
        (th::muted_foreground(), "CARGANDO")
    } else if c.stopped() {
        (th::red(), "DETENIDO")
    } else if c.enabled {
        (
            th::green(),
            if c.allow_input {
                "VE Y ACTÚA"
            } else {
                "SOLO VE"
            },
        )
    } else {
        (th::text_muted(), "APAGADO")
    };
    let mut banner = col().w_full().gap(6.0);
    if let Some(e) = &shown.error {
        banner = banner.child(text(e.clone()).font(type_::LABEL).color(th::red()).wrap());
    }
    if let Some(n) = &shown.notice {
        banner = banner.child(text(n.clone()).font(type_::LABEL).color(th::green()).wrap());
    }
    if shown.unauthorized {
        banner = banner.child(
            text("La sesión venció: pulsa «Cerrar sesión» y vuelve a entrar.")
                .font(type_::LABEL)
                .color(th::red())
                .wrap(),
        );
    }
    let no_vision = shown.loaded && shown.value["model_has_vision"] == json!(false);
    let status = row().w_full().gap(12.0).center_y().flex_wrap()
        .child(icon_box(&icons::MONITOR_COG, tone, 40.0))
        .child(col().gap(3.0).grow(1.0).shrink()
            .child(row().gap(8.0).center_y().child(text("Control del equipo").font(TextStyle::new(15.0, 700.0)).color(th::foreground())).child(chip(label, tone)))
            .child(text(format!("{actions} acciones en esta sesión.")).font(type_::LABEL).color(th::muted_foreground()))
            .children(no_vision.then(|| text("El modelo del coordinador no ve imágenes: sin visión no puede usar la pantalla. Elige uno con visión en Agentes.").font(type_::LABEL).color(th::amber()).wrap())));
    let stop = row()
        .w_full()
        .gap(10.0)
        .flex_wrap()
        .center_y()
        .child(button(
            "computer/stop",
            "PARAR YA",
            th::red(),
            rune::Color::WHITE,
            |s: &mut State| {
                if let Some(conn) = s.settings.conn.clone() {
                    s.computer.remote.call(
                        &conn,
                        "POST",
                        "/api/computer/stop",
                        None,
                        Some("Detenido: no se ejecutará ninguna acción más"),
                    )
                }
            },
        ))
        .children(c.stopped().then(|| {
            button(
                "computer/resume",
                "Reanudar",
                th::glass_strong(),
                th::foreground(),
                |s: &mut State| {
                    if let Some(conn) = s.settings.conn.clone() {
                        s.computer.remote.call(
                            &conn,
                            "POST",
                            "/api/computer/resume",
                            None,
                            Some("Reanudado"),
                        )
                    }
                },
            )
        }))
        .child(button(
            "computer/reset",
            "Reiniciar contador",
            th::glass_strong(),
            th::foreground(),
            |s: &mut State| {
                if let Some(conn) = s.settings.conn.clone() {
                    s.computer.remote.call(
                        &conn,
                        "POST",
                        "/api/computer/reset",
                        None,
                        Some("Contador reiniciado"),
                    )
                }
            },
        ));
    let content = col().w_full().gap(16.0)
        .child(header("Escritorio", "Control", "del equipo", "Deja que el coordinador mire la pantalla y, solo si lo permites, mueva el ratón y escriba. Apagado por defecto; solo funciona con un modelo que vea imágenes.", Some(row().key("computer/refresh").gap(8.0).center_y().px(14.0).py(9.0).radius(10.0).fill(th::glass_strong()).border(1.0, th::border_base()).child(rune::icon(&icons::REFRESH, 13.0).color(th::muted_foreground())).child(text("ACTUALIZAR").font(mono(10.5, 1.2)).color(th::foreground())).tap(|s: &mut State| if let Some(conn) = s.settings.conn.clone() { s.computer.load(&conn) }))))
        .child(banner)
        .child(cardx().key("computer/status").w_full().padding(20.0).gap(14.0).child(status).child(stop))
        .child(card("computer/card-enable", "Permisos", col().w_full().gap(14.0)
            .child(row().w_full().gap(12.0).center_y().justify_between()
                .child(col().gap(3.0).shrink().grow(1.0).child(text("Permitir ver la pantalla").font(TextStyle::new(13.0, 600.0)).color(th::foreground())).child(hint("Capturas de pantalla. Se pide aprobación una vez por conversación.")))
                .child(switch("computer/enabled".into(), c.enabled, |s: &mut State| s.computer.enabled = !s.computer.enabled)))
            .child(row().w_full().gap(12.0).center_y().justify_between()
                .child(col().gap(3.0).shrink().grow(1.0).child(text("Permitir mover el ratón y escribir").font(TextStyle::new(13.0, 600.0)).color(th::foreground())).child(hint("Es un permiso aparte: actúa en tu equipo como si fueras tú. Requiere ver la pantalla.")))
                .child(switch("computer/input".into(), c.allow_input, |s: &mut State| s.computer.allow_input = !s.computer.allow_input)))))
        .child(card("computer/card-backend", "Cómo se controla", col().w_full().gap(8.0)
            .child(row().gap(8.0).flex_wrap().children(["auto", "x11", "portal"].map(|b| pill(b.to_uppercase(), format!("computer/backend/{b}"), c.backend == b, move |s: &mut State| s.computer.backend = b.to_string()))))
            .child(hint("Auto elige X11 o el portal de Wayland según tu sesión. En Wayland el sistema pedirá permiso la primera vez."))))
        .child(card("computer/card-limits", "Límites", col().w_full().gap(10.0)
            .child(text("Tamaño máximo de la captura").font(type_::LABEL).color(th::muted_foreground()))
            .child(choices("computer/dim", c.max_dimension, &[800, 1280, 1920], " px", |s, v| s.computer.max_dimension = v))
            .child(text("Acciones por sesión").font(type_::LABEL).color(th::muted_foreground()))
            .child(choices("computer/actions", c.max_actions, &[50, 200, 1000], "", |s, v| s.computer.max_actions = v))
            .child(text("La aprobación vale").font(type_::LABEL).color(th::muted_foreground()))
            .child(choices("computer/minutes", c.minutes, &[5, 10, 30], " min", |s, v| s.computer.minutes = v))))
        .child(row().gap(10.0).center_y()
            .child(button("computer/save", if c.remote.working() { "Guardando…" } else { "Guardar" }, if c.dirty() { th::amber() } else { th::glass_strong() }, if c.dirty() { th::on_amber() } else { th::muted_foreground() }, |s: &mut State| if let Some(conn) = s.settings.conn.clone() { let b = s.computer.body(); s.computer.remote.call(&conn, "PUT", "/api/computer", Some(b), Some("Guardado")) }))
            .children(c.dirty().then(|| text("Cambios sin guardar").font(type_::LABEL).color(th::amber()))));
    col().w_full().grow(1.0).min_h(0.0).child(
        col()
            .w_full()
            .padding(28.0)
            .gap(20.0)
            .child(content)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
    )
}
