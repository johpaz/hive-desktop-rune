//! Vista de Rune para las superficies A2UI. Estado local (campos, pestañas, modales) en `A2State`;
//! las acciones del usuario salen por `outbox` como frames `a2ui:action`.

use crate::ui::Tap;
use std::collections::{HashMap, HashSet};

use rune::color::hex;
use rune::tok::{radius, type_};
use rune::{col, el, row, text, text_input, El, TextEdit};
use serde_json::{json, Value};

use super::model::*;
use crate::State;

#[derive(Default)]
pub struct A2State {
    pub store: Store,
    /// Un editor por campo de texto; la clave es `superficie/componente` (o con la ruta de plantilla).
    pub edits: HashMap<String, TextEdit>,
    pub tabs: HashMap<String, String>,
    pub open: HashSet<String>,
    /// Frames `a2ui:action` por enviar al gateway.
    pub outbox: Vec<Value>,
    pub images: super::images::Images,
    pub audio: super::audio::Audio,
    /// Posición de la conversación (cantidad de mensajes) en la que nació cada superficie, para
    /// pintarla dentro del hilo y no aparte.
    pub anchors: HashMap<String, usize>,
}

fn is_text_input(kind: &str) -> bool {
    matches!(kind, "TextField" | "DateTimeInput")
}

fn new_edit(def: &Value, initial: String) -> TextEdit {
    let edit = TextEdit::new(initial);
    let obscured = ["variant", "textFieldType"]
        .iter()
        .any(|k| def.get(*k).and_then(Value::as_str) == Some("obscured"));
    if obscured {
        edit.secure()
    } else {
        edit
    }
}

impl A2State {
    /// Aplica un frame `a2ui:*` y reconcilia los campos de texto con el modelo.
    pub fn apply(&mut self, kind: &str, data: &Value) -> bool {
        let changed = self.store.apply(kind, data);
        if changed {
            self.sync_edits();
        }
        changed
    }

    /// El modelo manda: un campo ligado a una ruta absoluta muestra lo que dice el modelo
    /// (a diferencia de la UI web, donde una edición local sobrevive a una actualización del servidor).
    fn sync_edits(&mut self) {
        let mut wanted: Vec<(String, Value, String)> = vec![];
        for s in &self.store.surfaces {
            for (id, def) in &s.components {
                let kind = def.get("component").and_then(Value::as_str).unwrap_or("");
                let path = def
                    .get("value")
                    .and_then(|v| v.get("path"))
                    .and_then(Value::as_str);
                if let (true, Some(p)) = (is_text_input(kind), path) {
                    if p.starts_with('/') {
                        let text = dyn_string(Some(&json!({"path": p})), &Ctx::new(&s.data, None));
                        wanted.push((format!("{}/{}", s.id, id), def.clone(), text));
                    }
                }
            }
        }
        for (key, def, text) in wanted {
            let edit = self
                .edits
                .entry(key)
                .or_insert_with(|| new_edit(&def, String::new()));
            if edit.text != text {
                edit.replace_text(text);
            }
        }
        let live: HashSet<&str> = self.store.surfaces.iter().map(|s| s.id.as_str()).collect();
        self.edits
            .retain(|k, _| live.contains(k.split('/').next().unwrap_or("")));
    }

    /// Escribe en el modelo local. Solo rutas absolutas: una relativa no tiene dónde escribirse.
    pub fn write(&mut self, surface: &str, path: &str, value: Value) {
        if !path.starts_with('/') {
            return;
        }
        if let Some(s) = self.store.get_mut(surface) {
            set_path(&mut s.data, path, Some(value));
        }
        self.sync_edits();
    }

    pub fn fire(&mut self, surface: &str, source: &str, name: &str, context: Value) {
        self.outbox
            .push(action_frame(name, surface, source, context));
    }
}

struct R<'a> {
    st: &'a A2State,
    s: &'a Surface,
}

/// Pinta una superficie con su encabezado (nombre del agente o id) y su árbol de componentes.
pub fn surface_view(st: &A2State, s: &Surface) -> El {
    let t = crate::ui::theme::tok();
    let accent = s
        .theme
        .get("primaryColor")
        .and_then(Value::as_str)
        .and_then(hex)
        .unwrap_or_else(crate::ui::theme::accent_tok);
    let title = s
        .theme
        .get("agentDisplayName")
        .and_then(Value::as_str)
        .unwrap_or(&s.id)
        .to_uppercase();
    let r = R { st, s };
    let body = match &s.root {
        Some(root) if s.components.contains_key(root) => node(&r, root, None, &mut vec![], root),
        _ => text("Sin contenido disponible")
            .font(type_::LABEL)
            .color(t.text_secondary),
    };
    col()
        .key(format!("a2:{}", s.id))
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .radius(radius::SM)
        .fill(t.card)
        .border(1.0, accent.alpha(0.25))
        .child(text(title).font(type_::LABEL).color(accent))
        .child(body)
}

