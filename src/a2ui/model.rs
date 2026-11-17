//! Almacén de superficies, modelo de datos, bindings, funciones y validaciones de A2UI.

use std::collections::HashMap;

use regex::Regex;
use serde_json::{json, Map, Value};

// ───────────────────────────── almacén ─────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct Surface {
    pub id: String,
    pub theme: Value,
    pub root: Option<String>,
    /// Orden de primera aparición.
    pub order: Vec<String>,
    pub components: HashMap<String, Value>,
    pub data: Value,
}

#[derive(Debug, Default)]
pub struct Store {
    pub surfaces: Vec<Surface>,
}

impl Store {
    pub fn get(&self, id: &str) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|s| s.id == id)
    }

    /// Aplica un frame `a2ui:*` del servidor (`{type, data}`). Devuelve si algo cambió.
    pub fn apply(&mut self, kind: &str, data: &Value) -> bool {
        self.try_apply(kind, data).is_some()
    }

    fn try_apply(&mut self, kind: &str, data: &Value) -> Option<()> {
        let id = data.get("surfaceId")?.as_str()?.to_string();
        match kind {
            "a2ui:createSurface" => {
                let surface = Surface {
                    id: id.clone(),
                    theme: data.get("theme").cloned().unwrap_or_else(|| json!({})),
                    data: json!({}),
                    ..Default::default()
                };
                // Reemplaza una superficie con el mismo id: se reinician componentes y datos.
                match self.surfaces.iter().position(|s| s.id == id) {
                    Some(i) => self.surfaces[i] = surface,
                    None => self.surfaces.push(surface),
                }
            }
            "a2ui:updateComponents" => {
                let list = data.get("components")?.as_array()?;
                if list.is_empty() {
                    return None;
                }
                let s = self.get_mut(&id)?;
                for def in list {
                    if let Some((cid, def)) = normalize(def) {
                        if !s.components.contains_key(&cid) {
                            s.order.push(cid.clone());
                        }
                        // El id existente se reemplaza entero: no se mezclan propiedades.
                        s.components.insert(cid, def);
                    }
                }
                s.pick_root();
            }
            "a2ui:updateDataModel" => {
                let s = self.get_mut(&id)?;
                let path = data.get("path").and_then(Value::as_str).unwrap_or("");
                set_path(&mut s.data, path, data.get("value").cloned());
            }
            "a2ui:deleteSurface" => {
                self.get(&id)?;
                self.surfaces.retain(|s| s.id != id);
            }
            _ => return None,
        }
        Some(())
    }
}

/// `{"component":{"Card":{"child":"x"}}}` se aplana a `{"component":"Card","child":"x"}`.
fn normalize(def: &Value) -> Option<(String, Value)> {
    let obj = def.as_object()?;
    let id = obj.get("id")?.as_str()?.to_string();
    let mut out = obj.clone();
    if let Some(Value::Object(inner)) = obj.get("component") {
        if let Some((kind, props)) = inner.iter().next() {
            if let Value::Object(p) = props {
                for (k, v) in p {
                    out.insert(k.clone(), v.clone());
                }
            }
            out.insert("component".into(), Value::String(kind.clone()));
        }
    }
    Some((id, Value::Object(out)))
}

impl Surface {
    /// Conserva la raíz si sigue existiendo; si no, la primera componente que nadie referencia
    /// como hija (o la primera de la lista).
    fn pick_root(&mut self) {
        if self
            .root
            .as_ref()
            .is_some_and(|r| self.components.contains_key(r))
        {
            return;
        }
        let referenced: std::collections::HashSet<String> =
            self.components.values().flat_map(child_refs).collect();
        self.root = self
            .order
            .iter()
            .find(|id| !referenced.contains(*id))
            .or_else(|| self.order.first())
            .cloned();
    }

    pub fn component(&self, id: &str) -> Option<&Value> {
        self.components.get(id)
    }
}

