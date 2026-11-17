//! Model selection is saved by the gateway in the canonical HiveDB catalog.
use crate::settings::api::{self, Conn};
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
pub struct Catalog {
    pub models: Vec<(String, String)>,
    pub selected: Option<String>,
    pub loaded: bool,
    pub error: Option<String>,
    pending: Option<Receiver<Result<Value, String>>>,
}
impl Catalog {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn refresh(&mut self, conn: &Conn) {
        self.request(conn, None);
    }
    pub fn select(&mut self, conn: &Conn, id: String) {
        self.request(conn, Some(id));
    }
    fn request(&mut self, conn: &Conn, id: Option<String>) {
        if self.busy() {
            return;
        }
        self.error = None;
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let conn = conn.clone();
        std::thread::spawn(move || {
            let result = api::request(
                &conn,
                if id.is_some() { "POST" } else { "GET" },
                "/api/realtime/models",
                id.map(|id| json!({"modelId":id})),
            )
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(_) => Err("No se pudo cargar el catálogo Live".into()),
        };
        self.pending = None;
        match result {
            Ok(value) => {
                if let Some(rows) = value["models"].as_array() {
                    self.models = rows
                        .iter()
                        .filter_map(|row| {
                            Some((
                                row["id"].as_str()?.into(),
                                row["name"].as_str().or_else(|| row["id"].as_str())?.into(),
                            ))
                        })
                        .collect();
                    self.selected = value["selectedId"].as_str().map(str::to_owned);
                    self.loaded = true;
                    self.error = None;
                } else {
                    self.error = Some("El gateway devolvió un catálogo Live inválido".into());
                }
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
    pub fn label(&self) -> String {
        if self.busy() {
            return "Guardando / cargando…".into();
        }
        self.selected
            .as_ref()
            .and_then(|id| self.models.iter().find(|row| &row.0 == id))
            .map(|row| row.1.clone())
            .unwrap_or_else(|| {
                if self.loaded {
                    "Sin modelos realtime habilitados".into()
                } else {
                    "Cargar modelos".into()
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hivelive_selection_waits_for_confirmation_and_retains_previous_choice_on_error() {
        let mut catalog = Catalog::default();
        let (tx, rx) = mpsc::channel();
        catalog.pending = Some(rx);
        tx.send(Ok(json!({"selectedId":"gemini-3.8-live","models":[{"id":"gemini-3.8-live","name":"Gemini 3.8 Live"}]}))).unwrap();
        assert!(catalog.poll());
        assert!(catalog.loaded && !catalog.busy());
        assert_eq!(catalog.label(), "Gemini 3.8 Live");
        let (tx, rx) = mpsc::channel();
        catalog.pending = Some(rx);
        tx.send(Err("Modelo deshabilitado".into())).unwrap();
        assert!(catalog.poll());
        assert_eq!(catalog.selected.as_deref(), Some("gemini-3.8-live"));
        assert_eq!(catalog.error.as_deref(), Some("Modelo deshabilitado"));
    }
}
