//! Cliente REST de los ajustes del gateway. Rutas y formas según `packages/core/src/gateway/routes`.
//! Las claves nunca vuelven completas: el servidor solo manda `has_api_key` y una versión enmascarada.

use std::fmt;

use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::chat::urlenc;

#[derive(Clone, Debug, PartialEq)]
pub struct Conn {
    pub base: String,
    pub token: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ApiError {
    Unauthorized,
    Status(u16, String),
    Net(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "Sesión vencida: vuelve a iniciar sesión"),
            ApiError::Status(code, msg) if msg.is_empty() => {
                write!(f, "El gateway respondió {code}")
            }
            ApiError::Status(_, msg) => write!(f, "{msg}"),
            ApiError::Net(e) => write!(f, "No se pudo contactar al gateway: {e}"),
        }
    }
}

/// Texto legible de un cuerpo de error: `{"error":".."}`, `{"message":".."}` o texto plano.
fn error_text(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(v) => v["error"]
            .as_str()
            .or_else(|| v["message"].as_str())
            .unwrap_or("")
            .to_string(),
        Err(_) => body.trim().to_string(),
    }
}

pub(crate) fn request(
    conn: &Conn,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ApiError> {
    let mut req = ureq::request(method, &format!("{}{path}", conn.base));
    if let Some(t) = &conn.token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let resp = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match resp {
        Ok(r) => Ok(r.into_json().unwrap_or(Value::Null)),
        Err(ureq::Error::Status(401, _)) => Err(ApiError::Unauthorized),
        Err(ureq::Error::Status(code, r)) => Err(ApiError::Status(
            code,
            error_text(&r.into_string().unwrap_or_default()),
        )),
        Err(e) => Err(ApiError::Net(e.to_string())),
    }
}

/// `true`/`false` o `1`/`0` (el servidor manda `has_api_key` como entero).
fn truthy<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    })
}

fn text_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

fn string_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Value::deserialize(d)?;
    let v = if let Some(s) = v.as_str() {
        serde_json::from_str(s).unwrap_or(Value::Null)
    } else {
        v
    };
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .collect())
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub model_type: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default, deserialize_with = "string_list")]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub default_for: Vec<String>,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub availability_note: String,
}

impl Model {
    pub fn is_active(&self) -> bool {
        self.enabled && self.active.unwrap_or(self.enabled)
    }
    pub fn has(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }
    pub fn in_category(&self, cat: u8) -> bool {
        match cat {
            0 => {
                self.model_type.is_empty()
                    || matches!(self.model_type.as_str(), "llm" | "unknown" | "embedding")
            }
            1 => {
                matches!(self.model_type.as_str(), "vision" | "image" | "ocr")
                    || self.has("vision")
                    || self.has("image_generation")
                    || self.has("ocr")
            }
            _ => {
                matches!(
                    self.model_type.as_str(),
                    "stt" | "tts" | "realtime" | "voice" | "audio"
                ) || ["transcription", "tts", "speech", "audio_in", "audio_out"]
                    .iter()
                    .any(|c| self.has(c))
            }
        }
    }
    pub fn purpose(&self, cat: u8) -> Option<&'static str> {
        match cat {
            1 if (self.model_type == "llm" && self.has("vision"))
                || (self.model_type == "vision" && !self.has("ocr")) =>
            {
                Some("vision")
            }
            2 if self.model_type == "stt" && !self.has("realtime") => Some("stt"),
            2 if self.model_type == "tts" => Some("tts"),
            _ => None,
        }
    }
    pub fn kind_label(&self) -> &str {
        if self.has("image_generation") {
            "Generación de imágenes"
        } else if self.has("vision") {
            "Visión"
        } else {
            match self.model_type.as_str() {
                "stt" => "Transcripción",
                "tts" => "Síntesis de voz",
                "realtime" => "Conversación en vivo",
                "unknown" => "Capacidades sin publicar",
                _ => &self.model_type,
            }
        }
    }
    pub fn for_agent(&self) -> bool {
        self.enabled && (self.model_type.is_empty() || self.model_type == "llm")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Provider {
    pub id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub category: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default, deserialize_with = "truthy")]
    pub has_api_key: bool,
    #[serde(default)]
    pub masked_api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<Model>,
    /// Estado de Jev que el servidor adjunta a `openrouter` y `hiveagents` (forma sin documentar: se guarda en bruto).
    #[serde(default)]
    pub jev: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub description: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub tone: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub role: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub status: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub provider_id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub model_id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub system_prompt: String,
    /// Las herramientas y skills llegan como un texto JSON con la lista de nombres.
    #[serde(default, deserialize_with = "text_or_empty")]
    pub tools_json: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub skills_json: String,
    #[serde(default)]
    pub max_iterations: Option<u64>,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub workspace: String,
}

