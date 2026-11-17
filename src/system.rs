//! Datos del sistema para el Dashboard, la ética, el perfil y las actualizaciones. Las llamadas REST corren en
//! hilos y dejan su resultado en `data`; `sync` lo copia a lo que pinta la vista (igual que Ajustes).

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::settings::api::{request, ApiError, Conn};

/// Cada cuánto se vuelven a leer las métricas mientras se mira el Dashboard (la web lo hace cada 5 s).
pub const REFRESH: Duration = Duration::from_secs(5);

fn f(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or(0.0)
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

/// Métricas del proceso (`/api/system-stats`). La memoria llega en MB.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub runtime: String,
    pub process_metrics_available: bool,
    pub active_runs: u64,
    pub cpu: f64,
    pub rss: f64,
    pub heap_used: f64,
    pub heap_total: f64,
    pub external: f64,
    pub uptime: String,
    pub connections: u64,
    pub cores: u64,
    pub recent_messages: u64,
}

pub fn parse_stats(v: &Value) -> Stats {
    Stats {
        runtime: s(v, "runtime"),
        process_metrics_available: v["processMetricsAvailable"].as_bool().unwrap_or(true),
        active_runs: v["activeRuns"].as_u64().unwrap_or(0),
        cpu: f(v, "cpu"),
        rss: f(&v["memory"], "rss"),
        heap_used: f(&v["memory"], "heapUsed"),
        heap_total: f(&v["memory"], "heapTotal"),
        external: f(&v["memory"], "external"),
        uptime: s(v, "uptime"),
        connections: v["connections"].as_u64().unwrap_or(0),
        cores: v["cores"].as_u64().unwrap_or(0),
        recent_messages: v["recentMessages"].as_u64().unwrap_or(0),
    }
}

/// Mensajes por hora (`/api/activity-stats`): `[{time: "2026-11-08 03:00", count}]`.
pub fn parse_activity(v: &Value) -> Vec<(String, u64)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|x| (s(x, "time"), x["count"].as_u64().unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default()
}

/// La hora de una muestra de actividad: «2026-11-08 03:00» → «03:00».
pub fn hour_label(time: &str) -> String {
    time.rsplit(' ').next().unwrap_or(time).to_string()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Version {
    pub current: String,
    pub latest: String,
    /// `up-to-date`, `update-available`…
    pub status: String,
    pub installation: String,
}

pub fn parse_version(v: &Value) -> Version {
    Version {
        current: s(v, "current"),
        latest: s(v, "latest"),
        status: s(v, "status"),
        installation: s(v, "installationType"),
    }
}

impl Version {
    pub fn up_to_date(&self) -> bool {
        self.status == "up-to-date" || (!self.current.is_empty() && self.current == self.latest)
    }
}

/// Consumo de tokens y costo (`/api/usage-stats`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub cost_incomplete: bool,
    pub tokens: u64,
    pub input: u64,
    pub output: u64,
    pub cost: f64,
    /// (proveedor, tokens, costo), de más a menos tokens.
    pub providers: Vec<(String, u64, f64)>,
    pub toon_saved_tokens: u64,
}

