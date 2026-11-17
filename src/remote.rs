//! Lectura y guardado de un documento de ajustes del gateway desde un hilo, para las pestañas que lo edita
//! una persona (Control del equipo, Oráculo). Quien lo usa guarda el JSON tal como llegó y construye su formulario.
use crate::settings::api::{request, ApiError, Conn};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub loaded: bool,
    pub value: Value,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub unauthorized: bool,
    /// Sube con cada respuesta buena: es la señal para volver a rellenar el formulario.
    pub generation: u64,
}

#[derive(Default)]
pub struct Remote {
    data: Arc<Mutex<Data>>,
    pub shown: Data,
    busy: Arc<AtomicBool>,
    filled: u64,
}

impl Remote {
    /// Copia lo que dejaron los hilos. Devuelve `(algo cambió, hay respuesta nueva que volcar al formulario)`.
    pub fn sync(&mut self) -> (bool, bool) {
        let now = self.data.lock().unwrap().clone();
        if now == self.shown {
            return (false, false);
        }
        let fresh = now.generation != self.filled;
        self.filled = now.generation;
        self.shown = now;
        (true, fresh)
    }

    pub fn working(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Una operación a la vez; el resultado reemplaza el documento mostrado.
    pub fn call(
        &self,
        conn: &Conn,
        method: &'static str,
        path: &'static str,
        body: Option<Value>,
        notice: Option<&'static str>,
    ) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut d = self.data.lock().unwrap();
            d.error = None;
            d.notice = None;
        }
        let (conn, data, busy) = (conn.clone(), self.data.clone(), self.busy.clone());
        std::thread::spawn(move || {
            let r = request(&conn, method, path, body);
            let mut d = data.lock().unwrap();
            match r {
                Ok(v) => {
                    d.value = v;
                    d.loaded = true;
                    d.error = None;
                    d.notice = notice.map(str::to_string);
                    d.generation += 1;
                }
                Err(e) => {
                    d.unauthorized |= e == ApiError::Unauthorized;
                    d.loaded = true;
                    d.error = Some(e.to_string());
                }
            }
            drop(d);
            busy.store(false, Ordering::SeqCst);
        });
    }
}