impl Agent {
    pub fn is_coordinator(&self) -> bool {
        self.role == "coordinator"
    }

    /// Nombres de la lista JSON de `toolsJson`/`skillsJson`; vacío si falta o está mal formada.
    fn names(json: &str) -> Vec<String> {
        serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
    }

    pub fn tools(&self) -> Vec<String> {
        Self::names(&self.tools_json)
    }

    pub fn skills(&self) -> Vec<String> {
        Self::names(&self.skills_json)
    }
}

/// El coordinador de la colmena: **uno solo**. Si el servidor tuviera varios marcados como `coordinator`
/// (el `POST /api/agents` de REST da ese rol a todo agente nuevo), el primero manda y el resto son especialistas.
pub fn core_id(agents: &[Agent]) -> Option<&str> {
    agents
        .iter()
        .find(|a| a.is_coordinator())
        .map(|a| a.id.as_str())
}

/// Los agentes marcados como coordinador que sobran (todos menos el que manda).
pub fn extra_coordinators(agents: &[Agent]) -> Vec<&Agent> {
    let core = core_id(agents);
    agents
        .iter()
        .filter(|a| a.is_coordinator() && Some(a.id.as_str()) != core)
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    #[serde(default, rename = "type", deserialize_with = "text_or_empty")]
    pub kind: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub status: String,
    #[serde(default)]
    pub is_configured: bool,
}

/// Herramienta del agente. `active` y `enabled` llegan como booleano o como 0/1.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Tool {
    pub id: String,
    #[serde(default)]
    pub runtime_available: Option<bool>,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub description: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub category: String,
    #[serde(default, deserialize_with = "truthy")]
    pub active: bool,
    #[serde(default, deserialize_with = "truthy")]
    pub enabled: bool,
}

/// Skill del agente (se ignora el cuerpo: puede ser enorme).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Skill {
    pub id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub description: String,
    #[serde(default, deserialize_with = "truthy")]
    pub active: bool,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub category: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub version: String,
    /// Herramientas y disparadores llegan como una lista separada por comas.
    #[serde(default, deserialize_with = "text_or_empty")]
    pub tools: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub triggers: String,
}

impl Skill {
    fn count(list: &str) -> usize {
        list.split(',').filter(|x| !x.trim().is_empty()).count()
    }

    pub fn tool_count(&self) -> usize {
        Self::count(&self.tools)
    }

    pub fn trigger_count(&self) -> usize {
        Self::count(&self.triggers)
    }
}

/// Servidor MCP conectado al gateway. Solo lectura.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct McpServer {
    pub id: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub status: String,
    #[serde(default, deserialize_with = "truthy")]
    pub enabled: bool,
    #[serde(default, rename = "toolsCount", alias = "tools_count")]
    pub tools_count: Option<u64>,
    /// Según la versión llega como lista, como número o no llega.
    #[serde(default)]
    pub tools: Value,
    /// `{transport, url, command, args…}`.
    #[serde(default)]
    pub config: Value,
}

impl McpServer {
    pub fn transport(&self) -> String {
        self.config["transport"].as_str().unwrap_or("").to_string()
    }

    pub fn url(&self) -> String {
        self.config["url"].as_str().unwrap_or("").to_string()
    }

    pub fn tool_count(&self) -> Option<u64> {
        self.tools_count.or_else(|| match &self.tools {
            Value::Array(a) => Some(a.len() as u64),
            Value::Number(n) => n.as_u64(),
            _ => None,
        })
    }
}

