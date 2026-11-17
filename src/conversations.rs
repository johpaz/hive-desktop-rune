//! REST loads run off the UI thread; generations discard stale responses.
use crate::{
    chat,
    settings::api::{self, Conn},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Deserialize)]
pub struct Conversation {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "messageCount")]
    pub message_count: u64,
    #[serde(default, rename = "lastMessageAt")]
    pub updated: u64,
}
impl Conversation {
    pub fn title(&self) -> &str {
        self.title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("Conversación nueva")
    }
}
pub struct Loaded {
    pub items: Vec<Conversation>,
    pub thread: String,
    pub history: Value,
}
type Completion = (u64, Result<Loaded, String>);
#[derive(Default)]
pub struct Conversations {
    pub items: Vec<Conversation>,
    pub active: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub can_continue: bool,
    pub remote_active: bool,
    pub refresh_pending: bool,
    pub list_open: bool,
    generation: u64,
    slot: Arc<Mutex<Option<Completion>>>,
    last: Option<std::time::Instant>,
}
impl Conversations {
    pub fn request(&mut self, conn: Conn, target: Option<String>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.error = None;
        self.refresh_pending = false;
        self.last = Some(std::time::Instant::now());
        let slot = self.slot.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Loaded, String> {
                let thread = match target {
                    Some(id) => id,
                    None => api::request(&conn, "POST", "/api/conversations", Some(json!({})))
                        .map_err(|error| error.to_string())?["conversation"]["id"]
                        .as_str()
                        .ok_or("Respuesta sin conversación")?
                        .to_owned(),
                };
                let list = api::request(&conn, "GET", "/api/conversations?channel=webchat", None)
                    .map_err(|error| error.to_string())?;
                let items: Vec<Conversation> =
                    serde_json::from_value(list["conversations"].clone())
                        .map_err(|error| error.to_string())?;
                let history = api::request(
                    &conn,
                    "GET",
                    &format!(
                        "/api/chat/history?threadId={}&limit=40",
                        chat::urlenc(&thread)
                    ),
                    None,
                )
                .map_err(|error| error.to_string())?;
                Ok(Loaded {
                    items,
                    thread,
                    history,
                })
            })();
            let mut pending = slot.lock().unwrap();
            if pending
                .as_ref()
                .is_none_or(|(current, _)| *current <= generation)
            {
                *pending = Some((generation, result));
            }
        });
    }
    pub fn take(&mut self) -> Option<Result<Loaded, String>> {
        let (generation, result) = self.slot.lock().unwrap().take()?;
        if generation != self.generation {
            return None;
        }
        self.loading = false;
        match &result {
            Ok(loaded) => {
                self.items = loaded.items.clone();
                self.active = Some(loaded.thread.clone());
                self.remote_active = loaded.history["active"] == true;
                self.can_continue = loaded.history["canContinue"].as_bool().unwrap_or(true);
                self.error = loaded.history["continuationError"]
                    .as_str()
                    .map(str::to_owned);
            }
            Err(error) => self.error = Some(error.clone()),
        }
        Some(result)
    }
    pub fn due(&self) -> bool {
        !self.loading
            && (self.refresh_pending
                || (self.remote_active && self.last.is_none_or(|at| at.elapsed().as_secs() >= 2)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn webchat_discards_old_history_and_errors_after_a_new_selection() {
        let mut state = Conversations {
            generation: 2,
            loading: true,
            ..Default::default()
        };
        *state.slot.lock().unwrap() = Some((1, Err("old error".into())));
        assert!(state.take().is_none());
        assert!(state.loading);
        assert!(state.error.is_none());
        *state.slot.lock().unwrap() = Some((
            2,
            Ok(Loaded {
                thread: "new".into(),
                items: vec![],
                history: json!({"canContinue":true,"active":false}),
            }),
        ));
        assert!(state.take().unwrap().is_ok());
        assert_eq!(state.active.as_deref(), Some("new"));
        assert!(state.can_continue);
        *state.slot.lock().unwrap() = Some((1, Err("late error".into())));
        assert!(state.take().is_none());
        assert!(state.error.is_none());
        assert_eq!(state.active.as_deref(), Some("new"));
    }
    #[test]
    fn webchat_titles_use_the_original_new_conversation_fallback() {
        let item: Conversation =
            serde_json::from_value(json!({"id":"opaque","title":"  "})).unwrap();
        assert_eq!(item.title(), "Conversación nueva");
    }
}