fn node(r: &R, id: &str, scope: Option<&Value>, anc: &mut Vec<String>, key: &str) -> El {
    let t = crate::ui::theme::tok();
    let Some(def) = r.s.component(id) else {
        return col();
    };
    if anc.iter().any(|a| a == id) || anc.len() >= 48 {
        return text(format!(
            "Componente \"{id}\" omitido: se contiene a sí mismo"
        ))
        .font(type_::LABEL)
        .color(t.text_secondary);
    }
    anc.push(id.to_string());
    let el = build(r, def, id, scope, anc, key);
    anc.pop();
    match def.get("weight").and_then(Value::as_f64) {
        Some(w) => el.grow(w as f32),
        None => el,
    }
}

fn kids(r: &R, def: &Value, scope: Option<&Value>, anc: &mut Vec<String>, key: &str) -> Vec<El> {
    let ctx = Ctx::new(&r.s.data, scope);
    let Some(children) = def.get("children") else {
        return vec![];
    };
    children_of(children, &ctx)
        .into_iter()
        .map(|c| node(r, &c.id, c.scope.as_ref(), anc, &format!("{key}>{}", c.key)))
        .collect()
}

fn icon_glyph(name: &str) -> &'static str {
    match name {
        "check" => "✓",
        "close" => "✕",
        "add" => "+",
        "star" => "★",
        "home" => "⌂",
        "search" => "⌕",
        "mail" => "✉",
        "call" => "☎",
        "settings" => "⚙",
        "warning" => "⚠",
        "info" => "ⓘ",
        "error" => "⊗",
        "favorite" => "♥",
        "arrowBack" => "←",
        "arrowForward" => "→",
        "delete" => "✕",
        "edit" => "✎",
        "refresh" => "↻",
        "send" => "➤",
        "person" | "accountCircle" => "☺",
        "lock" => "🔒",
        _ => "?",
    }
}