fn list<T: for<'de> Deserialize<'de>>(
    conn: &Conn,
    path: &str,
    key: &str,
) -> Result<Vec<T>, ApiError> {
    let v = request(conn, "GET", path, None)?;
    serde_json::from_value(v.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| ApiError::Net(format!("respuesta inesperada de {path}: {e}")))
}

pub fn providers(conn: &Conn) -> Result<Vec<Provider>, ApiError> {
    list(conn, "/api/providers", "providers")
}

pub fn agents(conn: &Conn) -> Result<Vec<Agent>, ApiError> {
    list(conn, "/api/agents", "agents")
}

pub fn channels(conn: &Conn) -> Result<Vec<Channel>, ApiError> {
    list(conn, "/api/channels", "channels")
}

pub fn tools(conn: &Conn) -> Result<Vec<Tool>, ApiError> {
    list(conn, "/api/tools", "tools")
}

/// Acepta la lista suelta o envuelta en `servers`, porque la forma exacta no está documentada.
pub fn mcp_servers(conn: &Conn) -> Result<Vec<McpServer>, ApiError> {
    let v = request(conn, "GET", "/api/mcp/servers", None)?;
    let items = if v.is_array() {
        v
    } else {
        v.get("servers").cloned().unwrap_or(Value::Null)
    };
    serde_json::from_value(items)
        .map_err(|e| ApiError::Net(format!("respuesta inesperada de /api/mcp/servers: {e}")))
}

pub fn skills(conn: &Conn) -> Result<Vec<Skill>, ApiError> {
    list(conn, "/api/skills", "skills")
}

/// Activa o desactiva una herramienta para todos los agentes.
pub fn toggle_tool(conn: &Conn, id: &str, active: bool) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/tools/{}/toggle", urlenc(id)),
        Some(json!({"active": active})),
    )
    .map(drop)
}

pub fn toggle_skill(conn: &Conn, id: &str, active: bool) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/skills/{}/toggle", urlenc(id)),
        Some(json!({"active": active})),
    )
    .map(drop)
}

/// Activa o desactiva un proveedor (y, en cascada, todos sus modelos).
/// Borra una superficie A2UI del servidor (la papelera del Panel interactivo de la web).
pub fn delete_surface(conn: &Conn, id: &str) -> Result<(), ApiError> {
    request(
        conn,
        "DELETE",
        &format!("/api/a2ui/surfaces/{}", urlenc(id)),
        None,
    )
    .map(drop)
}

pub fn toggle_model(conn: &Conn, id: &str, active: bool) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/models/{}/toggle", urlenc(id)),
        Some(json!({"active": active})),
    )
    .map(drop)
}

pub fn toggle_provider(conn: &Conn, id: &str, active: bool) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/providers/{}/toggle", urlenc(id)),
        Some(json!({"active": active})),
    )
    .map(drop)
}

/// Guarda o reemplaza la clave del proveedor. Va en claro en el cuerpo; el servidor la cifra.
pub fn set_api_key(conn: &Conn, id: &str, key: &str) -> Result<(), ApiError> {
    request(
        conn,
        "PUT",
        &format!("/api/providers/{}", urlenc(id)),
        Some(json!({"apiKey": key.trim()})),
    )
    .map(drop)
}

/// Borra la clave guardada del proveedor.
pub fn clear_api_key(conn: &Conn, id: &str) -> Result<(), ApiError> {
    request(
        conn,
        "PUT",
        &format!("/api/providers/{}", urlenc(id)),
        Some(json!({"clearApiKey": true})),
    )
    .map(drop)
}

/// Comprueba una clave contra el proveedor sin guardarla. Un fallo de la clave llega como 200 con
/// `success:false`, así que se lee el cuerpo.
pub fn verify_key(conn: &Conn, provider: &str, key: &str) -> Result<(), ApiError> {
    let v = request(
        conn,
        "POST",
        "/api/setup/verify-provider",
        Some(json!({"provider": provider, "apiKey": key.trim()})),
    )?;
    if v["success"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(ApiError::Status(
            200,
            v["error"]
                .as_str()
                .unwrap_or("La clave no es válida")
                .to_string(),
        ))
    }
}

