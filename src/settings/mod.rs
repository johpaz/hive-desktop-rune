//! Pantalla de ajustes: proveedores, agentes y canales.
//!
//! Las llamadas REST corren en hilos; el resultado vuelve por `data` y `sync` lo copia a la vista.
//! Nunca se muestra una clave guardada (solo la versión enmascarada que manda el servidor) y la
//! que se escribe se borra del campo en cuanto se envía.

pub mod api;
pub mod view;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
};

use api::{Agent, ApiError, Channel, Conn, McpServer, Provider, Skill, Tool};
use rune::TextEdit;
use serde_json::json;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Providers,
    Channels,
    Tools,
    Skills,
    Mcp,
    Ocr,
    Computer,
    Oracle,
    Ethics,
    Profile,
    Display,
    Updates,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub providers: Vec<Provider>,
    pub piper: Option<serde_json::Value>,
    pub agents: Vec<Agent>,
    pub channels: Vec<Channel>,
    pub tools: Vec<Tool>,
    pub skills: Vec<Skill>,
    pub mcp: Vec<McpServer>,
    /// Herramientas y skills se leen la primera vez que se abre su pestaña.
    pub extras_loaded: bool,
    /// Operaciones en vuelo.
    pub busy: u32,
    pub loaded: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// El gateway rechazó el token: hay que volver a iniciar sesión.
    pub unauthorized: bool,
    /// Vinculación de WhatsApp en curso (el QR se renueva solo mientras se espera el escaneo).
    pub whatsapp: Option<WhatsApp>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WhatsApp {
    pub channel: String,
    /// `starting`, `qr`, `connected`, `error` o `timeout`.
    pub status: String,
    pub qr: Option<String>,
}

/// QR → RGBA8: módulos de 6 px y margen de 4 módulos, negro sobre blanco para que lo lea cualquier teléfono.
pub fn qr_rgba(code: &str) -> Result<(u32, u32, Vec<u8>), String> {
    use qrcode::{Color, QrCode};
    const SCALE: usize = 6;
    const QUIET: usize = 4;
    let qr = QrCode::new(code.as_bytes()).map_err(|e| e.to_string())?;
    let n = qr.width();
    let side = (n + 2 * QUIET) * SCALE;
    let mut rgba = vec![255u8; side * side * 4];
    for (i, c) in qr.to_colors().into_iter().enumerate() {
        if c != Color::Dark {
            continue;
        }
        let (mx, my) = (i % n + QUIET, i / n + QUIET);
        for y in my * SCALE..(my + 1) * SCALE {
            for x in mx * SCALE..(mx + 1) * SCALE {
                let o = (y * side + x) * 4;
                rgba[o..o + 3].fill(0);
            }
        }
    }
    Ok((side as u32, side as u32, rgba))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Refresh,
    ToggleProvider {
        id: String,
        active: bool,
    },
    ToggleModel {
        id: String,
        active: bool,
    },
    DefaultModel {
        id: String,
        purpose: String,
    },
    SyncModels {
        id: String,
    },
    Piper {
        operation: String,
        payload: serde_json::Value,
    },
    SetKey {
        id: String,
    },
    ClearKey {
        id: String,
    },
    VerifyKey {
        id: String,
    },
    ToggleChannel {
        id: String,
        active: bool,
    },
    LinkWhatsApp {
        id: String,
    },
    CreateChannel,
    SetChannelToken {
        id: String,
    },
    /// Guarda el formulario completo del panel de Agentes (nombre, descripción, prompt, tono, iteraciones, activo).
    SaveAgentForm,
    /// Crea un agente con el formulario «Desplegar nodo».
    CreateAgent,
    /// Pasa a especialista a los agentes marcados como coordinador que sobran (solo puede haber uno).
    DemoteExtraCoordinators,
    RefreshExtras,
    ToggleTool {
        id: String,
        active: bool,
    },
    ToggleSkill {
        id: String,
        active: bool,
    },
}

/// El formulario «Desplegar nodo»: lo que pide la web al crear un agente.
#[derive(Default)]
pub struct NewAgent {
    pub name: TextEdit,
    pub desc: TextEdit,
    pub prompt: TextEdit,
    pub workspace: TextEdit,
    /// Posición en la lista de proveedores de lenguaje con clave, y en la de sus modelos activos.
    pub provider: usize,
    pub model: usize,
    /// Lista desplegada: 0 = ninguna, 1 = proveedor, 2 = modelo.
    pub open: u8,
}