fn build(
    r: &R,
    def: &Value,
    id: &str,
    scope: Option<&Value>,
    anc: &mut Vec<String>,
    key: &str,
) -> El {
    let t = crate::ui::theme::tok();
    let accent = crate::ui::theme::accent_tok();
    let red = hex("#E5484D").unwrap_or(accent);
    let ctx = Ctx::new(&r.s.data, scope);
    let sid = r.s.id.clone();
    let ek = |suffix: &str| format!("{}/{}{suffix}", r.s.id, key);
    let kind: String = def
        .get("component")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(char::is_ascii)
        .collect();
    let str_of = |k: &str| dyn_string(def.get(k), &ctx);
    let model_path = def
        .get("value")
        .and_then(|v| v.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let action = def.get("action").and_then(action_event);
    let this = id.to_string();

    match kind.trim() {
        "Text" => {
            let variant = def
                .get("variant")
                .or_else(|| def.get("usageHint"))
                .and_then(Value::as_str)
                .unwrap_or("body");
            let content = str_of("text");
            match variant {
                "code" => col()
                    .px(8.0)
                    .py(6.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .child(text(content).font(type_::BODY).color(t.text)),
                "caption" | "label" => text(content)
                    .font(type_::LABEL)
                    .color(t.text_secondary)
                    .wrap(),
                "h1" | "h2" | "h3" => text(content).font(type_::TITLE).color(t.text).wrap(),
                // Texto normal: el mismo parser de markdown del chat (negrita, cursiva, código, listas, enlaces).
                _ => rune_kit::markdown::Markdown::new(content)
                    .scroll_wide(true)
                    .render(&rune_kit::markdown::MarkdownStyle::neutral(
                        !crate::ui::theme::is_light(),
                    )),
            }
        }
        "Row" | "Column" => {
            let is_row = kind.trim() == "Row";
            let mut e = if is_row {
                row().gap(8.0)
            } else {
                col().gap(12.0)
            };
            let justify = def
                .get("justify")
                .or_else(|| def.get("distribution"))
                .and_then(Value::as_str)
                .unwrap_or("start");
            e = match justify {
                "center" => e.justify_center(),
                "end" => e.justify_end(),
                "spaceBetween" => e.justify_between(),
                "spaceAround" => e.justify_around(),
                "spaceEvenly" => e.justify_evenly(),
                _ => e.justify_start(),
            };
            let align = def
                .get("align")
                .or_else(|| def.get("alignment"))
                .and_then(Value::as_str)
                .unwrap_or("start");
            e = match align {
                "center" => e.items_center(),
                "end" => e.items_end(),
                "stretch" => e.items_stretch(),
                _ => e.items_start(),
            };
            kids(r, def, scope, anc, key)
                .into_iter()
                .fold(e, |e, c| e.child(c))
        }
        "Card" => {
            let inner = def
                .get("child")
                .and_then(Value::as_str)
                .map(|c| node(r, c, scope, anc, &format!("{key}>{c}")));
            let card = col()
                .px(12.0)
                .py(10.0)
                .radius(radius::SM)
                .fill(t.control_off)
                .border(1.0, t.separator);
            match inner {
                Some(c) => card.child(c),
                None => card,
            }
        }
        "List" => {
            let horizontal = def.get("direction").and_then(Value::as_str) == Some("horizontal");
            let e = if horizontal {
                row().gap(8.0)
            } else {
                col().gap(8.0)
            };
            let mut items = kids(r, def, scope, anc, key);
            if items.is_empty() {
                if let Some(c) = def.get("child").and_then(Value::as_str) {
                    items.push(node(r, c, scope, anc, &format!("{key}>{c}")));
                }
            }
            items.into_iter().fold(e, |e, c| e.child(c))
        }
        "Divider" => {
            if def.get("axis").and_then(Value::as_str) == Some("vertical") {
                el().w(1.0).h_full().fill(t.separator)
            } else {
                el().w_full().h(1.0).fill(t.separator)
            }
        }
        "Image" => {
            use super::images::Status;
            let url = str_of("url");
            if url.is_empty() {
                return col();
            }
            let alt = def.get("alt").and_then(Value::as_str).unwrap_or("imagen");
            let hint = def.get("usageHint").and_then(Value::as_str).unwrap_or("");
            // Ancho máximo por pista; el alto sale de la proporción de la imagen.
            let (width, round): (f32, bool) = match hint {
                "icon" => (32.0, false),
                "avatar" => (48.0, true),
                "smallFeature" => (96.0, false),
                "mediumFeature" => (192.0, false),
                "largeFeature" => (256.0, false),
                "header" => (480.0, false),
                _ => (320.0, false),
            };
            let slot = r.st.images.get(&url);
            match slot.status {
                Status::Loading => col()
                    .size(width.min(120.0), 48.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .center()
                    .child(text("Cargando…").font(type_::LABEL).color(t.text_secondary)),
                Status::Failed(e) => col()
                    .px(10.0)
                    .py(8.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .child(
                        text(format!("[{alt}] {e}"))
                            .font(type_::LABEL)
                            .color(t.text_secondary)
                            .wrap(),
                    ),
                Status::Ready => {
                    let fit = match def.get("fit").and_then(Value::as_str) {
                        Some("cover") => Some(rune::SurfaceFit::Fill),
                        Some("fill") => None, // estirar a la caja
                        _ => Some(rune::SurfaceFit::Fit),
                    };
                    let e = el()
                        .key(ek(""))
                        .w(width)
                        .radius(if round { width / 2.0 } else { 6.0 });
                    match fit {
                        Some(f) => e.pixels_fit(&slot.pixels, f),
                        None => e.pixels(&slot.pixels),
                    }
                }
            }
        }
        "Icon" => text(icon_glyph(&str_of("name")))
            .font(type_::BODY)
            .color(t.text),
        "AudioPlayer" => {
            use super::audio::Clip;
            let url = str_of("url");
            if url.is_empty() {
                return col();
            }
            let desc = str_of("description");
            let a = &r.st.audio;
            let (playing, loading) = (a.is_playing(&url), a.is_loading(&url));
            let label = if playing {
                "■ Detener"
            } else if loading {
                "Cargando…"
            } else {
                "▶ Reproducir"
            };
            let secs = match a.clips.peek(&url) {
                Some(Clip::Ready { seconds, .. }) => format!(" · {} s", seconds.round() as u32),
                _ => String::new(),
            };
            let title = if desc.is_empty() { url.clone() } else { desc };
            let u = url.clone();
            let mut c = col().gap(4.0).child(
                row()
                    .gap(10.0)
                    .center_y()
                    .px(10.0)
                    .py(8.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .child(
                        row()
                            .key(ek(""))
                            .px(12.0)
                            .py(6.0)
                            .radius(radius::SM)
                            .fill(accent.alpha(0.5))
                            .center()
                            .child(text(label).font(type_::LABEL).color(t.text))
                            .tap(move |s: &mut State| {
                                // Solo suena con un clic: el agente elige la URL, la persona decide oírla.
                                if playing {
                                    s.a2.audio.stop();
                                } else if !loading {
                                    s.a2.audio.play(&u);
                                }
                            }),
                    )
                    .child(
                        text(format!("{title}{secs}"))
                            .font(type_::LABEL)
                            .color(t.text_secondary)
                            .wrap()
                            .grow(1.0),
                    ),
            );
            if a.error_url.as_deref() == Some(url.as_str()) {
                if let Some(e) = &a.error {
                    c = c.child(text(e.clone()).font(type_::LABEL).color(red).wrap());
                }
            }
            c
        }
        "Video" => {
            let url = str_of("url");
            if url.is_empty() {
                return col();
            }
            // No hay decodificador de vídeo: se ofrece abrirlo en el navegador, y solo si la URL es pública.
            match super::images::external_link(&url) {
                Ok((full, host)) => row()
                    .gap(10.0)
                    .center_y()
                    .px(10.0)
                    .py(8.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .child(
                        row()
                            .key(ek(""))
                            .px(12.0)
                            .py(6.0)
                            .radius(radius::SM)
                            .fill(accent.alpha(0.5))
                            .center()
                            .child(text("▶ Abrir vídeo").font(type_::LABEL).color(t.text))
                            .tap(move |s: &mut State| {
                                if let Err(e) = super::images::open_in_browser(&full) {
                                    s.a2.audio.error = Some(e);
                                }
                            }),
                    )
                    .child(
                        text(format!("en el navegador · {host}"))
                            .font(type_::LABEL)
                            .color(t.text_secondary)
                            .wrap()
                            .grow(1.0),
                    ),
                Err(e) => col()
                    .px(10.0)
                    .py(8.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .child(
                        text(format!("Video no disponible: {e}"))
                            .font(type_::LABEL)
                            .color(t.text_secondary)
                            .wrap(),
                    ),
            }
        }
        "Button" => {
            let disabled = def
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || !run_checks(def.get("checks"), &ctx).is_empty();
            let label = match def.get("child").and_then(Value::as_str) {
                Some(c) => node(r, c, scope, anc, &format!("{key}>{c}")),
                None => text(if def.get("text").is_some() {
                    str_of("text")
                } else {
                    "Button".into()
                })
                .font(type_::BODY)
                .color(t.text),
            };
            let variant = def
                .get("variant")
                .and_then(Value::as_str)
                .unwrap_or("primary");
            let fill = match variant {
                "primary" => accent.alpha(if disabled { 0.25 } else { 0.9 }),
                "borderless" => t.card.alpha(0.0),
                _ => t.control_off,
            };
            let b = row()
                .key(ek(""))
                .px(14.0)
                .py(8.0)
                .radius(radius::SM)
                .fill(fill)
                .center()
                .child(label);
            match (disabled, action) {
                (false, Some((name, context))) => {
                    let sc = scope.cloned();
                    b.tap(move |s: &mut State| {
                        let Some(surface) = s.a2.store.get(&sid) else {
                            return;
                        };
                        let resolved =
                            resolve_context(&context, &Ctx::new(&surface.data, sc.as_ref()));
                        s.a2.fire(&sid, &this, &name, resolved);
                    })
                }
                _ => b,
            }
        }
        "TextField" | "DateTimeInput" => {
            let is_date = kind.trim() == "DateTimeInput";
            let label = str_of("label");
            let edit_key = if scope.is_none() {
                format!("{}/{}", r.s.id, id)
            } else {
                ek("")
            };
            let initial = if is_date {
                String::new()
            } else {
                str_of("value")
            };
            let temp;
            let edit = match r.st.edits.get(&edit_key) {
                Some(e) => e,
                None => {
                    temp = new_edit(def, initial);
                    &temp
                }
            };
            let multiline = ["variant", "textFieldType"].iter().any(|k| {
                matches!(
                    def.get(*k).and_then(Value::as_str),
                    Some("longText" | "multiline" | "code" | "multiline_code")
                )
            });
            let placeholder = if is_date {
                match (
                    dyn_bool(def.get("enableDate"), &ctx),
                    dyn_bool(def.get("enableTime"), &ctx),
                ) {
                    (true, true) => "yyyy-MM-ddTHH:mm".to_string(),
                    (true, false) => "yyyy-MM-dd".to_string(),
                    _ => "HH:mm".to_string(),
                }
            } else {
                str_of("placeholder")
            };
            let (k1, k2, k3) = (edit_key.clone(), edit_key.clone(), edit_key.clone());
            let (p1, s1) = (model_path.clone(), sid.clone());
            let mut field = text_input(edit, type_::BODY.lh(26.0))
                .key(edit_key.clone())
                .grow(1.0)
                .color(t.text)
                .placeholder(placeholder, t.text_secondary)
                .on_edit(move |s: &mut State| {
                    s.a2.edits
                        .entry(k1.clone())
                        .or_insert_with(|| TextEdit::new(""))
                })
                .on_input_change(move |s: &mut State, _| {
                    let text =
                        s.a2.edits
                            .get(&k2)
                            .map(|e| e.text.clone())
                            .unwrap_or_default();
                    if let Some(p) = &p1 {
                        s.a2.write(&s1, p, json!(text));
                    }
                });
            if !multiline {
                field = field.single_line();
                if let Some((name, _)) = action {
                    let (sid2, this2) = (sid.clone(), this.clone());
                    // La acción de un campo lleva el texto actual; su `context` no se usa.
                    field = field.on_submit(move |s: &mut State, _| {
                        let text =
                            s.a2.edits
                                .get(&k3)
                                .map(|e| e.text.clone())
                                .unwrap_or_default();
                        s.a2.fire(&sid2, &this2, &name, json!({"value": text}));
                    });
                }
            }
            let current = edit.text.clone();
            let mut errors = run_checks(def.get("checks"), &ctx);
            if def.get("checks").is_none() && !current.is_empty() {
                if let Some(re) = def.get("validationRegexp").and_then(Value::as_str) {
                    if !regex::Regex::new(re).is_ok_and(|r| r.is_match(&current)) {
                        errors = vec!["Formato inválido".into()];
                    }
                }
            }
            let (with_date, with_time) = (
                dyn_bool(def.get("enableDate"), &ctx),
                dyn_bool(def.get("enableTime"), &ctx),
            );
            if is_date
                && def.get("checks").is_none()
                && !current.is_empty()
                && !valid_datetime(&current, with_date, with_time)
            {
                errors = vec![format!(
                    "Usa el formato {}",
                    datetime_format(with_date, with_time)
                )];
            }
            let mut c = col().gap(4.0).w_full();
            if !label.is_empty() {
                c = c.child(text(label).font(type_::LABEL).color(t.text_secondary));
            }
            let mut input_row = row()
                .w_full()
                .gap(8.0)
                .center_y()
                .px(10.0)
                .py(6.0)
                .radius(radius::SM)
                .fill(t.control_off)
                .child(field);
            if let (true, Some(p)) = (is_date, model_path.clone()) {
                let s3 = sid.clone();
                input_row = input_row.child(
                    row()
                        .key(ek("now"))
                        .px(8.0)
                        .py(3.0)
                        .radius(radius::SM)
                        .fill(accent.alpha(0.35))
                        .child(
                            text(if with_date { "Hoy" } else { "Ahora" })
                                .font(type_::LABEL)
                                .color(t.text),
                        )
                        .tap(move |s: &mut State| {
                            s.a2.write(&s3, &p, json!(now_text(with_date, with_time)))
                        }),
                );
            }
            c = c.child(input_row);
            match errors.first() {
                Some(m) => c.child(text(m.clone()).font(type_::LABEL).color(red)),
                None => c,
            }
        }
        "CheckBox" => {
            let on = dyn_bool(def.get("value"), &ctx);
            let mark = el()
                .size(18.0, 18.0)
                .radius(4.0)
                .fill(if on { accent } else { t.control_off })
                .center()
                .child(if on {
                    text("✓").font(type_::LABEL).color(t.text)
                } else {
                    text("")
                });
            let b = row()
                .key(ek(""))
                .gap(8.0)
                .center_y()
                .child(mark)
                .child(text(str_of("label")).font(type_::BODY).color(t.text));
            match model_path {
                Some(p) => b.tap(move |s: &mut State| {
                    let Some(surface) = s.a2.store.get(&sid) else {
                        return;
                    };
                    let cur = dyn_bool(Some(&json!({"path": p})), &Ctx::new(&surface.data, None));
                    s.a2.write(&sid, &p, json!(!cur));
                }),
                None => b,
            }
        }
        "ChoicePicker" => {
            let single = def.get("variant").and_then(Value::as_str) == Some("mutuallyExclusive");
            let max = def
                .get("maxAllowedSelections")
                .and_then(Value::as_u64)
                .unwrap_or(if single { 1 } else { 5 }) as usize;
            let binding = def.get("selections").or_else(|| def.get("value"));
            let bound = binding
                .and_then(|v| v.get("path"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let selected = dyn_string_list(binding, &ctx);
            let mut c = col().gap(6.0);
            let label = str_of("label");
            if !label.is_empty() {
                c = c.child(text(label).font(type_::LABEL).color(t.text_secondary));
            }
            for (n, opt) in def
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let title = dyn_string(opt.get("label"), &ctx);
                let value = opt
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| title.clone());
                let on = selected.contains(&value);
                let (s1, this1, bound1, act, v1) = (
                    sid.clone(),
                    this.clone(),
                    bound.clone(),
                    action.clone(),
                    value.clone(),
                );
                let letter = ((b'A' + (n % 26) as u8) as char).to_string();
                c = c.child(
                    row()
                        .key(ek(&format!("#{n}")))
                        .gap(8.0)
                        .px(10.0)
                        .py(6.0)
                        .center_y()
                        .radius(radius::SM)
                        .fill(if on {
                            accent.alpha(0.35)
                        } else {
                            t.control_off
                        })
                        .child(text(letter).font(type_::LABEL).color(t.text_secondary))
                        .child(text(title).font(type_::BODY).color(t.text))
                        .tap(move |s: &mut State| {
                            let Some(surface) = s.a2.store.get(&s1) else {
                                return;
                            };
                            let cur = dyn_string_list(
                                bound1.as_ref().map(|p| json!({"path": p})).as_ref(),
                                &Ctx::new(&surface.data, None),
                            );
                            let next: Vec<String> = if single {
                                vec![v1.clone()]
                            } else if cur.contains(&v1) {
                                cur.into_iter().filter(|x| *x != v1).collect()
                            } else if cur.len() >= max {
                                return; // en el límite se ignora el alta
                            } else {
                                cur.into_iter().chain([v1.clone()]).collect()
                            };
                            if let Some(p) = &bound1 {
                                s.a2.write(&s1, p, json!(next));
                            }
                            if let Some((name, _)) = &act {
                                let first = next
                                    .first()
                                    .cloned()
                                    .map(Value::String)
                                    .unwrap_or(Value::Null);
                                s.a2.fire(
                                    &s1,
                                    &this1,
                                    name,
                                    json!({"selections": next, "selected": first}),
                                );
                            }
                        }),
                );
            }
            c
        }
        "Slider" => {
            let min = def
                .get("min")
                .or_else(|| def.get("minValue"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let max = def
                .get("max")
                .or_else(|| def.get("maxValue"))
                .and_then(Value::as_f64)
                .unwrap_or(100.0);
            let step = def.get("step").and_then(Value::as_f64).unwrap_or(1.0);
            let value = dyn_number(def.get("value"), &ctx);
            let label = def
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let make = |sign: f64, glyph: &str, suffix: &str| {
                let (s1, this1, p1, act) = (
                    sid.clone(),
                    this.clone(),
                    model_path.clone(),
                    action.clone(),
                );
                row()
                    .key(ek(suffix))
                    .size(28.0, 28.0)
                    .radius(radius::SM)
                    .fill(t.control_off)
                    .center()
                    .child(text(glyph.to_string()).font(type_::BODY).color(t.text))
                    .tap(move |s: &mut State| {
                        let Some(surface) = s.a2.store.get(&s1) else {
                            return;
                        };
                        let cur = p1
                            .as_ref()
                            .map(|p| {
                                dyn_number(
                                    Some(&json!({"path": p})),
                                    &Ctx::new(&surface.data, None),
                                )
                            })
                            .unwrap_or(value);
                        let next = (cur + sign * step).clamp(min, max);
                        if let Some(p) = &p1 {
                            s.a2.write(&s1, p, json!(next));
                        }
                        if let Some((name, _)) = &act {
                            s.a2.fire(&s1, &this1, name, json!({"value": next}));
                        }
                    })
            };
            // Pista arrastrable: el valor sigue al puntero y la acción sale al soltar (no en cada movimiento).
            const TRACK_W: f32 = 160.0;
            let frac = if max > min {
                ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let (s2, this2, p2, act2) = (
                sid.clone(),
                this.clone(),
                model_path.clone(),
                action.clone(),
            );
            let track = row()
                .key(ek("track"))
                .w(TRACK_W)
                .h(20.0)
                .radius(10.0)
                .fill(t.control_off)
                .child(
                    el().w((TRACK_W * frac).max(10.0))
                        .h(20.0)
                        .radius(10.0)
                        .fill(accent.alpha(0.6)),
                )
                .on_drag(move |s: &mut State, d: &mut rune::Drag, _| {
                    if d.phase == rune::Phase::Up && d.cancelled() {
                        return;
                    }
                    let next = slider_value(d.x, d.w, min, max, step);
                    if let Some(p) = &p2 {
                        s.a2.write(&s2, p, json!(next));
                    }
                    if d.phase == rune::Phase::Up {
                        if let Some((name, _)) = &act2 {
                            s.a2.fire(&s2, &this2, name, json!({"value": next}));
                        }
                    }
                });
            row()
                .gap(10.0)
                .center_y()
                .child(text(label).font(type_::BODY).color(t.text))
                .child(make(-1.0, "−", "-"))
                .child(track)
                .child(make(1.0, "+", "+"))
                .child(
                    text(js_string(&json!(value)))
                        .font(type_::BODY)
                        .color(t.text),
                )
        }
        "Tabs" => {
            let list = def
                .get("tabs")
                .or_else(|| def.get("tabItems"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let tab_key = ek("");
            let titles: Vec<String> = list
                .iter()
                .map(|x| dyn_string(x.get("title"), &ctx))
                .collect();
            let active =
                r.st.tabs
                    .get(&tab_key)
                    .cloned()
                    .or_else(|| titles.first().cloned())
                    .unwrap_or_default();
            let mut bar = row().gap(6.0);
            for (n, title) in titles.iter().enumerate() {
                let (tk, tt) = (tab_key.clone(), title.clone());
                bar = bar.child(
                    row()
                        .key(ek(&format!("#{n}")))
                        .px(12.0)
                        .py(6.0)
                        .radius(radius::SM)
                        .fill(if *title == active {
                            accent.alpha(0.35)
                        } else {
                            t.control_off
                        })
                        .child(text(title.clone()).font(type_::LABEL).color(t.text))
                        .tap(move |s: &mut State| {
                            s.a2.tabs.insert(tk.clone(), tt.clone());
                        }),
                );
            }
            let body = titles
                .iter()
                .position(|x| *x == active)
                .and_then(|n| list[n].get("child")?.as_str().map(str::to_string));
            let c = col().gap(8.0).child(bar);
            match body {
                Some(b) => c.child(node(r, &b, scope, anc, &format!("{key}>{b}"))),
                None => c,
            }
        }
        "Modal" => {
            let open_key = ek("");
            let trigger = def
                .get("trigger")
                .or_else(|| def.get("entryPointChild"))
                .and_then(Value::as_str);
            let content = def
                .get("content")
                .or_else(|| def.get("contentChild"))
                .and_then(Value::as_str);
            let (k1, k2) = (open_key.clone(), open_key.clone());
            let mut c = col().gap(8.0);
            if let Some(tr) = trigger {
                c = c.child(
                    col()
                        .key(ek("#t"))
                        .child(node(r, tr, scope, anc, &format!("{key}>{tr}")))
                        .tap(move |s: &mut State| {
                            if !s.a2.open.remove(&k1) {
                                s.a2.open.insert(k1.clone());
                            }
                        }),
                );
            }
            if let (true, Some(co)) = (r.st.open.contains(&open_key), content) {
                c = c.child(
                    col()
                        .gap(8.0)
                        .padding(12.0)
                        .radius(radius::SM)
                        .fill(t.control_off)
                        .border(1.0, t.separator)
                        .child(node(r, co, scope, anc, &format!("{key}>{co}")))
                        .child(
                            text("Cerrar")
                                .key(ek("#x"))
                                .font(type_::LABEL)
                                .color(accent)
                                .tap(move |s: &mut State| {
                                    s.a2.open.remove(&k2);
                                }),
                        ),
                );
            }
            c
        }
        other => col()
            .px(10.0)
            .py(8.0)
            .radius(radius::SM)
            .border(1.0, t.separator)
            .border_dash(4.0, 3.0)
            .child(
                text(format!("Unknown component: {other}"))
                    .font(type_::LABEL)
                    .color(t.text_secondary),
            ),
    }
}

/// Valor de una pista de deslizador a partir de la posición del puntero: se ajusta al paso y se limita al rango.
fn slider_value(x: f32, w: f32, min: f64, max: f64, step: f64) -> f64 {
    if w <= 0.0 || max <= min {
        return min;
    }
    let raw = min + (x / w).clamp(0.0, 1.0) as f64 * (max - min);
    let snapped = if step > 0.0 {
        min + ((raw - min) / step).round() * step
    } else {
        raw
    };
    // Redondeo para no arrastrar ruido de coma flotante (0.30000000000000004).
    (snapped.clamp(min, max) * 1e6).round() / 1e6
}

#[cfg(test)]
mod slider_tests {
    use super::slider_value;

    #[test]
    fn follows_the_pointer_snaps_to_the_step_and_stays_in_range() {
        assert_eq!(slider_value(0.0, 160.0, 0.0, 100.0, 1.0), 0.0);
        assert_eq!(slider_value(80.0, 160.0, 0.0, 100.0, 1.0), 50.0);
        assert_eq!(slider_value(160.0, 160.0, 0.0, 100.0, 1.0), 100.0);
        assert_eq!(
            slider_value(-30.0, 160.0, 0.0, 100.0, 1.0),
            0.0,
            "fuera por la izquierda"
        );
        assert_eq!(
            slider_value(400.0, 160.0, 0.0, 100.0, 1.0),
            100.0,
            "fuera por la derecha"
        );
        assert_eq!(
            slider_value(83.0, 160.0, 0.0, 100.0, 10.0),
            50.0,
            "ajusta al paso de 10"
        );
        assert_eq!(
            slider_value(48.0, 160.0, 0.0, 1.0, 0.1),
            0.3,
            "sin ruido de coma flotante"
        );
        assert_eq!(
            slider_value(10.0, 0.0, 5.0, 9.0, 1.0),
            5.0,
            "pista sin ancho"
        );
        assert_eq!(slider_value(10.0, 160.0, 7.0, 7.0, 1.0), 7.0, "rango vacío");
    }
}

/// Formato que espera un `DateTimeInput`, el mismo que enseña el marcador de posición.
fn datetime_format(date: bool, time: bool) -> &'static str {
    match (date, time) {
        (true, true) => "yyyy-MM-ddTHH:mm",
        (true, false) => "yyyy-MM-dd",
        _ => "HH:mm",
    }
}

/// ¿`text` es una fecha, una hora o ambas válidas? Acepta también segundos en la hora.
fn valid_datetime(text: &str, date: bool, time: bool) -> bool {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    match (date, time) {
        (true, true) => {
            NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").is_ok()
                || NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").is_ok()
        }
        (true, false) => NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok(),
        _ => {
            NaiveTime::parse_from_str(text, "%H:%M").is_ok()
                || NaiveTime::parse_from_str(text, "%H:%M:%S").is_ok()
        }
    }
}

/// El instante actual (hora local) en el formato del campo.
fn now_text(date: bool, time: bool) -> String {
    let now = chrono::Local::now();
    match (date, time) {
        (true, true) => now.format("%Y-%m-%dT%H:%M").to_string(),
        (true, false) => now.format("%Y-%m-%d").to_string(),
        _ => now.format("%H:%M").to_string(),
    }
}

#[cfg(test)]
mod datetime_tests {
    use super::{datetime_format, now_text, valid_datetime};

    #[test]
    fn validates_each_mode_and_rejects_impossible_values() {
        assert!(valid_datetime("2026-11-07", true, false));
        assert!(
            !valid_datetime("2026-02-30", true, false),
            "el 30 de febrero no existe"
        );
        assert!(!valid_datetime("07/11/2026", true, false));
        assert!(valid_datetime("2026-11-07T18:30", true, true));
        assert!(valid_datetime("2026-11-07T18:30:15", true, true));
        assert!(!valid_datetime("2026-11-07", true, true), "falta la hora");
        assert!(valid_datetime("23:59", false, true));
        assert!(!valid_datetime("24:00", false, true));
        assert!(!valid_datetime("", true, false));
    }

    #[test]
    fn the_now_button_produces_text_the_validator_accepts() {
        for (d, t) in [(true, true), (true, false), (false, true)] {
            let n = now_text(d, t);
            assert!(
                valid_datetime(&n, d, t),
                "{n} en modo {}",
                datetime_format(d, t)
            );
        }
    }
}