/// Cambia campos del agente (`name`, `description`, `tone`, …).
/// Crea un agente (`POST /api/agents`) y devuelve su id. El servidor no guarda el prompt del sistema al crear:
/// quien llama lo manda después con `update_agent`.
pub fn create_agent(conn: &Conn, body: Value) -> Result<String, ApiError> {
    let v = request(conn, "POST", "/api/agents", Some(body))?;
    v["agent"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::Net("respuesta sin id de agente".into()))
}

pub fn update_agent(conn: &Conn, id: &str, patch: Value) -> Result<(), ApiError> {
    request(
        conn,
        "PUT",
        &format!("/api/agents/{}", urlenc(id)),
        Some(patch),
    )
    .map(drop)
}

/// Crea un canal (`config` lleva el token del bot y, según el tipo, `applicationId` o `signingSecret`).
pub fn create_channel(conn: &Conn, kind: &str, config: Value) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        "/api/channels",
        Some(json!({"type": kind, "config": config})),
    )
    .map(drop)
}

/// Estado de conexión de un canal; para WhatsApp trae el texto del QR mientras espera el escaneo.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    #[serde(default, deserialize_with = "text_or_empty")]
    pub status: String,
    #[serde(default)]
    pub qr_code: Option<String>,
}

pub fn channel_status(conn: &Conn, kind: &str, id: &str) -> Result<ChannelStatus, ApiError> {
    let v = request(
        conn,
        "GET",
        &format!("/api/channels/{}/{}/status", urlenc(kind), urlenc(id)),
        None,
    )?;
    serde_json::from_value(v).map_err(|e| ApiError::Net(e.to_string()))
}

/// Reconecta el canal sin cambiar su configuración (en WhatsApp genera un QR nuevo).
pub fn reconnect_channel(conn: &Conn, id: &str) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/channels/{}/reconnect", urlenc(id)),
        Some(json!({"config": {}})),
    )
    .map(drop)
}

/// Cambia el token del bot y reconecta el canal (lo mismo que hace la web con «Cambiar token»).
pub fn set_channel_token(conn: &Conn, id: &str, token: &str) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/channels/{}/reconnect", urlenc(id)),
        Some(json!({"config": {"botToken": token.trim()}})),
    )
    .map(drop)
}