impl NewAgent {
    /// El cuerpo del `POST /api/agents`; el error es lo que se le dice a la persona.
    pub fn body(&self, providers: &[Provider]) -> Result<serde_json::Value, &'static str> {
        let name = self.name.text.trim();
        if name.is_empty() {
            return Err("El agente necesita un nombre");
        }
        let mut body = json!({"name": name, "description": self.desc.text.trim()});
        let usable = Self::usable(providers);
        if let Some(p) = usable.get(self.provider) {
            body["providerId"] = json!(p.id);
            let models: Vec<_> = p.models.iter().filter(|m| m.for_agent()).collect();
            if let Some(m) = models.get(self.model) {
                body["modelId"] = json!(m.id);
            }
        } // sin elección, el servidor usa el modelo predeterminado
        let ws = self.workspace.text.trim();
        if !ws.is_empty() {
            body["workspace"] = json!(ws);
        }
        Ok(body)
    }

    /// Proveedores de lenguaje activos y con clave: los únicos con los que un agente puede funcionar.
    pub fn usable(providers: &[Provider]) -> Vec<&Provider> {
        providers
            .iter()
            .filter(|p| {
                (p.category.is_empty() || p.category == "llm")
                    && p.enabled
                    && p.active
                    && p.has_api_key
            })
            .collect()
    }
}

/// El formulario de configuración de un agente (pantalla Agentes).
pub struct AgentForm {
    pub id: String,
    pub name: TextEdit,
    pub desc: TextEdit,
    pub prompt: TextEdit,
    pub tone: TextEdit,
    pub iters: TextEdit,
    pub enabled: bool,
}

impl AgentForm {
    pub fn from(a: &Agent) -> AgentForm {
        AgentForm {
            id: a.id.clone(),
            name: TextEdit::new(a.name.clone()),
            desc: TextEdit::new(a.description.clone()),
            prompt: TextEdit::new(a.system_prompt.clone()),
            tone: TextEdit::new(a.tone.clone()),
            iters: TextEdit::new(a.max_iterations.map(|n| n.to_string()).unwrap_or_default()),
            enabled: a.enabled,
        }
    }

    /// El cuerpo del `PUT /api/agents/:id`; el error es lo que se le dice a la persona.
    pub fn patch(&self) -> Result<serde_json::Value, &'static str> {
        let name = self.name.text.trim();
        if name.is_empty() {
            return Err("El agente necesita un nombre");
        }
        let iters = self.iters.text.trim();
        let mut patch = json!({
            "name": name,
            "description": self.desc.text.trim(),
            "systemPrompt": self.prompt.text,
            "tone": self.tone.text.trim(),
            "enabled": self.enabled,
        });
        if !iters.is_empty() {
            match iters.parse::<u64>() {
                Ok(n) if (1..=200).contains(&n) => patch["maxIterations"] = json!(n),
                _ => return Err("Las iteraciones deben ser un número entre 1 y 200"),
            }
        }
        Ok(patch)
    }
}

#[derive(Default)]
pub struct Settings {
    /// Agente en edición en la pantalla de Agentes.
    pub agent_form: Option<AgentForm>,
    /// Formulario «Desplegar nodo», si está abierto.
    pub new_agent: Option<NewAgent>,
    pub data: Arc<Mutex<Data>>,
    /// Copia de `data` que pinta la vista.
    pub shown: Data,
    pub conn: Option<Conn>,
    pub tab: Tab,
    /// Campo de clave por proveedor (siempre secreto).
    pub keys: HashMap<String, TextEdit>,
    /// Proveedor cuya clave está a punto de borrarse, a la espera de confirmación.
    pub confirm_clear: Option<String>,
    /// Proveedor con la lista de modelos desplegada.
    pub models_open: Option<String>,
    /// Proveedor o canal con su panel de configuración abierto (como el «CONFIG» de la web).
    pub config_open: Option<String>,
    /// Pestaña de proveedores: 0 = lenguaje, 1 = imagen, 2 = voz.
    pub prov_cat: u8,
    /// Filtro de canales: 0 = todos, 1 = activos, 2 = inactivos.
    pub chan_filter: u8,
    /// El formulario «Nuevo canal» desplegado.
    pub add_open: bool,
    /// Categoría elegida en las galerías de herramientas y de skills.
    pub tool_cat: Option<String>,
    pub skill_cat: Option<String>,
    /// Campo del token del bot por canal (siempre secreto).
    pub channel_tokens: HashMap<String, TextEdit>,
    /// Formulario «Añadir canal»: tipo elegido y sus campos (token y, según el tipo, `applicationId` o `signingSecret`).
    pub add_kind: Option<String>,
    pub add_token: TextEdit,
    pub add_extra: TextEdit,
    /// Imagen del QR de WhatsApp y el texto con que se dibujó.
    pub qr_pixels: rune::Pixels,
    qr_drawn: Option<String>,
    /// Pausa entre consultas del estado de WhatsApp (2 s; las pruebas la acortan).
    pub whatsapp_poll_ms: Option<u64>,
    /// Búsqueda en herramientas y skills.
    pub filter: TextEdit,
    pub piper_text: TextEdit,
}