/// Ids que una componente referencia como hijos.
pub fn child_refs(def: &Value) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(c) = def.get("children") {
        collect_children(c, &mut v);
    }
    for k in [
        "child",
        "trigger",
        "content",
        "entryPointChild",
        "contentChild",
    ] {
        if let Some(s) = def.get(k).and_then(Value::as_str) {
            v.push(s.to_string());
        }
    }
    for k in ["tabItems", "tabs"] {
        if let Some(Value::Array(items)) = def.get(k) {
            v.extend(
                items
                    .iter()
                    .filter_map(|t| t.get("child")?.as_str().map(str::to_string)),
            );
        }
    }
    v
}

fn collect_children(c: &Value, out: &mut Vec<String>) {
    match c {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => out.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string))),
        Value::Object(o) => {
            for k in ["explicitList", "array"] {
                if let Some(Value::Array(a)) = o.get(k) {
                    out.extend(a.iter().filter_map(|x| x.as_str().map(str::to_string)));
                }
            }
            if let Some(s) = o.get("componentId").and_then(Value::as_str) {
                out.push(s.to_string());
            }
            if let Some(s) = o
                .get("template")
                .and_then(|t| t.get("componentId"))
                .and_then(Value::as_str)
            {
                out.push(s.to_string());
            }
        }
        _ => {}
    }
}

/// Una hija a pintar: su id, el alcance de plantilla que hereda y su clave estable.
#[derive(Clone, Debug, PartialEq)]
pub struct ChildRef {
    pub id: String,
    pub scope: Option<Value>,
    pub key: String,
}

/// Resuelve `children` en las cinco formas que acepta la UI. Con plantilla, repite
/// `componentId` por cada elemento del arreglo; el alcance NO se encadena.
pub fn children_of(children: &Value, ctx: &Ctx) -> Vec<ChildRef> {
    let plain = |ids: Vec<String>, scope: Option<&Value>| {
        ids.into_iter()
            .map(|id| ChildRef {
                key: id.clone(),
                id,
                scope: scope.cloned(),
            })
            .collect::<Vec<_>>()
    };
    let scope = ctx.scope;
    match children {
        Value::String(s) => plain(vec![s.clone()], scope),
        Value::Array(a) => plain(
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect(),
            scope,
        ),
        Value::Object(o) => {
            for k in ["explicitList", "array"] {
                if let Some(Value::Array(a)) = o.get(k) {
                    return plain(
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect(),
                        scope,
                    );
                }
            }
            let (path, cid) = if let (Some(p), Some(c)) = (
                o.get("path").and_then(Value::as_str),
                o.get("componentId").and_then(Value::as_str),
            ) {
                (p, c)
            } else if let Some(t) = o.get("template") {
                match (
                    t.get("dataBinding").and_then(Value::as_str),
                    t.get("componentId").and_then(Value::as_str),
                ) {
                    (Some(p), Some(c)) => (p, c),
                    _ => return vec![],
                }
            } else {
                return vec![];
            };
            let Some(Value::Array(items)) = ctx.resolve_path(path) else {
                return vec![];
            };
            items
                .iter()
                .enumerate()
                .map(|(i, item)| ChildRef {
                    id: cid.to_string(),
                    key: format!("{cid}_{i}"),
                    scope: Some(if item.is_object() {
                        item.clone()
                    } else {
                        json!({"value": item, "index": i})
                    }),
                })
                .collect()
        }
        _ => vec![],
    }
}

// ───────────────────────── modelo de datos ─────────────────────────

