//! La pestaña «Oráculo» de Ajustes: un segundo modelo que revisa la respuesta final del coordinador antes de que
//! la vea la persona (`/api/oracle`). Apagado por defecto; si falla, la respuesta sale igual. La clave se escribe
//! aquí, se guarda cifrada en el gateway y nunca vuelve: solo se muestra si existe.
use crate::remote::Remote;
use crate::settings::api::Conn;
use crate::settings::view::{cardx, chip, field, header, icon_box, mono, pill, switch};
use crate::ui::{icons, text, theme as th, Tap};
use crate::State;
use rune::tok::{type_, TextStyle};
use rune::{col, row, El, TextEdit};
use serde_json::{json, Value};

pub struct Oracle {
    pub remote: Remote,
    pub enabled: bool,
    pub endpoint: TextEdit,
    pub model: TextEdit,
    pub key: TextEdit,
    pub self_hosted: bool,
    pub timeout_ms: u64,
    pub corrections: u64,
    pub confidence: f64,
    pub support: f64,
}

impl Default for Oracle {
    fn default() -> Self {
        Self {
            remote: Remote::default(),
            enabled: false,
            endpoint: TextEdit::new(""),
            model: TextEdit::new(""),
            key: TextEdit::new("").secure(),
            self_hosted: false,
            timeout_ms: 5000,
            corrections: 2,
            confidence: 0.8,
            support: 0.5,
        }
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

impl Oracle {
    pub fn sync(&mut self) -> bool {
        let (changed, fresh) = self.remote.sync();
        if fresh && self.remote.shown.error.is_none() {
            let s = self.remote.shown.value["settings"].clone();
            self.enabled = s["enabled"].as_bool().unwrap_or(false);
            self.endpoint = TextEdit::new(s["endpoint"].as_str().unwrap_or_default());
            self.model = TextEdit::new(s["model"].as_str().unwrap_or_default());
            self.key = TextEdit::new("").secure();
            self.self_hosted = s["self_hosted"].as_bool().unwrap_or(false);
            self.timeout_ms = s["timeout_ms"].as_u64().unwrap_or(5000);
            self.corrections = s["max_corrections"].as_u64().unwrap_or(2);
            self.confidence = s["min_confidence"].as_f64().unwrap_or(0.8);
            self.support = s["support_threshold"].as_f64().unwrap_or(0.5);
        }
        changed
    }
    pub fn load(&self, conn: &Conn) {
        self.remote.call(conn, "GET", "/api/oracle", None, None);
    }
    pub fn has_key(&self) -> bool {
        self.remote.shown.value["has_api_key"]
            .as_bool()
            .unwrap_or(false)
    }
    /// La clave solo viaja si se escribió una nueva.
    pub fn body(&self) -> Value {
        let mut b = json!({"enabled":self.enabled,"endpoint":self.endpoint.text.trim(),"model":self.model.text.trim(),"self_hosted":self.self_hosted,
            "timeout_ms":self.timeout_ms,"max_corrections":self.corrections,"min_confidence":self.confidence,"support_threshold":self.support});
        let key = self.key.text.trim();
        if !key.is_empty() {
            b["api_key"] = json!(key);
        }
        b
    }
    pub fn dirty(&self) -> bool {
        let s = &self.remote.shown.value["settings"];
        self.remote.shown.loaded
            && (self.enabled != s["enabled"].as_bool().unwrap_or(false)
                || self.endpoint.text.trim() != s["endpoint"].as_str().unwrap_or_default()
                || self.model.text.trim() != s["model"].as_str().unwrap_or_default()
                || !self.key.text.trim().is_empty()
                || self.self_hosted != s["self_hosted"].as_bool().unwrap_or(false)
                || self.timeout_ms != s["timeout_ms"].as_u64().unwrap_or(5000)
                || self.corrections != s["max_corrections"].as_u64().unwrap_or(2)
                || !near(self.confidence, s["min_confidence"].as_f64().unwrap_or(0.8))
                || !near(self.support, s["support_threshold"].as_f64().unwrap_or(0.5)))
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
fn label(t: &str) -> El {
    text(t.to_string())
        .font(type_::LABEL)
        .color(th::muted_foreground())
}
fn row_switch(key: &'static str, title: &str, note: &str, on: bool, press: fn(&mut State)) -> El {
    row()
        .w_full()
        .gap(12.0)
        .center_y()
        .justify_between()
        .child(
            col()
                .gap(3.0)
                .shrink()
                .grow(1.0)
                .child(
                    text(title.to_string())
                        .font(TextStyle::new(13.0, 600.0))
                        .color(th::foreground()),
                )
                .child(hint(note)),
        )
        .child(switch(key.into(), on, press))
}

pub fn page(s: &State) -> El {
    let o = &s.oracle;
    let shown = &o.remote.shown;
    let live = &shown.value["status"];
    let cooling = live["cooling_down"].as_bool().unwrap_or(false);
    let (tone, state) = if !shown.loaded {
        (th::muted_foreground(), "CARGANDO")
    } else if !o.enabled {
        (th::text_muted(), "APAGADO")
    } else if cooling {
        (th::amber(), "EN PAUSA")
    } else {
        (th::green(), "ACTIVO")
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
    let decisions = live["decisions"].as_u64().unwrap_or(0);
    let status = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(icon_box(&icons::SHIELD, tone, 40.0))
        .child(
            col()
                .gap(3.0)
                .grow(1.0)
                .shrink()
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            text("Revisor de respuestas")
                                .font(TextStyle::new(15.0, 700.0))
                                .color(th::foreground()),
                        )
                        .child(chip(state, tone)),
                )
                .child(
                    text(if o.has_key() {
                        format!("Clave guardada · {decisions} revisiones desde que arrancó.")
                    } else {
                        "Sin clave guardada.".to_string()
                    })
                    .font(type_::LABEL)
                    .color(th::muted_foreground())
                    .wrap(),
                )
                .children(live["last_error"].as_str().map(|e| {
                    text(format!("Último error del oráculo: {e}"))
                        .font(type_::LABEL)
                        .color(th::amber())
                        .wrap()
                })),
        );
    let pills = |prefix: &'static str, current: f64, values: &[f64], set: fn(&mut State, f64)| {
        let mut r = row().gap(8.0).flex_wrap();
        for v in values.iter().copied() {
            r = r.child(pill(
                format!("{v}"),
                format!("{prefix}/{v}"),
                near(current, v),
                move |s: &mut State| set(s, v),
            ));
        }
        r
    };
    let content = col().w_full().gap(16.0)
        .child(header("Calidad", "Oráculo", "revisor", "Un segundo modelo mira la respuesta final antes de que la veas y, si no está respaldada por lo que devolvieron las herramientas, hace que el asistente la corrija. Es consultivo: si falla, la respuesta sale igual.", Some(row().key("oracle/refresh").gap(8.0).center_y().px(14.0).py(9.0).radius(10.0).fill(th::glass_strong()).border(1.0, th::border_base()).child(rune::icon(&icons::REFRESH, 13.0).color(th::muted_foreground())).child(text("ACTUALIZAR").font(mono(10.5, 1.2)).color(th::foreground())).tap(|s: &mut State| if let Some(conn) = s.settings.conn.clone() { s.oracle.load(&conn) }))))
        .child(banner)
        .child(cardx().key("oracle/status").w_full().padding(20.0).child(status))
        .child(card("oracle/card-enable", "Activación", col().w_full().gap(14.0)
            .child(row_switch("oracle/enabled", "Revisar las respuestas del coordinador", "Cada revisión es una llamada a otro modelo y gasta presupuesto de la tarea.", o.enabled, |s| s.oracle.enabled = !s.oracle.enabled))
            .child(row_switch("oracle/self-hosted", "Servidor propio", "Márcalo si el oráculo corre en tu red (permite direcciones locales).", o.self_hosted, |s| s.oracle.self_hosted = !s.oracle.self_hosted))))
        .child(card("oracle/card-endpoint", "Servicio", col().w_full().gap(10.0)
            .child(label("Dirección (endpoint)")).child(field(&o.endpoint, "oracle/endpoint".into(), "https://…", |s: &mut State| &mut s.oracle.endpoint))
            .child(label("Modelo")).child(field(&o.model, "oracle/model".into(), "nombre del modelo", |s: &mut State| &mut s.oracle.model))
            .child(label(if o.has_key() { "Clave de API (ya hay una guardada; escribe otra solo para reemplazarla)" } else { "Clave de API" })).child(field(&o.key, "oracle/key".into(), "••••••••", |s: &mut State| &mut s.oracle.key))
            .child(hint("La clave se guarda cifrada en el gateway y nunca se vuelve a mostrar."))))
        .child(card("oracle/card-rules", "Cuándo corrige", col().w_full().gap(10.0)
            .child(label("Correcciones como máximo por respuesta"))
            .child(row().gap(8.0).flex_wrap().children([1u64, 2, 3].map(|v| pill(v.to_string(), format!("oracle/corr/{v}"), o.corrections == v, move |s: &mut State| s.oracle.corrections = v))))
            .child(label("Confianza mínima para frenar una respuesta")).child(pills("oracle/conf", o.confidence, &[0.6, 0.8, 0.9], |s, v| s.oracle.confidence = v))
            .child(label("Una respuesta con menos respaldo que esto se revisa")).child(pills("oracle/support", o.support, &[0.3, 0.5, 0.7], |s, v| s.oracle.support = v))
            .child(label("Tiempo máximo de espera"))
            .child(row().gap(8.0).flex_wrap().children([3000u64, 5000, 10000].map(|v| pill(format!("{} s", v / 1000), format!("oracle/timeout/{v}"), o.timeout_ms == v, move |s: &mut State| s.oracle.timeout_ms = v))))))
        .child(row().gap(10.0).center_y()
            .child(row().key("oracle/save").px(18.0).py(10.0).radius(10.0).fill(if o.dirty() { th::amber() } else { th::glass_strong() }).center()
                .child(text(if o.remote.working() { "Guardando…" } else { "Guardar" }).font(TextStyle::new(13.0, 700.0)).color(if o.dirty() { th::on_amber() } else { th::muted_foreground() }))
                .tap(|s: &mut State| if let Some(conn) = s.settings.conn.clone() { let b = s.oracle.body(); s.oracle.remote.call(&conn, "PUT", "/api/oracle", Some(b), Some("Guardado")) }))
            .children(o.dirty().then(|| text("Cambios sin guardar").font(type_::LABEL).color(th::amber()))));
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