type Snapshot = (Vec<Provider>, Vec<Agent>, Vec<Channel>);
type Extras = (Vec<Tool>, Vec<Skill>, Vec<McpServer>);

fn load_extras(conn: &Conn) -> Result<Extras, ApiError> {
    Ok((
        api::tools(conn)?,
        api::skills(conn)?,
        api::mcp_servers(conn)?,
    ))
}

fn load(conn: &Conn) -> Result<Snapshot, ApiError> {
    Ok((
        api::providers(conn)?,
        api::agents(conn)?,
        api::channels(conn)?,
    ))
}

impl Settings {
    /// Copia lo que dejaron los hilos y prepara los campos nuevos. Devuelve si cambió algo visible.
    pub fn sync(&mut self) -> bool {
        let now = self.data.lock().unwrap().clone();
        if now == self.shown {
            return false;
        }
        // El QR se redibuja solo cuando cambia su texto.
        let wanted = now.whatsapp.as_ref().and_then(|w| w.qr.clone());
        if wanted != self.qr_drawn {
            if let Some(Ok((w, h, rgba))) = wanted.as_deref().map(qr_rgba) {
                self.qr_pixels.set(w, h, rgba);
            }
            self.qr_drawn = wanted;
        }
        self.shown = now;
        true
    }

    fn note(&self, msg: &str) {
        let mut d = self.data.lock().unwrap();
        d.notice = Some(msg.to_string());
        d.error = None;
    }

    /// Reconecta WhatsApp y consulta su estado hasta que se vincule, falle o pasen ~3 min.
    fn link_whatsapp(&mut self, conn: Conn, id: String) {
        let data = self.data.clone();
        let pause = std::time::Duration::from_millis(self.whatsapp_poll_ms.unwrap_or(2000));
        {
            let mut d = data.lock().unwrap();
            if d.whatsapp
                .as_ref()
                .is_some_and(|w| matches!(w.status.as_str(), "starting" | "qr"))
            {
                return; // ya hay una vinculación en curso
            }
            d.whatsapp = Some(WhatsApp {
                channel: id.clone(),
                status: "starting".into(),
                qr: None,
            });
        }
        thread::spawn(move || {
            let set = |status: &str, qr: Option<String>| {
                data.lock().unwrap().whatsapp = Some(WhatsApp {
                    channel: id.clone(),
                    status: status.into(),
                    qr,
                });
            };
            if let Err(e) = api::reconnect_channel(&conn, &id) {
                let mut d = data.lock().unwrap();
                d.unauthorized |= e == ApiError::Unauthorized;
                d.error = Some(e.to_string());
                d.whatsapp = None;
                return;
            }
            for _ in 0..90 {
                thread::sleep(pause);
                match api::channel_status(&conn, "whatsapp", &id) {
                    Ok(st) => match (st.status.as_str(), st.qr_code) {
                        ("qr", Some(code)) => set("qr", Some(code)),
                        ("connected", _) => {
                            set("connected", None);
                            if let Ok((p, a, c)) = load(&conn) {
                                let mut d = data.lock().unwrap();
                                (d.providers, d.agents, d.channels) = (p, a, c);
                            }
                            return;
                        }
                        ("error", _) => return set("error", None),
                        _ => {} // starting o disconnected: Baileys renueva el QR solo
                    },
                    Err(ApiError::Unauthorized) => {
                        data.lock().unwrap().unauthorized = true;
                        return set("error", None);
                    }
                    Err(_) => {} // fallo de red puntual: se sigue consultando
                }
            }
            set("timeout", None);
        });
    }

    pub fn key_edit(&self, id: &str) -> Option<&TextEdit> {
        self.keys.get(id)
    }