/// Lee `path` ("/a/b" o "a/b"); recorre objetos y arreglos (por índice).
pub fn get_at_path<'a>(obj: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() || path == "/" {
        return Some(obj);
    }
    let mut cur = obj;
    for seg in path.strip_prefix('/').unwrap_or(path).split('/') {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Escribe `value` en `path`, creando `{}` donde falte. Sin `value` borra la clave; sin ruta
/// (o "/") reemplaza todo el modelo.
pub fn set_path(root: &mut Value, path: &str, value: Option<Value>) {
    if path.is_empty() || path == "/" {
        *root = value.unwrap_or_else(|| json!({}));
        return;
    }
    let segs: Vec<&str> = path.strip_prefix('/').unwrap_or(path).split('/').collect();
    let (last, mid) = segs.split_last().expect("al menos un segmento");
    let mut cur = root;
    for seg in mid {
        if let Ok(i) = seg.parse::<usize>() {
            if cur.as_array().is_some_and(|a| i < a.len()) {
                cur = &mut cur[i];
                continue;
            }
        }
        if !cur.is_object() {
            *cur = json!({});
        }
        cur = cur
            .as_object_mut()
            .unwrap()
            .entry(seg.to_string())
            .or_insert_with(|| json!({}));
    }
    let index = last.parse::<usize>().ok();
    if !cur.is_object() && !(cur.is_array() && index.is_some()) {
        *cur = json!({});
    }
    match value {
        None => {
            if let Some(m) = cur.as_object_mut() {
                m.remove(*last);
            }
        }
        Some(v) => {
            if let Some(m) = cur.as_object_mut() {
                m.insert(last.to_string(), v);
            } else if let (Some(i), Some(slot)) = (index, index.and_then(|i| cur.get_mut(i))) {
                let _ = i;
                *slot = v;
            }
        }
    }
}

/// Modelo de datos + alcance de plantilla con el que se resuelven los bindings.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub model: &'a Value,
    pub scope: Option<&'a Value>,
}

impl<'a> Ctx<'a> {
    pub fn new(model: &'a Value, scope: Option<&'a Value>) -> Self {
        Ctx { model, scope }
    }

    /// Absoluta ("/x") desde la raíz; relativa: primero el alcance de plantilla y, si no está,
    /// la raíz del modelo.
    pub fn resolve_path(&self, path: &str) -> Option<Value> {
        if path.is_empty() {
            return None;
        }
        if path.starts_with('/') {
            return get_at_path(self.model, path).cloned();
        }
        self.scope
            .and_then(|s| get_at_path(s, path))
            .or_else(|| get_at_path(self.model, path))
            .cloned()
    }
}

fn is_path(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|o| o.contains_key("path") && !o.contains_key("call"))
}

fn is_call(v: &Value) -> bool {
    v.as_object().is_some_and(|o| o.contains_key("call"))
}

/// `String(x)` de JavaScript.
pub fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            Some(f) => f.to_string(),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|x| {
                if x.is_null() {
                    String::new()
                } else {
                    js_string(x)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `Number(x)` de JavaScript (NaN si no se puede).
fn js_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => *b as u8 as f64,
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

fn truthy_true(v: &Value) -> bool {
    v == &Value::Bool(true) || v.as_str() == Some("true")
}

/// DynamicString: literal, `{path}` o `{call,args}`; lo ausente es "".
pub fn dyn_string(v: Option<&Value>, ctx: &Ctx) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(x) if is_path(x) => ctx
            .resolve_path(x["path"].as_str().unwrap_or(""))
            .filter(|r| !r.is_null())
            .map(|r| js_string(&r))
            .unwrap_or_default(),
        Some(x) if is_call(x) => execute_call(x, ctx)
            .filter(|r| !r.is_null())
            .map(|r| js_string(&r))
            .unwrap_or_default(),
        Some(x) => js_string(x),
    }
}

pub fn dyn_number(v: Option<&Value>, ctx: &Ctx) -> f64 {
    let n = |r: Option<Value>| r.and_then(|r| r.as_f64()).unwrap_or(0.0);
    match v {
        None | Some(Value::Null) => 0.0,
        Some(Value::Number(x)) => x.as_f64().unwrap_or(0.0),
        Some(x) if is_path(x) => n(ctx.resolve_path(x["path"].as_str().unwrap_or(""))),
        Some(x) if is_call(x) => n(execute_call(x, ctx)),
        Some(x) => {
            let f = js_number(x);
            if f.is_nan() {
                0.0
            } else {
                f
            }
        }
    }
}

