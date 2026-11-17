//! La pestaña «OCR» de Ajustes: activar la lectura de documentos escaneados, elegir la carpeta que puede
//! leer y el proveedor que los lee, y decidir si hace falta aprobar cada envío.
//!
//! Todo vive en el gateway (`GET/PUT /api/ocr`); esta pantalla edita una copia y la guarda al pulsar
//! «Guardar», así un cambio a medias nunca llega al servidor. El gateway valida (la carpeta existe, el
//! proveedor sirve y tiene clave) y su mensaje se muestra tal cual.
use crate::settings::api::{request, Conn};
use crate::settings::view::{cardx, chip, field, header, icon_box, mono, pill, switch};
use crate::ui::{icons, text, theme as th, Tap};
use crate::State;
use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, El, TextEdit};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Un proveedor y modelo que podrían leer documentos.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    pub provider_id: String,
    pub model_id: String,
    pub name: String,
    /// `ocr` (endpoint dedicado) o `vision` (un modelo de chat que ve).
    pub kind: String,
    pub supported: bool,
    pub note: Option<String>,
}

impl Candidate {
    pub fn kind_label(&self) -> &'static str {
        if self.kind == "ocr" {
            "OCR dedicado"
        } else {
            "Modelo de visión"
        }
    }
    pub fn label(&self) -> String {
        format!(
            "{} · {}",
            self.provider_id,
            if self.name.is_empty() {
                &self.model_id
            } else {
                &self.name
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub folder: String,
    pub provider_id: String,
    pub model_id: String,
    pub ask: bool,
    pub session_minutes: u64,
    pub max_pages: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            folder: String::new(),
            provider_id: String::new(),
            model_id: String::new(),
            ask: true,
            session_minutes: 10,
            max_pages: 50,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub loaded: bool,
    pub settings: Settings,
    pub ready: bool,
    pub candidates: Vec<Candidate>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub unauthorized: bool,
    /// Sube cada vez que llegan ajustes del servidor: es la señal para volver a rellenar el formulario.
    pub generation: u64,
}

/// `GET/PUT /api/ocr` → ajustes, si está listo y los proveedores posibles.
pub fn parse(v: &Value) -> (Settings, bool, Vec<Candidate>) {
    let s = &v["settings"];
    let settings = Settings {
        enabled: s["enabled"].as_bool().unwrap_or(false),
        folder: s["folder"].as_str().unwrap_or_default().to_string(),
        provider_id: s["provider_id"].as_str().unwrap_or_default().to_string(),
        model_id: s["model_id"].as_str().unwrap_or_default().to_string(),
        ask: s["ask"].as_bool().unwrap_or(true),
        session_minutes: s["session_minutes"].as_u64().unwrap_or(10),
        max_pages: s["max_pages"].as_u64().unwrap_or(50),
    };
    let candidates = v["candidates"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| Candidate {
                    provider_id: c["provider_id"].as_str().unwrap_or_default().to_string(),
                    model_id: c["model_id"].as_str().unwrap_or_default().to_string(),
                    name: c["name"].as_str().unwrap_or_default().to_string(),
                    kind: c["kind"].as_str().unwrap_or("vision").to_string(),
                    supported: c["supported"].as_bool().unwrap_or(false),
                    note: c["note"].as_str().map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    (settings, v["ready"].as_bool().unwrap_or(false), candidates)
}

pub struct Ocr {
    data: Arc<Mutex<Data>>,
    pub shown: Data,
    busy: Arc<AtomicBool>,
    // El formulario: una copia editable de los ajustes.
    pub enabled: bool,
    pub folder: TextEdit,
    pub provider_id: String,
    pub model_id: String,
    pub ask: bool,
    pub minutes: u64,
    pub max_pages: u64,
    pub picker_open: bool,
    filled: u64,
}

impl Default for Ocr {
    fn default() -> Self {
        let defaults = Settings::default();
        Self {
            data: Arc::default(),
            shown: Data::default(),
            busy: Arc::default(),
            enabled: defaults.enabled,
            folder: TextEdit::new(""),
            provider_id: String::new(),
            model_id: String::new(),
            ask: defaults.ask,
            minutes: defaults.session_minutes,
            max_pages: defaults.max_pages,
            picker_open: false,
            filled: 0,
        }
    }
}

impl Ocr {
    fn fill(&mut self, s: &Settings) {
        self.enabled = s.enabled;
        self.folder = TextEdit::new(s.folder.clone());
        self.provider_id = s.provider_id.clone();
        self.model_id = s.model_id.clone();
        self.ask = s.ask;
        self.minutes = s.session_minutes;
        self.max_pages = s.max_pages;
    }

    /// Copia lo que dejaron los hilos; devuelve si cambió algo visible.
    pub fn sync(&mut self) -> bool {
        let now = self.data.lock().unwrap().clone();
        if now == self.shown {
            return false;
        }
        if now.generation != self.filled {
            self.fill(&now.settings);
            self.filled = now.generation;
        }
        self.shown = now;
        true
    }

    /// Lee los ajustes en un hilo (si no hay otra operación en curso).
    pub fn refresh(&self, conn: &Conn) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (conn, data, busy) = (conn.clone(), self.data.clone(), self.busy.clone());
        std::thread::spawn(move || {
            let r = request(&conn, "GET", "/api/ocr", None);
            apply(&data, r, None);
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// El cuerpo del `PUT`: lo que muestra el formulario.
    pub fn body(&self) -> Value {
        json!({"enabled":self.enabled,"folder":self.folder.text.trim(),"provider_id":self.provider_id,"model_id":self.model_id,
            "ask":self.ask,"session_minutes":self.minutes,"max_pages":self.max_pages})
    }

    pub fn save(&mut self, conn: &Conn) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut d = self.data.lock().unwrap();
            d.error = None;
            d.notice = None;
        }
        let (conn, data, busy, body) = (
            conn.clone(),
            self.data.clone(),
            self.busy.clone(),
            self.body(),
        );
        std::thread::spawn(move || {
            let r = request(&conn, "PUT", "/api/ocr", Some(body));
            apply(&data, r, Some("Guardado"));
            busy.store(false, Ordering::SeqCst);
        });
    }

    pub fn working(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// ¿Hay cambios sin guardar?
    pub fn dirty(&self) -> bool {
        let now = &self.shown.settings;
        self.enabled != now.enabled
            || self.folder.text.trim() != now.folder
            || self.provider_id != now.provider_id
            || self.model_id != now.model_id
            || self.ask != now.ask
            || self.minutes != now.session_minutes
            || self.max_pages != now.max_pages
    }

    fn chosen(&self) -> Option<&Candidate> {
        self.shown
            .candidates
            .iter()
            .find(|c| c.provider_id == self.provider_id && c.model_id == self.model_id)
    }

    /// Lo que falta para que funcione, en palabras (vacío si nada).
    pub fn missing(&self) -> Vec<&'static str> {
        let mut m = vec![];
        if self.folder.text.trim().is_empty() {
            m.push("elegir la carpeta de documentos");
        }
        if self.chosen().is_none() {
            m.push("elegir un proveedor de OCR");
        } else if !self.chosen().is_some_and(|c| c.supported) {
            m.push("elegir un proveedor con adaptador (el elegido aún no lo tiene)");
        }
        m
    }
}

fn apply(
    data: &Arc<Mutex<Data>>,
    r: Result<Value, crate::settings::api::ApiError>,
    notice: Option<&str>,
) {
    let mut d = data.lock().unwrap();
    match r {
        Ok(v) => {
            let (settings, ready, candidates) = parse(&v);
            d.settings = settings;
            d.ready = ready;
            d.candidates = candidates;
            d.loaded = true;
            d.error = None;
            d.notice = notice.map(str::to_string);
            d.generation += 1;
        }
        Err(e) => {
            d.unauthorized |= e == crate::settings::api::ApiError::Unauthorized;
            d.loaded = true;
            d.error = Some(e.to_string());
        }
    }
}

// ─── Pantalla ────────────────────────────────────────────────────────────────

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

fn banner(o: &Ocr) -> El {
    let mut b = col().w_full().gap(6.0);
    if let Some(e) = &o.shown.error {
        b = b.child(text(e.clone()).font(type_::LABEL).color(th::red()).wrap());
    }
    if let Some(n) = &o.shown.notice {
        b = b.child(text(n.clone()).font(type_::LABEL).color(th::green()).wrap());
    }
    if o.shown.unauthorized {
        b = b.child(
            text("La sesión venció: pulsa «Cerrar sesión» y vuelve a entrar.")
                .font(type_::LABEL)
                .color(th::red())
                .wrap(),
        );
    }
    if !o.shown.loaded {
        b = b.child(
            text("Cargando…")
                .font(type_::LABEL)
                .color(th::muted_foreground()),
        );
    }
    b
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

/// Un selector con la lista de proveedores posibles; los que aún no tienen adaptador se ven pero no se eligen.
fn provider_picker(o: &Ocr) -> El {
    if o.shown.candidates.is_empty() {
        return col().w_full().gap(6.0)
            .child(text("No hay ningún proveedor que sirva todavía.").font(type_::BODY).color(th::foreground()))
            .child(hint("Activa en Proveedores uno con su clave guardada y un modelo de OCR o de visión; después pulsa «Actualizar»."));
    }
    let shown = match o.chosen() {
        Some(c) => format!("{} — {}", c.label(), c.kind_label()),
        None if o.provider_id.is_empty() => "Elige un proveedor".to_string(),
        None => "El proveedor elegido ya no está disponible".to_string(),
    };
    let mut c = col().w_full().gap(6.0).child(
        row()
            .key("ocr/provider")
            .w_full()
            .px(12.0)
            .py(10.0)
            .radius(radius::SM)
            .fill(th::glass_strong())
            .border(1.0, th::border_base())
            .center_y()
            .child(
                text(shown)
                    .font(type_::BODY)
                    .color(th::foreground())
                    .grow(1.0),
            )
            .child(rune::icon(&icons::CHEVRON_DOWN, 14.0).color(th::muted_foreground()))
            .tap(|s: &mut State| s.ocr.picker_open = !s.ocr.picker_open),
    );
    if o.picker_open {
        let mut list = col()
            .w_full()
            .radius(radius::SM)
            .fill(th::card())
            .border(1.0, th::border_base())
            .py(4.0);
        for (i, cand) in o.shown.candidates.iter().enumerate() {
            let (provider, model, supported) = (
                cand.provider_id.clone(),
                cand.model_id.clone(),
                cand.supported,
            );
            let tone = if supported {
                th::foreground()
            } else {
                th::text_muted()
            };
            let mut item = col()
                .key(format!("ocr/provider/{i}"))
                .w_full()
                .px(12.0)
                .py(8.0)
                .gap(2.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(text(cand.label()).font(type_::BODY).color(tone))
                        .child(chip(
                            cand.kind_label(),
                            if cand.kind == "ocr" {
                                th::blue()
                            } else {
                                th::amber()
                            },
                        )),
                );
            if let Some(note) = &cand.note {
                item = item.child(hint(note));
            }
            list = list.child(item.tap(move |s: &mut State| {
                if supported {
                    s.ocr.provider_id = provider.clone();
                    s.ocr.model_id = model.clone();
                    s.ocr.picker_open = false;
                }
            }));
        }
        c = c.child(list);
    }
    c
}

/// Una fila de opciones numéricas en píldora (5 · 10 · 30 minutos).
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
            format!("{v} {unit}"),
            format!("{prefix}/{v}"),
            current == v,
            move |s: &mut State| set(s, v),
        ));
    }
    r
}

pub fn page(s: &State) -> El {
    let o = &s.ocr;
    let missing = o.missing();
    let (tone, label) = if !o.shown.loaded {
        (th::muted_foreground(), "CARGANDO")
    } else if o.shown.ready {
        (th::green(), "LISTO")
    } else if o.shown.settings.enabled {
        (th::amber(), "INCOMPLETO")
    } else {
        (th::text_muted(), "APAGADO")
    };
    let status = row()
        .w_full()
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(icon_box(&icons::FILE_TEXT, tone, 40.0))
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
                            text("Lectura de documentos")
                                .font(TextStyle::new(15.0, 700.0))
                                .color(th::foreground()),
                        )
                        .child(chip(label, tone)),
                )
                .child(
                    text(if o.shown.ready {
                        "El coordinador puede leer los documentos de la carpeta elegida."
                            .to_string()
                    } else if missing.is_empty() {
                        "Guarda los cambios para activarlo.".to_string()
                    } else {
                        format!("Falta: {}.", missing.join(" y "))
                    })
                    .font(type_::LABEL)
                    .color(th::muted_foreground())
                    .wrap(),
                ),
        );

    let content = col().w_full().gap(16.0)
        .child(header("Lectura de documentos", "OCR", "de documentos", "Convierte PDFs escaneados e imágenes con texto en texto que el asistente puede usar. Solo el coordinador lo usa, y solo lee la carpeta que elijas.", Some(refresh(o))))
        .child(banner(o))
        .child(cardx().key("ocr/status").w_full().padding(20.0).child(status))
        .child(card("ocr/card-enable", "Activación", row().w_full().gap(12.0).center_y().justify_between()
            .child(col().gap(3.0).shrink().grow(1.0).child(text("Permitir que el asistente lea documentos").font(TextStyle::new(13.0, 600.0)).color(th::foreground())).child(hint("Apagado por defecto. Un PDF que ya trae texto se lee sin enviarlo a ningún sitio.")))
            .child(switch("ocr/enabled".into(), o.enabled, |s: &mut State| s.ocr.enabled = !s.ocr.enabled))))
        .child(card("ocr/card-folder", "Carpeta de documentos", col().w_full().gap(8.0)
            .child(field(&o.folder, "ocr/folder".into(), "/ruta/absoluta/a/tus/documentos", |s: &mut State| &mut s.ocr.folder))
            .child(hint("El asistente solo puede leer archivos dentro de esta carpeta (PDF, png, jpg, webp y gif de hasta 25 MB). Nada fuera de ella."))))
        .child(card("ocr/card-provider", "Proveedor que lee los escaneos", col().w_full().gap(8.0)
            .child(provider_picker(o))
            .child(hint("Un modelo de OCR dedicado suele ser más fiel y barato; un modelo de visión sirve cuando no hay otro. Las claves se guardan en Proveedores, no aquí."))))
        .child(card("ocr/card-privacy", "Privacidad", col().w_full().gap(12.0)
            .child(row().w_full().gap(12.0).center_y().justify_between()
                .child(col().gap(3.0).shrink().grow(1.0).child(text("Pedir aprobación antes de enviar un documento").font(TextStyle::new(13.0, 600.0)).color(th::foreground())).child(hint("Se pregunta una vez por conversación y, si aceptas, vale unos minutos. Un escaneo sale de tu equipo hacia el proveedor.")))
                .child(switch("ocr/ask".into(), o.ask, |s: &mut State| s.ocr.ask = !s.ocr.ask)))
            .children(o.ask.then(|| col().gap(6.0).child(text("La aprobación vale").font(type_::LABEL).color(th::muted_foreground())).child(choices("ocr/minutes", o.minutes, &[5, 10, 30], "min", |s, v| s.ocr.minutes = v))))))
        .child(card("ocr/card-limits", "Límites", col().w_full().gap(8.0)
            .child(text("Páginas por lectura").font(type_::LABEL).color(th::muted_foreground()))
            .child(choices("ocr/pages", o.max_pages, &[10, 50, 100], "pág.", |s, v| s.ocr.max_pages = v))))
        .child(row().gap(10.0).center_y()
            .child(row().key("ocr/save").px(18.0).py(10.0).radius(10.0).fill(if o.dirty() { th::amber() } else { th::glass_strong() }).center()
                .child(text(if o.working() { "Guardando…" } else { "Guardar" }).font(TextStyle::new(13.0, 700.0)).color(if o.dirty() { th::on_amber() } else { th::muted_foreground() }))
                .tap(|s: &mut State| if let Some(conn) = s.settings.conn.clone() { s.ocr.save(&conn) }))
            .children(o.dirty().then(|| row().key("ocr/discard").px(18.0).py(10.0).radius(10.0).fill(th::glass_strong()).border(1.0, th::border_base()).center()
                .child(text("Descartar").font(TextStyle::new(13.0, 500.0)).color(th::foreground()))
                .tap(|s: &mut State| { let now = s.ocr.shown.settings.clone(); s.ocr.fill(&now); s.ocr.picker_open = false; })))
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

fn refresh(o: &Ocr) -> El {
    row()
        .key("ocr/refresh")
        .gap(8.0)
        .center_y()
        .px(14.0)
        .py(9.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(rune::icon(&icons::REFRESH, 13.0).color(th::muted_foreground()))
        .child(
            text(if o.working() {
                "TRABAJANDO…".to_string()
            } else {
                "ACTUALIZAR".to_string()
            })
            .font(mono(10.5, 1.2))
            .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            if let Some(conn) = s.settings.conn.clone() {
                s.ocr.refresh(&conn)
            }
        })
}