    /// Lanza una acción en un hilo y, si cambia algo en el servidor, vuelve a leer las listas.
    pub fn dispatch(&mut self, action: Action) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        self.confirm_clear = None;

        // Lo que se escribe en la interfaz se toma aquí, en el hilo de la UI.
        let mut key = String::new();
        let mut patch = None;
        let mut new_prompt = String::new();
        let kind_for_create = self.add_kind.clone();
        let form_target = self.agent_form.as_ref().map(|f| f.id.clone());
        let demote: Vec<String> = if matches!(action, Action::DemoteExtraCoordinators) {
            api::extra_coordinators(&self.shown.agents)
                .iter()
                .map(|a| a.id.clone())
                .collect()
        } else {
            Vec::new()
        };
        match &action {
            Action::SetKey { id } | Action::VerifyKey { id } => {
                key = self
                    .keys
                    .get(id)
                    .map(|e| e.text.trim().to_string())
                    .unwrap_or_default();
                if key.is_empty() {
                    return self.note("Escribe la clave primero");
                }
                if matches!(action, Action::SetKey { .. }) {
                    if let Some(e) = self.keys.get_mut(id) {
                        e.clear(); // la clave no se queda en pantalla
                    }
                }
            }
            Action::CreateChannel => {
                let Some(kind) = self.add_kind.clone() else {
                    return self.note("Elige primero el tipo de canal");
                };
                let (token, extra) = (
                    self.add_token.text.trim().to_string(),
                    self.add_extra.text.trim().to_string(),
                );
                let config = match kind.as_str() {
                    "whatsapp" => json!({}),
                    "telegram" if token.len() > 10 => json!({"botToken": token}),
                    "discord" if token.len() > 10 && !extra.is_empty() => json!({"botToken": token, "applicationId": extra}),
                    "slack" if token.len() > 10 && extra.len() > 10 => json!({"botToken": token, "signingSecret": extra}),
                    _ => return self.note("Completa los datos del canal (token del bot y, en Discord y Slack, el segundo campo)"),
                };
                patch = Some(config);
                self.add_token.clear(); // los secretos no se quedan en pantalla
                self.add_extra.clear();
                self.add_kind = None;
            }
            Action::SetChannelToken { id } => {
                key = self
                    .channel_tokens
                    .get(id)
                    .map(|e| e.text.trim().to_string())
                    .unwrap_or_default();
                if key.len() <= 10 {
                    return self.note("Escribe el token completo del bot primero");
                }
                if let Some(e) = self.channel_tokens.get_mut(id) {
                    e.clear(); // el token no se queda en pantalla
                }
            }
            Action::CreateAgent => {
                let Some(form) = &self.new_agent else { return };
                match form.body(&self.shown.providers) {
                    Ok(b) => patch = Some(b),
                    Err(e) => return self.note(e),
                }
                new_prompt = self
                    .new_agent
                    .as_ref()
                    .map(|f| f.prompt.text.clone())
                    .unwrap_or_default();
                self.new_agent = None; // el formulario se cierra al enviarlo
            }
            Action::SaveAgentForm => {
                let Some(form) = &self.agent_form else { return };
                match form.patch() {
                    Ok(p) => patch = Some(p),
                    Err(e) => return self.note(e),
                }
            }
            _ => {}
        }

        if let Action::LinkWhatsApp { id } = &action {
            return self.link_whatsapp(conn, id.clone());
        }