pub fn parse_usage(v: &Value) -> Usage {
    let mut providers: Vec<(String, u64, f64)> = v["byProvider"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, p)| {
                    (
                        k.clone(),
                        p["tokens"].as_u64().unwrap_or(0),
                        f(p, "costUsd"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    providers.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Usage {
        cost_incomplete: v["costIncomplete"].as_bool().unwrap_or(false),
        tokens: v["totalTokens"].as_u64().unwrap_or(0),
        input: v["totalInputTokens"].as_u64().unwrap_or(0),
        output: v["totalOutputTokens"].as_u64().unwrap_or(0),
        cost: f(v, "totalCostUsd"),
        providers,
        toon_saved_tokens: v["toonSavedTokens"].as_u64().unwrap_or(0),
    }
}

/// Un agente que el curador propone apagar (`/api/agents/proposals`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Proposal {
    pub agent: String,
    pub reason: String,
}

pub fn parse_proposals(v: &Value) -> Vec<Proposal> {
    v["proposals"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| Proposal {
                    agent: s(p, "agentName"),
                    reason: s(p, "reason"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Los motivos que sabe traducir la web; cualquier otro se muestra tal cual.
pub fn describe_reason(reason: &str) -> String {
    match reason {
        "" => "revisión pendiente".into(),
        "harmful_count exceeded helpful_count" => {
            "más resultados dañinos que útiles en sus verificaciones".into()
        }
        other => other.to_string(),
    }
}

/// Un protocolo ético (`/api/ethics`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ethic {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub active: bool,
}

pub fn parse_ethics(v: &Value) -> Vec<Ethic> {
    v["ethics"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(Ethic {
                        id: e["id"].as_str()?.to_string(),
                        name: s(e, "name"),
                        description: s(e, "description"),
                        content: s(e, "content"),
                        active: e["active"].as_bool().unwrap_or(false)
                            || e["active"].as_u64() == Some(1),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// El usuario (`/api/users`, el primero).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub language: String,
    pub timezone: String,
    pub occupation: String,
    pub notes: String,
    pub cron_channel: String,
}

pub fn parse_user(v: &Value) -> Option<User> {
    let u = v["users"].as_array()?.first()?;
    Some(User {
        id: u["id"].as_str()?.to_string(),
        name: s(u, "name"),
        email: s(u, "email"),
        language: s(u, "language"),
        timezone: s(u, "timezone"),
        occupation: s(u, "occupation"),
        notes: s(u, "notes"),
        cron_channel: s(u, "preferred_cron_channel"),
    })
}

impl User {
    /// Las iniciales del avatar: «John Paez» → «JP»; un solo nombre → su inicial; vacío → «?».
    pub fn initials(&self) -> String {
        let letters: String = self
            .name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect();
        if letters.is_empty() {
            "?".into()
        } else {
            letters
        }
    }
}

/// Una nota del cuaderno de apuntes de los agentes (`/api/notes`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
    pub id: String,
    pub thread_id: String,
    pub key: String,
    pub value: String,
    pub source: String,
    /// Segundos desde 1970.
    pub created_at: i64,
}

pub fn parse_notes(v: &Value) -> Vec<Note> {
    v["notes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|n| {
                    Some(Note {
                        id: n["id"].as_str()?.to_string(),
                        thread_id: s(n, "thread_id"),
                        key: s(n, "key"),
                        value: s(n, "value"),
                        source: s(n, "source"),
                        created_at: n["created_at"].as_i64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Note {
    /// El final del id de la conversación (`…/conv-e5992b532870` → `2b532870`, ocho caracteres) que la web usa de etiqueta.
    pub fn thread_tag(&self) -> String {
        let t = self
            .thread_id
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_start_matches("conv-");
        let n = t.chars().count();
        t.chars().skip(n.saturating_sub(8)).collect()
    }
}

/// Una tarea programada (`/api/cron`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Task {
    pub id: String,
    pub name: String,
    /// `active`, `paused`, `completed`…
    pub status: String,
    /// `recurring`, `one_shot`, `interval`…
    pub kind: String,
    pub cron: String,
    pub channel: String,
    pub timezone: String,
    pub run_count: u64,
    pub error_count: u64,
    pub last_run: String,
    pub next_run: String,
    pub last_error: String,
    pub task: String,
    /// Las tareas internas de Hive (limpieza, etc.) no se enseñan.
    pub internal: bool,
}

pub fn parse_tasks(v: &Value) -> Vec<Task> {
    v["tasks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let name = s(t, "name");
                    let payload: Value = t["payload"]
                        .as_str()
                        .and_then(|p| serde_json::from_str(p).ok())
                        .unwrap_or(Value::Null);
                    Some(Task {
                        id: t["id"].as_str()?.to_string(),
                        internal: name.starts_with("_hive")
                            || payload["_internal"].as_bool().unwrap_or(false),
                        name,
                        status: s(t, "status"),
                        kind: s(t, "task_type"),
                        cron: s(t, "cron_expression"),
                        channel: s(t, "channel"),
                        timezone: s(t, "timezone"),
                        run_count: t["run_count"].as_u64().unwrap_or(0),
                        error_count: t["error_count"].as_u64().unwrap_or(0),
                        last_run: s(t, "last_run_at"),
                        next_run: s(t, "next_run_at"),
                        last_error: s(t, "last_error"),
                        task: s(t, "task"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Una ejecución de una tarea (`/api/cron/:id/history`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub status: String,
    pub started: String,
    pub duration_ms: u64,
    pub response: String,
    pub error: String,
}

pub fn parse_history(v: &Value) -> Vec<Run> {
    v["history"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| Run {
                    status: s(r, "status"),
                    started: s(r, "started_at"),
                    duration_ms: r["duration_ms"].as_u64().unwrap_or(0),
                    response: s(r, "agent_response"),
                    error: s(r, "error_message"),
                })
                .collect()
        })
        .unwrap_or_default()
}

const MONTHS: [&str; 12] = [
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
];

/// «2026-11-09T13:00:00.000Z» → «9 nov, 08:00 a.m.» (en la hora de este equipo). Vacío o roto → «—».
pub fn when_label(iso: &str) -> String {
    use chrono::{Datelike, Local, Timelike};
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return "—".into();
    };
    let t = t.with_timezone(&Local);
    let (h, pm) = (
        if t.hour() % 12 == 0 {
            12
        } else {
            t.hour() % 12
        },
        t.hour() >= 12,
    );
    format!(
        "{} {}, {:02}:{:02} {}",
        t.day(),
        MONTHS[t.month0() as usize],
        h,
        t.minute(),
        if pm { "p.m." } else { "a.m." }
    )
}

/// Segundos desde 1970 → «24/9/2026».
pub fn date_label(secs: i64) -> String {
    use chrono::{Datelike, TimeZone};
    chrono::Local
        .timestamp_opt(secs, 0)
        .single()
        .map_or("—".into(), |t| {
            format!("{}/{}/{}", t.day(), t.month(), t.year())
        })
}

/// Lo que se hace con una tarea programada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CronAction {
    Pause,
    Resume,
    /// Ejecutar ahora, sin tocar su horario.
    Trigger,
    Delete,
}

impl CronAction {
    fn call(self, id: &str) -> (&'static str, String) {
        match self {
            CronAction::Pause => ("POST", format!("/api/cron/{id}/pause")),
            CronAction::Resume => ("POST", format!("/api/cron/{id}/resume")),
            CronAction::Trigger => ("POST", format!("/api/cron/{id}/trigger")),
            CronAction::Delete => ("DELETE", format!("/api/cron/{id}")),
        }
    }

    fn done(self) -> &'static str {
        match self {
            CronAction::Pause => "Tarea pausada",
            CronAction::Resume => "Tarea reanudada",
            CronAction::Trigger => "Tarea lanzada",
            CronAction::Delete => "Tarea eliminada",
        }
    }
}

/// La respuesta del servidor a una petición del API Client (`POST /api/http-request`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HttpReply {
    pub ok: bool,
    pub status: Option<u16>,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    /// El cuerpo ya formateado: JSON con sangría o el texto tal cual.
    pub body: String,
    pub error: Option<String>,
}

pub fn parse_http_reply(v: &Value) -> HttpReply {
    let headers = v["headers"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, x)| {
                    (
                        k.clone(),
                        x.as_str().map_or_else(|| x.to_string(), str::to_string),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let body = match &v["body"] {
        Value::Null => String::new(),
        Value::String(t) => serde_json::from_str::<Value>(t)
            .ok()
            .filter(|j| j.is_object() || j.is_array())
            .map_or_else(
                || t.clone(),
                |j| serde_json::to_string_pretty(&j).unwrap_or_else(|_| t.clone()),
            ),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    HttpReply {
        ok: v["ok"].as_bool().unwrap_or(false),
        status: v["status"].as_u64().map(|n| n as u16),
        status_text: s(v, "statusText"),
        headers,
        body,
        error: v["error"].as_str().map(str::to_string),
    }
}

/// El cuerpo del `POST /api/http-request`: las cabeceras y los parámetros activos con clave, y el cuerpo salvo en GET/HEAD.
pub fn http_payload(
    method: &str,
    url: &str,
    headers: &[(String, String, bool)],
    query: &[(String, String, bool)],
    body: &str,
) -> Result<Value, &'static str> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Escribe la URL");
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("La URL debe empezar por http:// o https://");
    }
    let pairs = |l: &[(String, String, bool)]| -> serde_json::Map<String, Value> {
        l.iter()
            .filter(|(k, _, on)| *on && !k.trim().is_empty())
            .map(|(k, v, _)| (k.trim().to_string(), json!(v)))
            .collect()
    };
    let mut p = json!({"method": method, "url": url, "headers": pairs(headers), "query_params": pairs(query)});
    if !body.trim().is_empty() && !matches!(method, "GET" | "HEAD") {
        p["body"] = json!(body.trim());
    }
    Ok(p)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub stats: Option<Stats>,
    pub activity: Vec<(String, u64)>,
    pub version: Option<Version>,
    pub usage: Option<Usage>,
    pub proposals: Vec<Proposal>,
    pub ethics: Vec<Ethic>,
    pub user: Option<User>,
    pub has_credentials: bool,
    pub notes: Vec<Note>,
    pub tasks: Vec<Task>,
    /// El historial leído de cada tarea (por id).
    pub history: std::collections::BTreeMap<String, Vec<Run>>,
    pub notes_loaded: bool,
    pub tasks_loaded: bool,
    pub dashboard_loaded: bool,
    pub ethics_loaded: bool,
    pub profile_loaded: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub unauthorized: bool,
}

/// Qué se quiere leer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Dashboard,
    Ethics,
    Profile,
    Notes,
    Cron,
}

#[derive(Default)]
pub struct Sys {
    pub data: Arc<Mutex<Data>>,
    pub shown: Data,
    last: Option<Instant>,
    busy: Arc<AtomicBool>,
}

fn get(conn: &Conn, path: &str) -> Result<Value, ApiError> {
    request(conn, "GET", path, None)
}

fn merge_err(d: &mut Data, e: ApiError) {
    d.unauthorized |= e == ApiError::Unauthorized;
    d.error = Some(e.to_string());
}

impl Sys {
    /// Copia lo que dejaron los hilos; devuelve si cambió algo visible.
    pub fn sync(&mut self) -> bool {
        let now = self.data.lock().unwrap().clone();
        if now == self.shown {
            return false;
        }
        self.shown = now;
        true
    }

    /// Lee `part` en un hilo (si no hay otra lectura en curso).
    pub fn refresh(&mut self, conn: &Conn, part: Part) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        self.last = Some(Instant::now());
        let (conn, data, busy) = (conn.clone(), self.data.clone(), self.busy.clone());
        std::thread::spawn(move || {
            match part {
                Part::Dashboard => {
                    // Cada endpoint por separado: si uno falla, los demás se muestran igual.
                    let stats = get(&conn, "/api/system-stats").map(|v| parse_stats(&v));
                    let activity =
                        get(&conn, "/api/activity-stats?hours=12").map(|v| parse_activity(&v));
                    let version = get(&conn, "/api/version").map(|v| parse_version(&v));
                    let usage = get(&conn, "/api/usage-stats?hours=24").map(|v| parse_usage(&v));
                    let proposals =
                        get(&conn, "/api/agents/proposals").map(|v| parse_proposals(&v));
                    let mut d = data.lock().unwrap();
                    d.dashboard_loaded = true;
                    d.error = None;
                    match stats {
                        Ok(x) => d.stats = Some(x),
                        Err(e) => merge_err(&mut d, e),
                    }
                    match activity {
                        Ok(x) => d.activity = x,
                        Err(e) => merge_err(&mut d, e),
                    }
                    match version {
                        Ok(x) => d.version = Some(x),
                        Err(e) => merge_err(&mut d, e),
                    }
                    match usage {
                        Ok(x) => d.usage = Some(x),
                        Err(e) => merge_err(&mut d, e),
                    }
                    match proposals {
                        Ok(x) => d.proposals = x,
                        Err(e) => merge_err(&mut d, e),
                    }
                }
                Part::Ethics => {
                    let r = get(&conn, "/api/ethics").map(|v| parse_ethics(&v));
                    let mut d = data.lock().unwrap();
                    d.ethics_loaded = true;
                    match r {
                        Ok(x) => {
                            d.ethics = x;
                            d.error = None
                        }
                        Err(e) => merge_err(&mut d, e),
                    }
                }
                Part::Notes => {
                    let r = get(&conn, "/api/notes").map(|v| parse_notes(&v));
                    let mut d = data.lock().unwrap();
                    d.notes_loaded = true;
                    match r {
                        Ok(x) => {
                            d.notes = x;
                            d.error = None
                        }
                        Err(e) => merge_err(&mut d, e),
                    }
                }
                Part::Cron => {
                    let r = get(&conn, "/api/cron").map(|v| parse_tasks(&v));
                    let mut d = data.lock().unwrap();
                    d.tasks_loaded = true;
                    match r {
                        Ok(x) => {
                            d.tasks = x;
                            d.error = None
                        }
                        Err(e) => merge_err(&mut d, e),
                    }
                }
                Part::Profile => {
                    let users = get(&conn, "/api/users").map(|v| parse_user(&v));
                    let auth = get(&conn, "/api/auth/status");
                    let mut d = data.lock().unwrap();
                    d.profile_loaded = true;
                    match users {
                        Ok(u) => {
                            d.user = u;
                            d.error = None
                        }
                        Err(e) => merge_err(&mut d, e),
                    }
                    if let Ok(a) = auth {
                        d.has_credentials = a["hasCredentials"].as_bool().unwrap_or(false);
                    }
                }
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Que la próxima `tick` lea sin esperar (al abrir el Dashboard).
    /// Si hay una lectura en curso.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn reset_clock(&mut self) {
        self.last = None;
    }

    /// Mientras se mira el Dashboard, relee las métricas cada `REFRESH`.
    pub fn tick(&mut self, conn: Option<&Conn>) {
        let Some(conn) = conn else { return };
        if self.last.is_none_or(|t| t.elapsed() >= REFRESH) {
            self.refresh(conn, Part::Dashboard);
        }
    }

    /// Pausa, reanuda, lanza o borra una tarea programada y vuelve a leer la lista.
    pub fn cron_action(&mut self, conn: &Conn, id: &str, action: CronAction) {
        let (conn2, data, id) = (conn.clone(), self.data.clone(), id.to_string());
        std::thread::spawn(move || {
            let (method, path) = action.call(&crate::chat::urlenc(&id));
            let r = request(&conn2, method, &path, None);
            let list = get(&conn2, "/api/cron").map(|v| parse_tasks(&v));
            let mut d = data.lock().unwrap();
            match r {
                Ok(_) => {
                    d.notice = Some(action.done().into());
                    d.error = None;
                }
                Err(e) => merge_err(&mut d, e),
            }
            if let Ok(t) = list {
                d.tasks = t;
            }
        });
    }

    /// Lee las últimas ejecuciones de una tarea.
    pub fn read_history(&mut self, conn: &Conn, id: &str) {
        let (conn2, data, id) = (conn.clone(), self.data.clone(), id.to_string());
        std::thread::spawn(move || {
            let r = get(
                &conn2,
                &format!("/api/cron/{}/history?limit=10", crate::chat::urlenc(&id)),
            )
            .map(|v| parse_history(&v));
            let mut d = data.lock().unwrap();
            match r {
                Ok(runs) => {
                    d.history.insert(id, runs);
                }
                Err(e) => merge_err(&mut d, e),
            }
        });
    }

    /// Guarda cambios del perfil (`PATCH /api/user/settings?userId=…`) y vuelve a leerlo.
    pub fn save_profile(&mut self, conn: &Conn, user_id: &str, patch: Value) {
        let (conn2, data, id) = (conn.clone(), self.data.clone(), user_id.to_string());
        std::thread::spawn(move || {
            let r = request(
                &conn2,
                "PATCH",
                &format!("/api/user/settings?userId={}", crate::chat::urlenc(&id)),
                Some(patch),
            );
            let users = get(&conn2, "/api/users").map(|v| parse_user(&v));
            let mut d = data.lock().unwrap();
            match r {
                Ok(_) => {
                    d.notice = Some("Perfil guardado".into());
                    d.error = None;
                }
                Err(e) => merge_err(&mut d, e),
            }
            if let Ok(Some(u)) = users {
                d.user = Some(u);
            }
        });
    }
}

/// Construye el cuerpo del `PATCH` del perfil; el error es lo que se le dice a la persona.
pub fn profile_patch(
    name: &str,
    occupation: &str,
    email: &str,
    language: &str,
    timezone: &str,
    notes: &str,
) -> Result<Value, &'static str> {
    let (name, email) = (name.trim(), email.trim());
    if name.is_empty() {
        return Err("Escribe tu nombre");
    }
    if !email.is_empty()
        && !(email.contains('@') && email.rsplit('@').next().is_some_and(|d| d.contains('.')))
    {
        return Err("El correo no parece válido");
    }
    Ok(
        json!({"name": name, "occupation": occupation.trim(), "email": email, "language": language.trim(), "timezone": timezone.trim(), "notes": notes.trim()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_are_read_from_the_real_payload() {
        let v = json!({"cpu": 0.1, "memory": {"rss": 239, "heapUsed": 49, "heapTotal": 27, "heapPercent": 182.2, "external": 30}, "uptime": "00:03:07", "connections": 1, "cores": 16, "recentMessages": 2});
        let st = parse_stats(&v);
        assert_eq!(
            (
                st.rss,
                st.heap_used,
                st.external,
                st.connections,
                st.cores,
                st.recent_messages
            ),
            (239.0, 49.0, 30.0, 1, 16, 2)
        );
        assert_eq!((st.cpu, st.uptime.as_str()), (0.1, "00:03:07"));
        // Sin el campo, se asume que las métricas del proceso están disponibles (un servidor antiguo no las anuncia).
        assert_eq!(
            parse_stats(&json!({})),
            Stats {
                process_metrics_available: true,
                ..Stats::default()
            },
            "un payload vacío no rompe nada"
        );
    }

    #[test]
    fn activity_is_listed_by_hour_and_the_hour_label_is_short() {
        let a = parse_activity(
            &json!([{"time": "2026-11-08 03:00", "count": 7}, {"time": "2026-11-08 15:00", "count": 2}]),
        );
        assert_eq!(
            a,
            [
                ("2026-11-08 03:00".to_string(), 7),
                ("2026-11-08 15:00".to_string(), 2)
            ]
        );
        assert_eq!(hour_label(&a[1].0), "15:00");
        assert!(parse_activity(&json!({"no": "lista"})).is_empty());
    }

    #[test]
    fn version_says_whether_it_is_up_to_date() {
        let v = parse_version(
            &json!({"current": "1.1.4", "latest": "1.1.4", "status": "up-to-date", "installationType": "binary"}),
        );
        assert!(v.up_to_date());
        assert_eq!(
            (v.current.as_str(), v.installation.as_str()),
            ("1.1.4", "binary")
        );
        assert!(!parse_version(
            &json!({"current": "1.1.4", "latest": "1.2.0", "status": "update-available"})
        )
        .up_to_date());
    }

    #[test]
    fn usage_orders_providers_by_tokens_and_reads_the_savings() {
        let u = parse_usage(
            &json!({"totalTokens": 569539, "totalInputTokens": 539114, "totalOutputTokens": 30425, "totalCostUsd": 0.146, "toonSavedTokens": 765,
            "byProvider": {"hiveagents": {"tokens": 313177, "costUsd": 0}, "openrouter": {"tokens": 71628, "costUsd": 0.0025}, "gemini": {"tokens": 184734, "costUsd": 0.143}}}),
        );
        assert_eq!(
            (u.tokens, u.input, u.output, u.toon_saved_tokens),
            (569539, 539114, 30425, 765)
        );
        assert_eq!(
            u.providers.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
            ["hiveagents", "gemini", "openrouter"]
        );
        assert!((u.cost - 0.146).abs() < 1e-9);
    }

    #[test]
    fn proposals_and_their_reasons_read_like_the_webs() {
        let p = parse_proposals(
            &json!({"proposals": [{"agentName": "Cron", "reason": "harmful_count exceeded helpful_count"}, {"agentName": "X", "reason": null}]}),
        );
        assert_eq!(p.len(), 2);
        assert_eq!(
            describe_reason(&p[0].reason),
            "más resultados dañinos que útiles en sus verificaciones"
        );
        assert_eq!(describe_reason(&p[1].reason), "revisión pendiente");
        assert_eq!(
            describe_reason("otro motivo"),
            "otro motivo",
            "lo que no se sabe traducir se muestra tal cual"
        );
    }

    #[test]
    fn ethics_and_the_user_come_from_the_real_shapes() {
        let e = parse_ethics(
            &json!({"ethics": [{"id": "default", "name": "Ética por Defecto", "description": "Lineamientos", "content": "##ALWAYS", "active": true}, {"name": "sin id"}]}),
        );
        assert_eq!(e.len(), 1, "una fila sin id se ignora");
        assert!(e[0].active && e[0].name == "Ética por Defecto");
        let u = parse_user(&json!({"users": [{"id": "u1", "name": "John Paez", "email": "a@b.co", "language": "es", "timezone": "America/Bogota", "occupation": "Desarrollador", "notes": "Concreto", "preferred_cron_channel": "auto"}]})).unwrap();
        assert_eq!(
            (
                u.name.as_str(),
                u.timezone.as_str(),
                u.cron_channel.as_str()
            ),
            ("John Paez", "America/Bogota", "auto")
        );
        assert!(parse_user(&json!({"users": []})).is_none());
        assert_eq!(u.initials(), "JP");
        assert_eq!(
            User {
                name: "ana".into(),
                ..Default::default()
            }
            .initials(),
            "A"
        );
        assert_eq!(User::default().initials(), "?");
        assert_eq!(
            User {
                name: "a b c".into(),
                ..Default::default()
            }
            .initials(),
            "AB",
            "como mucho dos letras"
        );
    }

    #[test]
    fn the_profile_patch_validates_and_trims() {
        assert_eq!(
            profile_patch(" John ", "Dev ", "a@b.co", "es", "America/Bogota", "x").unwrap(),
            json!({"name": "John", "occupation": "Dev", "email": "a@b.co", "language": "es", "timezone": "America/Bogota", "notes": "x"})
        );
        assert_eq!(
            profile_patch("  ", "", "", "", "", ""),
            Err("Escribe tu nombre")
        );
        assert_eq!(
            profile_patch("J", "", "sin-arroba", "", "", ""),
            Err("El correo no parece válido")
        );
        assert_eq!(
            profile_patch("J", "", "a@sinpunto", "", "", ""),
            Err("El correo no parece válido")
        );
        assert!(
            profile_patch("J", "", "", "", "", "").is_ok(),
            "el correo es opcional"
        );
    }

    #[test]
    fn refresh_reads_every_dashboard_endpoint_once_and_survives_one_failing() {
        use crate::testutil::fake_http;
        let (port, server) = fake_http(
            |_, p, _, _| match p.split('?').next().unwrap() {
                "/api/system-stats" => (
                    200,
                    json!({"cpu": 1.5, "memory": {"rss": 100}, "connections": 2, "cores": 8}),
                ),
                "/api/activity-stats" => (200, json!([{"time": "2026-11-08 10:00", "count": 4}])),
                "/api/version" => (500, json!({"error": "sin red"})),
                "/api/usage-stats" => (200, json!({"totalTokens": 10})),
                _ => (200, json!({"proposals": []})),
            },
            5,
        );
        let conn = Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: None,
        };
        let mut sys = Sys::default();
        sys.refresh(&conn, Part::Dashboard);
        let paths: Vec<String> = server.join().unwrap().into_iter().map(|r| r.1).collect();
        assert_eq!(
            paths,
            [
                "/api/system-stats",
                "/api/activity-stats?hours=12",
                "/api/version",
                "/api/usage-stats?hours=24",
                "/api/agents/proposals"
            ]
        );
        let end = Instant::now() + Duration::from_secs(3);
        while !sys.sync() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            sys.shown.stats.as_ref().map(|s| (s.cpu, s.connections)),
            Some((1.5, 2))
        );
        assert_eq!(sys.shown.activity.len(), 1);
        assert!(sys.shown.version.is_none(), "la versión falló");
        assert_eq!(sys.shown.usage.as_ref().map(|u| u.tokens), Some(10));
        assert!(
            sys.shown.error.is_some(),
            "se avisa del fallo, sin perder el resto"
        );
    }

    #[test]
    fn notes_read_the_real_shape_and_tag_the_thread() {
        let n = parse_notes(
            &json!({"notes": [{"id": "u/webchat/conv-e5992b532870:capacidades", "thread_id": "u/webchat/conv-e5992b532870", "key": "capacidades_y_firma_correo", "value": "Capacidades:\n- A", "source": "agent", "created_at": 1790280173}, {"key": "sin id"}]}),
        );
        assert_eq!(n.len(), 1, "sin id se ignora");
        assert_eq!(
            n[0].thread_tag(),
            "2b532870",
            "los últimos 8 caracteres del id de la conversación, como la etiqueta de la web"
        );
        assert_eq!(n[0].created_at, 1790280173);
        assert_eq!(
            Note {
                thread_id: "x/conv-abc".into(),
                ..Default::default()
            }
            .thread_tag(),
            "abc"
        );
        assert_eq!(Note::default().thread_tag(), "");
    }

    #[test]
    fn tasks_read_the_real_shape_and_hide_the_internal_ones() {
        let t = parse_tasks(&json!({"tasks": [
            {"id": "a", "name": "_hive_cleanup_runs", "status": "active", "task_type": "recurring", "cron_expression": "0 4 * * *", "channel": "system", "payload": "{\"_internal\":true}"},
            {"id": "b", "name": "reporte-diario-tareas", "status": "active", "task_type": "recurring", "cron_expression": "0 8 * * 1-5", "channel": "cli", "timezone": "America/Bogota", "run_count": 0, "last_error": "Missed run at 2026-11-05T13:00:00.000Z (policy: skip)", "next_run_at": "2026-11-09T13:00:00.000Z", "payload": "{\"prompt\":\"x\"}", "task": "Generar reporte"},
            {"name": "sin id"}]}));
        assert_eq!(t.len(), 2);
        assert!(t[0].internal && !t[1].internal);
        assert_eq!(
            (
                t[1].cron.as_str(),
                t[1].channel.as_str(),
                t[1].timezone.as_str()
            ),
            ("0 8 * * 1-5", "cli", "America/Bogota")
        );
        assert!(t[1].last_error.starts_with("Missed run"));
        let _ = parse_history(&json!({}));
    }

    #[test]
    fn history_and_dates_read_like_the_webs() {
        let h = parse_history(
            &json!({"history": [{"status": "success", "started_at": "2026-11-08T07:00:00.003Z", "duration_ms": 32534, "agent_response": "He delegado", "error_message": null}]}),
        );
        assert_eq!(
            (
                h[0].status.as_str(),
                h[0].duration_ms,
                h[0].response.as_str()
            ),
            ("success", 32534, "He delegado")
        );
        assert_eq!(when_label(""), "—");
        assert_eq!(when_label("no es fecha"), "—");
        let w = when_label("2026-11-09T13:00:00.000Z");
        assert!(
            w.contains("nov") && (w.ends_with("a.m.") || w.ends_with("p.m.")),
            "{w}"
        );
        assert_eq!(date_label(1790280173).split('/').nth(2), Some("2026"));
    }

    #[test]
    fn cron_actions_map_to_the_webs_endpoints() {
        assert_eq!(
            CronAction::Pause.call("x"),
            ("POST", "/api/cron/x/pause".to_string())
        );
        assert_eq!(
            CronAction::Resume.call("x"),
            ("POST", "/api/cron/x/resume".to_string())
        );
        assert_eq!(
            CronAction::Trigger.call("x"),
            ("POST", "/api/cron/x/trigger".to_string())
        );
        assert_eq!(
            CronAction::Delete.call("x"),
            ("DELETE", "/api/cron/x".to_string())
        );
    }

    #[test]
    fn the_http_payload_validates_the_url_and_keeps_only_active_named_pairs() {
        let h = vec![
            ("Accept".to_string(), "json".to_string(), true),
            (" ".to_string(), "x".to_string(), true),
            ("Off".to_string(), "y".to_string(), false),
        ];
        let q = vec![("page".to_string(), "2".to_string(), true)];
        let p = http_payload("POST", " https://api.example.com/x ", &h, &q, " {\"a\":1} ").unwrap();
        assert_eq!(
            p,
            json!({"method": "POST", "url": "https://api.example.com/x", "headers": {"Accept": "json"}, "query_params": {"page": "2"}, "body": "{\"a\":1}"})
        );
        assert!(
            http_payload("GET", "https://a.b", &[], &[], "{}")
                .unwrap()
                .get("body")
                .is_none(),
            "GET no lleva cuerpo"
        );
        assert!(http_payload("HEAD", "https://a.b", &[], &[], "{}")
            .unwrap()
            .get("body")
            .is_none());
        assert_eq!(
            http_payload("GET", "  ", &[], &[], ""),
            Err("Escribe la URL")
        );
        assert_eq!(
            http_payload("GET", "ftp://x", &[], &[], ""),
            Err("La URL debe empezar por http:// o https://")
        );
    }

    #[test]
    fn the_http_reply_pretty_prints_json_bodies() {
        let r = parse_http_reply(
            &json!({"ok": true, "status": 200, "statusText": "OK", "headers": {"content-type": "application/json"}, "body": "{\"a\":1}"}),
        );
        assert!(r.ok && r.status == Some(200));
        assert_eq!(r.body, "{\n  \"a\": 1\n}");
        assert_eq!(
            r.headers,
            [("content-type".to_string(), "application/json".to_string())]
        );
        assert_eq!(
            parse_http_reply(&json!({"ok": true, "body": "texto plano"})).body,
            "texto plano"
        );
        assert_eq!(
            parse_http_reply(&json!({"ok": true, "body": {"x": [1]}})).body,
            "{\n  \"x\": [\n    1\n  ]\n}"
        );
        let e = parse_http_reply(&json!({"ok": false, "error": "timeout"}));
        assert_eq!(
            (e.ok, e.error.as_deref(), e.status),
            (false, Some("timeout"), None)
        );
    }
}