pub fn toggle_channel(conn: &Conn, id: &str, active: bool) -> Result<(), ApiError> {
    request(
        conn,
        "POST",
        &format!("/api/channels/{}/toggle", urlenc(id)),
        Some(json!({"active": active})),
    )
    .map(drop)
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn an_agent_exposes_its_tools_skills_and_limits() {
        let a: Agent = serde_json::from_value(json!({
            "id": "a", "name": "Código", "role": "worker", "toolsJson": "[\"cli_exec\",\"fs_read\"]", "skillsJson": "[\"sw\"]",
            "maxIterations": 20, "workspace": null, "systemPrompt": "# ROL"
        })).unwrap();
        assert_eq!(
            (a.tools().len(), a.skills().len(), a.max_iterations),
            (2, 1, Some(20))
        );
        assert_eq!(a.workspace, "", "un workspace nulo es vacío");
        assert!(!a.is_coordinator());
        let mk = |id: &str, role: &str| -> Agent {
            serde_json::from_value(json!({"id": id, "role": role})).unwrap()
        };
        let list = vec![
            mk("w1", "worker"),
            mk("boss", "coordinator"),
            mk("w2", "worker"),
            mk("falso", "coordinator"),
        ];
        assert_eq!(
            core_id(&list),
            Some("boss"),
            "el primero marcado como coordinador manda"
        );
        assert_eq!(
            extra_coordinators(&list)
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["falso"],
            "el otro sobra"
        );
        assert_eq!(core_id(&[mk("solo", "worker")]), None);
        assert!(extra_coordinators(&[mk("boss", "coordinator"), mk("w", "worker")]).is_empty());
        let bad: Agent =
            serde_json::from_value(json!({"id": "b", "toolsJson": "no es json"})).unwrap();
        assert!(
            bad.tools().is_empty(),
            "una lista mal formada no rompe nada"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fake_http;

    fn conn(port: u16, token: Option<&str>) -> Conn {
        Conn {
            base: format!("http://127.0.0.1:{port}"),
            token: token.map(str::to_string),
        }
    }

    #[test]
    fn multimodal_capabilities_accept_json_strings_arrays_and_null() {
        let vision:Model=serde_json::from_value(json!({"id":"gemini-eye","model_type":"llm","enabled":true,"capabilities":"[\"vision\",\"chat\"]"})).unwrap();
        assert!(vision.in_category(0) && vision.in_category(1));
        assert_eq!(vision.purpose(1), Some("vision"));
        let image: Model = serde_json::from_value(
            json!({"id":"paint","model_type":"image","capabilities":["image_generation"]}),
        )
        .unwrap();
        assert!(image.in_category(1));
        assert_eq!(image.purpose(1), None);
        assert!(!image.for_agent());
        let tts: Model = serde_json::from_value(
            json!({"id":"gemini-tts","model_type":"tts","capabilities":["tts","speech"]}),
        )
        .unwrap();
        assert!(tts.in_category(2));
        assert_eq!(tts.purpose(2), Some("tts"));
        assert!(!tts.in_category(0));
        let blank: Model =
            serde_json::from_value(json!({"id":"blank","capabilities":null})).unwrap();
        assert!(blank.capabilities.is_empty());
        let inactive: Model =
            serde_json::from_value(json!({"id":"available","enabled":true,"active":false}))
                .unwrap();
        assert!(!inactive.is_active());
        let legacy: Model = serde_json::from_value(json!({"id":"legacy","enabled":true})).unwrap();
        assert!(legacy.is_active());
    }

    #[test]
    fn providers_parse_the_real_response_shape() {
        let body = json!({"providers": [{
            "id": "openai", "name": "OpenAI", "base_url": "https://api.openai.com/v1", "category": "llm",
            "enabled": true, "active": true, "num_ctx": null, "has_api_key": 1, "has_headers": 0,
            "masked_api_key": "sk-p••••••••abcd",
            "models": [{"id": "openai/gpt-4o", "provider_id": "openai", "name": "gpt-4o", "model_type": "llm", "context_window": 128000,
                        "capabilities": null, "enabled": true, "active": true, "source": "catalog", "wire_id": "gpt-4o"}]
        }, {"id": "ollama", "name": null, "category": "llm", "has_api_key": 0, "masked_api_key": null, "models": []}]});
        let (port, server) = fake_http(move |_, _, _, _| (200, body.clone()), 1);
        let got = providers(&conn(port, Some("tok"))).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].has_api_key && got[0].enabled && got[0].active);
        assert_eq!(got[0].masked_api_key.as_deref(), Some("sk-p••••••••abcd"));
        assert_eq!(
            got[0].models[0],
            Model {
                id: "openai/gpt-4o".into(),
                name: "gpt-4o".into(),
                enabled: true,
                context_window: Some(128000),
                model_type: "llm".into(),
                active: Some(true),
                ..Default::default()
            }
        );
        assert!(!got[1].has_api_key && got[1].name.is_empty() && got[1].masked_api_key.is_none());
        let seen = server.join().unwrap();
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str(), seen[0].2.as_deref()),
            ("GET", "/api/providers", Some("Bearer tok"))
        );
    }

    #[test]
    fn agents_and_channels_parse() {
        let (port, server) = fake_http(
            |_, path, _, _| match path {
                "/api/agents" => (
                    200,
                    json!({"agents": [{"id": "a1", "name": "Hive", "description": null, "tone": "friendly", "role": "coordinator",
                    "status": "idle", "enabled": true, "providerId": "openai", "modelId": "openai/gpt-4o", "toolsJson": null}]}),
                ),
                _ => (
                    200,
                    json!({"channels": [{"id": "telegram", "type": "telegram", "enabled": true, "active": true, "status": "connected", "isConfigured": true},
                    {"id": "webchat", "type": "webchat", "enabled": true, "active": true}]}),
                ),
            },
            2,
        );
        let c = conn(port, None);
        let a = agents(&c).unwrap();
        assert_eq!(
            (
                a[0].name.as_str(),
                a[0].description.as_str(),
                a[0].provider_id.as_str(),
                a[0].role.as_str()
            ),
            ("Hive", "", "openai", "coordinator")
        );
        let ch = channels(&c).unwrap();
        assert_eq!(
            (
                ch[0].kind.as_str(),
                ch[0].status.as_str(),
                ch[0].is_configured
            ),
            ("telegram", "connected", true)
        );
        assert!(ch[1].status.is_empty() && !ch[1].is_configured);
        server.join().unwrap();
    }

    #[test]
    fn mutations_use_the_documented_methods_paths_and_bodies() {
        let (port, server) = fake_http(|_, _, _, _| (200, json!({"ok": true, "success": true})), 8);
        let c = conn(port, Some("t"));
        toggle_provider(&c, "openai", false).unwrap();
        set_api_key(&c, "openai", "  sk-nueva  ").unwrap();
        clear_api_key(&c, "openai").unwrap();
        update_agent(&c, "a1", json!({"name": "Pepe", "tone": "formal"})).unwrap();
        toggle_channel(&c, "telegram", true).unwrap();
        toggle_model(&c, "openai/gpt 4", true).unwrap();
        set_channel_token(&c, "telegram", " 123:abc-token ").unwrap();
        verify_key(&c, "openai", " sk-x ").unwrap();
        let seen = server.join().unwrap();
        let shape: Vec<(String, String, Value)> = seen
            .iter()
            .map(|(m, p, _, b)| (m.clone(), p.clone(), serde_json::from_str(b).unwrap()))
            .collect();
        assert_eq!(
            shape[0],
            (
                "POST".into(),
                "/api/providers/openai/toggle".into(),
                json!({"active": false})
            )
        );
        assert_eq!(
            shape[1],
            (
                "PUT".into(),
                "/api/providers/openai".into(),
                json!({"apiKey": "sk-nueva"})
            ),
            "la clave se recorta"
        );
        assert_eq!(
            shape[2],
            (
                "PUT".into(),
                "/api/providers/openai".into(),
                json!({"clearApiKey": true})
            )
        );
        assert_eq!(
            shape[3],
            (
                "PUT".into(),
                "/api/agents/a1".into(),
                json!({"name": "Pepe", "tone": "formal"})
            )
        );
        assert_eq!(
            shape[4],
            (
                "POST".into(),
                "/api/channels/telegram/toggle".into(),
                json!({"active": true})
            )
        );
        assert_eq!(
            shape[5],
            (
                "POST".into(),
                "/api/models/openai%2Fgpt%204/toggle".into(),
                json!({"active": true})
            ),
            "el id del modelo se codifica"
        );
        assert_eq!(
            shape[6],
            (
                "POST".into(),
                "/api/channels/telegram/reconnect".into(),
                json!({"config": {"botToken": "123:abc-token"}})
            ),
            "el token se recorta"
        );
        assert_eq!(
            shape[7],
            (
                "POST".into(),
                "/api/setup/verify-provider".into(),
                json!({"provider": "openai", "apiKey": "sk-x"})
            )
        );
        assert!(seen.iter().all(|r| r.2.as_deref() == Some("Bearer t")));
    }

    #[test]
    fn path_segments_are_escaped() {
        let (port, server) = fake_http(|_, _, _, _| (200, json!({})), 1);
        toggle_provider(&conn(port, None), "a/b c", true).unwrap();
        assert_eq!(
            server.join().unwrap()[0].1,
            "/api/providers/a%2Fb%20c/toggle"
        );
    }

    #[test]
    fn verify_reports_a_bad_key_that_comes_back_as_http_200() {
        let (port, server) = fake_http(
            |_, _, _, _| (200, json!({"success": false, "error": "API error: 401"})),
            1,
        );
        assert_eq!(
            verify_key(&conn(port, None), "openai", "mala"),
            Err(ApiError::Status(200, "API error: 401".into()))
        );
        server.join().unwrap();
    }

    #[test]
    fn errors_are_classified_and_readable() {
        let (port, server) = fake_http(
            |_, path, _, _| match path {
                "/api/providers" => (401, json!("Unauthorized")),
                "/api/agents" => (404, json!({"error": "Agent not found"})),
                _ => (500, json!({"message": "boom"})),
            },
            3,
        );
        let c = conn(port, None);
        assert_eq!(providers(&c), Err(ApiError::Unauthorized));
        assert_eq!(
            agents(&c),
            Err(ApiError::Status(404, "Agent not found".into()))
        );
        assert_eq!(channels(&c), Err(ApiError::Status(500, "boom".into())));
        server.join().unwrap();
        assert!(
            matches!(providers(&conn(1, None)), Err(ApiError::Net(_))),
            "sin servidor es un fallo de red"
        );
    }

    #[test]
    fn tools_and_skills_parse_with_flexible_flags() {
        let (port, server) = fake_http(
            |_, path, _, _| match path {
                "/api/tools" => (
                    200,
                    json!({"tools": [
                    {"id": "web_search", "name": "web_search", "description": "Busca", "category": "web", "active": 1, "enabled": true, "extra": {"x": 1}},
                    {"id": "x", "name": null, "category": null, "active": false}]}),
                ),
                _ => (
                    200,
                    json!({"skills": [{"id": "s1", "name": "Resumen", "description": null, "body": "# enorme", "active": 0}]}),
                ),
            },
            2,
        );
        let c = conn(port, None);
        let t = tools(&c).unwrap();
        assert!(t[0].active && t[0].enabled && t[0].category == "web");
        assert!(!t[1].active && t[1].name.is_empty() && t[1].category.is_empty());
        let s = skills(&c).unwrap();
        assert_eq!((s[0].name.as_str(), s[0].active), ("Resumen", false));
        server.join().unwrap();
    }

    #[test]
    fn tool_and_skill_toggles_post_the_new_state() {
        let (port, server) = fake_http(|_, _, _, _| (200, json!({"success": true})), 2);
        let c = conn(port, Some("t"));
        toggle_tool(&c, "web_search", false).unwrap();
        toggle_skill(&c, "s1", true).unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            (seen[0].0.as_str(), seen[0].1.as_str(), seen[0].3.as_str()),
            (
                "POST",
                "/api/tools/web_search/toggle",
                r#"{"active":false}"#
            )
        );
        assert_eq!(
            (seen[1].1.as_str(), seen[1].3.as_str()),
            ("/api/skills/s1/toggle", r#"{"active":true}"#)
        );
    }

    #[test]
    fn mcp_servers_accept_a_bare_list_or_a_wrapped_one() {
        let item = json!({"id": "cal", "name": "calendario", "status": "connected", "enabled": 1, "toolsCount": 5});
        let (port, server) = fake_http(
            {
                let item = item.clone();
                move |_, _, _, _| (200, json!([item.clone()]))
            },
            1,
        );
        let a = mcp_servers(&conn(port, None)).unwrap();
        assert_eq!(
            (
                a[0].name.as_str(),
                a[0].status.as_str(),
                a[0].enabled,
                a[0].tool_count()
            ),
            ("calendario", "connected", true, Some(5))
        );
        server.join().unwrap();
        let (port, _server) = fake_http(
            move |_, _, _, _| (200, json!({"servers": [item.clone(), {"id": "x"}]})),
            1,
        );
        let b = mcp_servers(&conn(port, None)).unwrap();
        assert_eq!(
            (b.len(), b[1].tool_count(), b[1].status.as_str()),
            (2, None, "")
        );
        // Con una lista en `tools` (y sin `toolsCount`) se cuentan sus elementos; ambos campos a la vez no chocan.
        let (port, server) = fake_http(
            |_, _, _, _| {
                (
                    200,
                    json!([{"id": "e", "tools": [{}, {}, {}]}, {"id": "f", "tools": [{}], "toolsCount": 9}]),
                )
            },
            1,
        );
        let c = mcp_servers(&conn(port, None)).unwrap();
        assert_eq!((c[0].tool_count(), c[1].tool_count()), (Some(3), Some(9)));
        server.join().unwrap();
    }
}