pub fn dyn_bool(v: Option<&Value>, ctx: &Ctx) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(x) if is_path(x) => ctx
            .resolve_path(x["path"].as_str().unwrap_or(""))
            .is_some_and(|r| truthy_true(&r)),
        Some(x) if is_call(x) => execute_call(x, ctx) == Some(Value::Bool(true)),
        Some(x) => {
            !matches!(x, Value::String(s) if s.is_empty())
                && !matches!(x.as_f64(), Some(f) if f == 0.0)
        }
    }
}

pub fn dyn_string_list(v: Option<&Value>, ctx: &Ctx) -> Vec<String> {
    let list = |r: Option<Value>, scalar: bool| match r {
        Some(Value::Array(a)) => a.iter().map(js_string).collect(),
        Some(x) if scalar && !x.is_null() => vec![js_string(&x)],
        _ => vec![],
    };
    match v {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.iter().map(js_string).collect(),
        Some(x) if is_path(x) => list(ctx.resolve_path(x["path"].as_str().unwrap_or("")), true),
        Some(x) if is_call(x) => list(execute_call(x, ctx), false),
        Some(_) => vec![],
    }
}

// ───────────────────────────── funciones ─────────────────────────────

fn resolve_arg(v: &Value, ctx: &Ctx) -> Value {
    if is_call(v) {
        execute_call(v, ctx).unwrap_or(Value::Null)
    } else if is_path(v) {
        ctx.resolve_path(v["path"].as_str().unwrap_or(""))
            .unwrap_or(Value::Null)
    } else {
        v.clone()
    }
}

/// Ejecuta `{call, args}`. Primero resuelve cada argumento. Función desconocida: `None`.
pub fn execute_call(call: &Value, ctx: &Ctx) -> Option<Value> {
    let name = call.get("call")?.as_str()?;
    let mut args = Map::new();
    if let Some(Value::Object(raw)) = call.get("args") {
        for (k, v) in raw {
            args.insert(k.clone(), resolve_arg(v, ctx));
        }
    }
    run_function(name, &args, ctx)
}

fn run_function(name: &str, a: &Map<String, Value>, ctx: &Ctx) -> Option<Value> {
    let value = a.get("value").cloned().unwrap_or(Value::Null);
    let get_f = |k: &str, d: f64| {
        a.get(k)
            .filter(|v| !v.is_null())
            .map(js_number)
            .unwrap_or(d)
    };
    Some(match name {
        "formatString" => Value::String(format_string(&js_string(&value), ctx)),
        "formatDate" => {
            let fmt = a
                .get("format")
                .map(js_string)
                .unwrap_or_else(|| "yyyy-MM-dd".into());
            Value::String(format_date(&js_string(&value), &fmt))
        }
        "formatNumber" => {
            let grouping = a.get("grouping") != Some(&Value::Bool(false));
            Value::String(format_number(
                js_number(&value),
                grouping,
                get_f("precision", 0.0).max(0.0) as usize,
            ))
        }
        "formatCurrency" => {
            let cur = a
                .get("currency")
                .map(js_string)
                .unwrap_or_else(|| "USD".into());
            Value::String(format_currency(js_number(&value), &cur))
        }
        "required" => Value::Bool(!value.is_null() && value.as_str() != Some("")),
        "regex" => {
            let pattern = a.get("pattern").map(js_string).unwrap_or_default();
            Value::Bool(
                pattern.is_empty()
                    || Regex::new(&pattern).is_ok_and(|r| r.is_match(&js_string(&value))),
            )
        }
        "email" => Value::Bool(
            Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$")
                .unwrap()
                .is_match(&js_string(&value)),
        ),
        "length" => {
            let n = js_string(&value).encode_utf16().count() as f64;
            Value::Bool(n >= get_f("min", 0.0) && n <= get_f("max", f64::INFINITY))
        }
        "numeric" => {
            let n = js_number(&Value::String(js_string(&value)));
            Value::Bool(
                !n.is_nan()
                    && a.get("min").is_none_or(|m| n >= js_number(m))
                    && a.get("max").is_none_or(|m| n <= js_number(m)),
            )
        }
        "and" => Value::Bool(a.get("values")?.as_array()?.iter().all(truthy_true)),
        "or" => Value::Bool(a.get("values")?.as_array()?.iter().any(truthy_true)),
        "not" => Value::Bool(!truthy_true(&value)),
        _ => return None,
    })
}