        if let Action::Piper { operation, payload } = &action {
            let operation = operation.clone();
            let mut payload = payload.clone();
            if operation == "speak" {
                payload = json!({"text":if self.piper_text.text.trim().is_empty(){"Hola, soy BIA. Esta es la voz local de Hive."}else{self.piper_text.text.trim()}});
            }
            let data = self.data.clone();
            if data.lock().unwrap().busy > 0 {
                return;
            }
            data.lock().unwrap().busy += 1;
            let output = crate::audio_prefs::load().output;
            thread::spawn(move || {
                let method = if operation == "status" { "GET" } else { "POST" };
                let result = api::request(
                    &conn,
                    method,
                    &format!("/api/tts-local/{operation}"),
                    if method == "GET" { None } else { Some(payload) },
                );
                let result = result.and_then(|value| {
                    if operation == "speak" {
                        play_piper(&value, output.as_deref()).map_err(ApiError::Net)?;
                    }
                    Ok(value)
                });
                let mut d = data.lock().unwrap();
                d.busy -= 1;
                match result {
                    Ok(v) => {
                        if operation != "speak" {
                            d.piper = Some(v);
                        }
                        d.error = None;
                        d.notice = Some(
                            if operation == "speak" {
                                "Prueba de voz terminada"
                            } else {
                                "Estado de Piper actualizado"
                            }
                            .into(),
                        );
                    }
                    Err(e) => {
                        d.unauthorized |= e == ApiError::Unauthorized;
                        d.error = Some(e.to_string());
                        d.notice = None;
                    }
                }
            });
            return;
        }
        let data = self.data.clone();
        data.lock().unwrap().busy += 1;
        thread::spawn(move || {
            let extras_action = matches!(
                action,
                Action::RefreshExtras | Action::ToggleTool { .. } | Action::ToggleSkill { .. }
            );
            let mutates = !matches!(
                action,
                Action::Refresh | Action::RefreshExtras | Action::VerifyKey { .. }
            );
            let result: Result<&'static str, ApiError> = match &action {
                Action::Piper { .. } => unreachable!(),
                Action::Refresh | Action::RefreshExtras => Ok(""),
                Action::ToggleTool { id, active } => {
                    api::toggle_tool(&conn, id, *active).map(|_| "Herramienta actualizada")
                }
                Action::ToggleSkill { id, active } => {
                    api::toggle_skill(&conn, id, *active).map(|_| "Skill actualizada")
                }
                Action::DefaultModel { id, purpose } => api::request(
                    &conn,
                    "POST",
                    &format!("/api/models/{}/default", crate::chat::urlenc(id)),
                    Some(json!({"purpose":purpose})),
                )
                .map(|_| "Modelo predeterminado guardado"),
                Action::SyncModels { id } => api::request(
                    &conn,
                    "POST",
                    &format!("/api/providers/{}/sync-models", crate::chat::urlenc(id)),
                    Some(json!({})),
                )
                .map(|_| "Catálogo actualizado; los nuevos modelos quedan desactivados"),
                Action::ToggleModel { id, active } => {
                    api::toggle_model(&conn, id, *active).map(|_| "Modelo actualizado")
                }
                Action::ToggleProvider { id, active } => {
                    api::toggle_provider(&conn, id, *active).map(|_| "Proveedor actualizado")
                }
                Action::SetKey { id } => {
                    api::set_api_key(&conn, id, &key).map(|_| "Clave guardada")
                }
                Action::ClearKey { id } => api::clear_api_key(&conn, id).map(|_| "Clave eliminada"),
                Action::VerifyKey { id } => {
                    api::verify_key(&conn, id, &key).map(|_| "La clave es válida")
                }
                Action::SetChannelToken { id } => api::set_channel_token(&conn, id, &key)
                    .map(|_| "Token guardado; el canal se está reconectando"),
                Action::LinkWhatsApp { .. } => Ok(""),
                Action::CreateChannel => {
                    let kind = kind_for_create.clone().unwrap_or_default();
                    api::create_channel(&conn, &kind, patch.clone().unwrap_or_default())
                        .map(|_| "Canal creado")
                }
                Action::ToggleChannel { id, active } => {
                    api::toggle_channel(&conn, id, *active).map(|_| "Canal actualizado")
                }
                Action::CreateAgent => api::create_agent(&conn, patch.clone().unwrap_or_default())
                    .and_then(|id| {
                        // El servidor da `coordinator` a todo agente que crea por REST y solo puede haber uno: se corrige en
                        // el acto a especialista, y de paso se guarda el prompt (el servidor no lo acepta al crear).
                        let mut fix = json!({"role": "worker"});
                        if !new_prompt.trim().is_empty() {
                            fix["systemPrompt"] = json!(new_prompt);
                        }
                        api::update_agent(&conn, &id, fix)
                    })
                    .map(|_| "Agente creado como especialista"),
                Action::DemoteExtraCoordinators => demote
                    .iter()
                    .try_for_each(|id| api::update_agent(&conn, id, json!({"role": "worker"})))
                    .map(|_| "Coordinadores sobrantes pasados a especialista"),
                Action::SaveAgentForm => api::update_agent(
                    &conn,
                    form_target.as_deref().unwrap_or(""),
                    patch.clone().unwrap_or_default(),
                )
                .map(|_| "Agente guardado"),
            };
            // Tras un cambio, o al refrescar, se vuelven a leer las listas.
            let fresh = (!extras_action && (mutates || matches!(action, Action::Refresh)))
                .then(|| load(&conn));
            let fresh_extras = (extras_action
                && (mutates || matches!(action, Action::RefreshExtras)))
            .then(|| load_extras(&conn));
            let mut d = data.lock().unwrap();
            d.busy -= 1;
            d.loaded = true;
            match result {
                Ok(msg) => {
                    d.notice = (!msg.is_empty()).then(|| msg.to_string());
                    d.error = None;
                }
                Err(e) => {
                    d.unauthorized |= e == ApiError::Unauthorized;
                    d.error = Some(e.to_string());
                    d.notice = None;
                }
            }
            match fresh_extras {
                Some(Ok((t, sk, m))) => {
                    (d.tools, d.skills, d.mcp) = (t, sk, m);
                    d.extras_loaded = true;
                }
                Some(Err(e)) if d.error.is_none() => {
                    d.unauthorized |= e == ApiError::Unauthorized;
                    d.error = Some(e.to_string());
                }
                _ => {}
            }
            match fresh {
                Some(Ok((p, a, c))) => {
                    (d.providers, d.agents, d.channels) = (p, a, c);
                }
                Some(Err(e)) if d.error.is_none() => {
                    d.unauthorized |= e == ApiError::Unauthorized;
                    d.error = Some(e.to_string());
                }
                _ => {}
            }
        });
    }
}

fn play_piper(value: &serde_json::Value, output: Option<&str>) -> Result<(), String> {
    use base64::Engine;
    let audio = value["audio"].as_str().ok_or("Piper no devolvió audio")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio)
        .map_err(|_| "Audio local inválido")?;
    let (samples, rate) = crate::a2ui::audio::decode(bytes, 60)?;
    let pcm = crate::a2ui::audio::to_pcm16_24k(&samples, rate);
    let player = hive_media::Player::start(output, None).map_err(|e| e.message)?;
    player.enqueue(&pcm);
    std::thread::sleep(std::time::Duration::from_millis(
        pcm.len() as u64 * 1000 / 48000 + 150,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn the_qr_is_black_on_white_with_a_quiet_zone_and_whole_modules() {
        let (w, h, rgba) = super::qr_rgba("2@abc123,def456,ghi789==").unwrap();
        assert_eq!(w, h);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        assert_eq!(w % 6, 0, "módulos de 6 px");
        let px = |x: u32, y: u32| rgba[((y * w + x) * 4) as usize];
        assert_eq!(px(0, 0), 255, "el margen es blanco");
        assert_eq!(px(w - 1, w - 1), 255);
        assert_eq!(px(4 * 6, 4 * 6), 0, "primer módulo del patrón de búsqueda");
        assert!(
            rgba.chunks(4)
                .all(|p| (p[0] == 0 || p[0] == 255) && p[3] == 255),
            "solo negro y blanco opacos"
        );
    }

    #[test]
    fn an_oversized_code_is_an_error_not_a_panic() {
        assert!(super::qr_rgba(&"x".repeat(9000)).is_err());
    }

    #[test]
    fn the_agent_form_builds_the_patch_and_rejects_bad_input() {
        let a: Agent = serde_json::from_value(json!({"id": "a1", "name": "Código", "description": "d", "systemPrompt": "# ROL", "tone": "professional", "maxIterations": 20, "enabled": true})).unwrap();
        let mut f = AgentForm::from(&a);
        assert_eq!(
            f.patch().unwrap(),
            json!({"name": "Código", "description": "d", "systemPrompt": "# ROL", "tone": "professional", "enabled": true, "maxIterations": 20})
        );
        f.name = TextEdit::new("   ");
        assert_eq!(f.patch(), Err("El agente necesita un nombre"));
        f.name = TextEdit::new(" Nuevo nombre ");
        f.iters = TextEdit::new("0");
        assert_eq!(
            f.patch(),
            Err("Las iteraciones deben ser un número entre 1 y 200")
        );
        f.iters = TextEdit::new("201");
        assert!(f.patch().is_err());
        f.iters = TextEdit::new("abc");
        assert!(f.patch().is_err());
        f.iters = TextEdit::new("");
        let p = f.patch().unwrap();
        assert_eq!(p["name"], "Nuevo nombre", "el nombre se recorta");
        assert!(p.get("maxIterations").is_none(), "vacío = no se toca");
        f.enabled = false;
        assert_eq!(f.patch().unwrap()["enabled"], false);
    }
}