/// Reemplaza cada `${expr}`: una ruta (absoluta o relativa) o `fn(clave:valor, …)`. Lo que no
/// se puede resolver queda tal cual.
pub fn format_string(template: &str, ctx: &Ctx) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let expr = &after[..j];
        match interpolate(expr, ctx) {
            Some(s) => out.push_str(&s),
            None => {
                out.push_str("${");
                out.push_str(expr);
                out.push('}');
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

fn interpolate(expr: &str, ctx: &Ctx) -> Option<String> {
    let call = Regex::new(r"(?s)^(\w+)\((.+)\)$").unwrap();
    if let Some(c) = call.captures(expr) {
        let mut args = Map::new();
        for (n, part) in c[2].split(',').enumerate() {
            let (k, v) = part
                .split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim()))
                .unwrap_or((n.to_string(), part.trim()));
            let v = v.trim_matches(|ch| ch == '\'' || ch == '"');
            let val = match v {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => v
                    .parse::<f64>()
                    .ok()
                    .filter(|_| !v.is_empty())
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .unwrap_or_else(|| Value::String(v.to_string())),
            };
            args.insert(k, val);
        }
        return run_function(&c[1], &args, ctx)
            .filter(|r| !r.is_null())
            .map(|r| js_string(&r));
    }
    ctx.resolve_path(expr).map(|v| js_string(&v))
}

fn group_thousands(int: &str) -> String {
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Equivale a `toLocaleString("en-US")` con decimales fijos.
pub fn format_number(n: f64, grouping: bool, precision: usize) -> String {
    if !n.is_finite() {
        return "0".into();
    }
    let s = format!("{:.*}", precision, n.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s.clone(), None),
    };
    let int = if grouping { group_thousands(&int) } else { int };
    let sign = if n < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "-"
    } else {
        ""
    };
    match frac {
        Some(f) => format!("{sign}{int}.{f}"),
        None => format!("{sign}{int}"),
    }
}

/// Aproximación de `Intl.NumberFormat("en-US", {style:"currency"})` para las monedas comunes.
pub fn format_currency(n: f64, currency: &str) -> String {
    let code = currency.to_ascii_uppercase();
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return format!("${:.2}", n);
    }
    let (symbol, decimals) = match code.as_str() {
        "USD" => ("$", 2),
        "EUR" => ("€", 2),
        "GBP" => ("£", 2),
        "JPY" => ("¥", 0),
        "MXN" => ("MX$", 2),
        "COP" => ("COP\u{a0}", 2),
        _ => return format!("{code}\u{a0}{}", format_number(n, true, 2)),
    };
    let body = format_number(n.abs(), true, decimals);
    format!("{}{symbol}{body}", if n < 0.0 { "-" } else { "" })
}

/// Formatea una fecha con los tokens yyyy yy MMMM MMM MM M dd d HH H hh h mm m ss s a EEEE E
/// (en-US, hora local). Si no se puede interpretar, devuelve el texto original.
pub fn format_date(input: &str, format: &str) -> String {
    use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Timelike};
    let dt: Option<NaiveDateTime> = DateTime::parse_from_rfc3339(input)
        .ok()
        .map(|d| d.with_timezone(&Local).naive_local())
        .or_else(|| NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%.f").ok())
        .or_else(|| NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M").ok())
        .or_else(|| NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S").ok())
        .or_else(|| {
            NaiveDate::parse_from_str(input, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        });
    let Some(dt) = dt else {
        return input.to_string();
    };
    const TOKENS: [&str; 19] = [
        "yyyy", "EEEE", "MMMM", "MMM", "yy", "MM", "dd", "HH", "hh", "mm", "ss", "E", "M", "d",
        "H", "h", "m", "s", "a",
    ];
    let h12 = match dt.hour() % 12 {
        0 => 12,
        h => h,
    };
    let mut out = String::new();
    let mut rest = format;
    'outer: while !rest.is_empty() {
        for t in TOKENS {
            if let Some(tail) = rest.strip_prefix(t) {
                out.push_str(&match t {
                    "yyyy" => dt.format("%Y").to_string(),
                    "yy" => dt.format("%y").to_string(),
                    "EEEE" => dt.format("%A").to_string(),
                    "E" => dt.format("%a").to_string(),
                    "MMMM" => dt.format("%B").to_string(),
                    "MMM" => dt.format("%b").to_string(),
                    "MM" => dt.format("%m").to_string(),
                    "M" => dt.format("%-m").to_string(),
                    "dd" => dt.format("%d").to_string(),
                    "d" => dt.format("%-d").to_string(),
                    "HH" => dt.format("%H").to_string(),
                    "H" => dt.hour().to_string(),
                    "hh" => format!("{h12:02}"),
                    "h" => h12.to_string(),
                    "mm" => dt.format("%M").to_string(),
                    "m" => dt.format("%-M").to_string(),
                    "ss" => dt.format("%S").to_string(),
                    "s" => dt.format("%-S").to_string(),
                    _ => (if dt.hour() < 12 { "AM" } else { "PM" }).to_string(),
                });
                rest = tail;
                continue 'outer;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

// ───────────────────────── validaciones y acciones ─────────────────────────

/// Mensajes de los `checks` que fallan, en orden.
pub fn run_checks(checks: Option<&Value>, ctx: &Ctx) -> Vec<String> {
    let Some(Value::Array(list)) = checks else {
        return vec![];
    };
    list.iter()
        .filter_map(|c| {
            let ok = match c.get("condition").filter(|x| x.is_object()) {
                Some(cond) => execute_call(cond, ctx) == Some(Value::Bool(true)),
                None => {
                    let call = json!({"call": c.get("call").cloned().unwrap_or(Value::Null), "args": c.get("args").cloned().unwrap_or_else(|| json!({}))});
                    execute_call(&call, ctx).is_some_and(|r| truthy_true(&r))
                }
            };
            (!ok).then(|| {
                c.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
                    format!("Validation failed: {}", c.get("call").and_then(Value::as_str).unwrap_or(""))
                })
            })
        })
        .collect()
}

/// `action` de un botón: `{"event":{name,context}}` o el formato antiguo `{name,context}`.
pub fn action_event(action: &Value) -> Option<(String, Value)> {
    let ev = action.get("event").unwrap_or(action);
    let name = ev.get("name")?.as_str()?.to_string();
    Some((
        name,
        ev.get("context").cloned().unwrap_or_else(|| json!({})),
    ))
}

/// Resuelve el contexto de una acción al momento del clic: `{path}` y `{call}` se evalúan,
/// el resto va tal cual.
pub fn resolve_context(context: &Value, ctx: &Ctx) -> Value {
    let Some(map) = context.as_object() else {
        return json!({});
    };
    let mut out = Map::new();
    for (k, v) in map {
        let r = if is_path(v) {
            ctx.resolve_path(v["path"].as_str().unwrap_or(""))
                .unwrap_or(Value::Null)
        } else if is_call(v) {
            execute_call(v, ctx).unwrap_or(Value::Null)
        } else {
            v.clone()
        };
        out.insert(k.clone(), r);
    }
    Value::Object(out)
}

/// Frame `a2ui:action` listo para enviar.
pub fn action_frame(name: &str, surface: &str, source: &str, context: Value) -> Value {
    json!({
        "type": "a2ui:action",
        "data": {
            "name": name,
            "surfaceId": surface,
            "sourceComponentId": source,
            "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "context": context,
        }
    })
}

#[cfg(test)]
mod tests;
