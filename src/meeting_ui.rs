//! Pantalla «Reuniones»: título, si se incluye el audio del equipo, transcripción en vivo y los informes.
//!
//! Los hilos (abrir la sesión, listar, generar el informe) dejan su resultado en `data`; `poll` lo copia
//! a la vista con `sync`. La sesión viva y sus capturas de audio están en `ACTIVE`.

use crate::ui::text;
use crate::ui::Tap;
use std::sync::{Arc, Mutex};

use hive_media::Capture;
use rune::tok::type_;
use rune::{col, row, El, ScrollSpec, TextEdit};

use crate::meeting::{self, MEvent, Meeting, Session};
use crate::settings::api::Conn;
use crate::settings::view::field;
use crate::State;

/// La reunión en marcha: la sesión, y las capturas que le dan audio (se sueltan al cerrar).
struct Active {
    meeting: Meeting,
    _mic: Capture,
    _system: Option<Capture>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub sessions: Vec<Session>,
    pub segments: Vec<(u64, String)>,
    pub live: bool,
    /// Abriendo la sesión (creándola en el servidor y abriendo el audio).
    pub starting: bool,
    /// La reunión se está cerrando: faltan las últimas transcripciones.
    pub stopping: bool,
    pub notice: Option<String>,
    pub error: Option<String>,
    /// Sesión cuyo informe se está generando.
    pub generating: Option<String>,
    pub loaded: bool,
}

#[derive(Default)]
pub struct Ui {
    pub data: Arc<Mutex<Data>>,
    pub shown: Data,
    pub title: TextEdit,
    /// «Incluir el audio del equipo» (lo que suena por los altavoces: la otra parte de una videollamada).
    pub include_system: bool,
}

impl Ui {
    /// Copia lo que dejaron los hilos; devuelve si cambió algo visible.
    pub fn sync(&mut self) -> bool {
        let now = self.data.lock().unwrap().clone();
        if now == self.shown {
            return false;
        }
        self.shown = now;
        true
    }

    /// Eventos de la reunión viva: transcripciones, cierre y errores.
    pub fn pump(&mut self, conn: Option<&Conn>) -> bool {
        let events = ACTIVE
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.meeting.drain())
            .unwrap_or_default();
        if events.is_empty() {
            return false;
        }
        let closed = apply_events(&mut self.data.lock().unwrap(), events);
        if closed {
            // Se sueltan las capturas fuera del candado de los datos.
            ACTIVE.lock().unwrap().take();
            if let Some(c) = conn {
                refresh(self.data.clone(), c.clone());
            }
        }
        true
    }

    pub fn open(&mut self, conn: Option<&Conn>) {
        if !self.shown.loaded {
            if let Some(c) = conn {
                refresh(self.data.clone(), c.clone());
            }
        }
    }
}

/// Aplica los eventos de la reunión a los datos de la vista; devuelve si el socket se cerró.
fn apply_events(d: &mut Data, events: Vec<MEvent>) -> bool {
    let mut closed = false;
    for e in events {
        match e {
            MEvent::Segment { seq, text, .. } if !text.is_empty() => d.segments.push((seq, text)),
            MEvent::Segment { .. } => {}
            MEvent::Stopped { segments } => {
                d.notice = Some(format!(
                    "Reunión terminada: {segments} fragmentos transcritos"
                ))
            }
            MEvent::Error(e) => d.error = Some(e),
            MEvent::Closed => closed = true,
        }
    }
    if closed {
        d.live = false;
        d.stopping = false;
    }
    closed
}

/// Vuelve a leer la lista de reuniones.
fn refresh(data: Arc<Mutex<Data>>, conn: Conn) {
    std::thread::spawn(move || {
        let r = meeting::list(&conn);
        let mut d = data.lock().unwrap();
        d.loaded = true;
        match r {
            Ok(s) => d.sessions = s,
            Err(e) => d.error = Some(e.to_string()),
        }
    });
}

/// El audio del equipo que se va a transcribir: el monitor de la salida por defecto, o el primero.
pub fn pick_system_source() -> Option<String> {
    let list = hive_media::list_system_sources().ok()?;
    list.iter()
        .find(|d| d.is_default)
        .or_else(|| list.first())
        .map(|d| d.id.clone())
}

/// Crea la sesión en el servidor, abre el socket y el audio. Corre en un hilo.
pub fn start(s: &mut State) {
    let title = s.meeting.title.text.trim().to_string();
    let (Some(conn), Some(crate::gateway::Status::Ready { port, .. })) =
        (s.settings.conn.clone(), s.shown.status.clone())
    else {
        s.meeting.data.lock().unwrap().error = Some("El gateway aún no está listo".into());
        return;
    };
    let include_system = s.meeting.include_system;
    let data = s.meeting.data.clone();
    {
        let mut d = data.lock().unwrap();
        if d.live || d.starting {
            return;
        }
        *d = Data {
            starting: true,
            sessions: std::mem::take(&mut d.sessions),
            loaded: d.loaded,
            ..Default::default()
        };
    }
    std::thread::spawn(move || {
        let result = open_session(
            &conn,
            port,
            if title.is_empty() {
                "Reunión sin título"
            } else {
                &title
            },
            include_system,
        );
        let mut d = data.lock().unwrap();
        d.starting = false;
        match result {
            Ok(active) => {
                *ACTIVE.lock().unwrap() = Some(active);
                d.live = true;
            }
            Err(e) => d.error = Some(e),
        }
    });
}

fn open_session(
    conn: &Conn,
    port: u16,
    title: &str,
    include_system: bool,
) -> Result<Active, String> {
    let id = meeting::create(conn, title).map_err(|e| e.to_string())?;
    let fail = |e: String| {
        let _ = meeting::stop(conn, &id); // no deja una sesión «activa» huérfana en el servidor
        e
    };
    let m = Meeting::start_authenticated(port, &id, meeting::CHUNK_SECS, conn.token.as_deref())
        .map_err(&fail)?;
    let mic = Capture::start(None, None, m.mic_sink())
        .map_err(|e| fail(format!("No se pudo abrir el micrófono: {}", e.message)))?;
    let system = if include_system {
        let src = pick_system_source().ok_or_else(|| {
            fail(
                "No encontré audio del equipo para capturar (solo Linux con PipeWire por ahora)"
                    .into(),
            )
        })?;
        Some(
            Capture::start(Some(&src), None, m.system_sink()).map_err(|e| {
                fail(format!(
                    "No se pudo capturar el audio del equipo: {}",
                    e.message
                ))
            })?,
        )
    } else {
        None
    };
    Ok(Active {
        meeting: m,
        _mic: mic,
        _system: system,
    })
}

/// Termina la reunión: manda lo que quede y cierra la sesión; las últimas transcripciones llegan después.
pub fn stop(s: &mut State) {
    let mut d = s.meeting.data.lock().unwrap();
    if !d.live || d.stopping {
        return;
    }
    d.stopping = true;
    drop(d);
    if let Some(a) = ACTIVE.lock().unwrap().as_ref() {
        a.meeting.finish();
    }
}

/// Apaga todo de golpe (cerrar sesión, salir): sin esperar transcripciones.
pub fn abort() {
    ACTIVE.lock().unwrap().take();
}

fn make_report(s: &mut State, id: String) {
    let Some(conn) = s.settings.conn.clone() else {
        return;
    };
    let data = s.meeting.data.clone();
    {
        let mut d = data.lock().unwrap();
        if d.generating.is_some() {
            return;
        }
        d.generating = Some(id.clone());
        d.error = None;
    }
    std::thread::spawn(move || {
        let r = meeting::report(&conn, &id);
        let list = meeting::list(&conn);
        let mut d = data.lock().unwrap();
        d.generating = None;
        match r {
            Ok(path) => d.notice = Some(format!("Informe listo: {path}")),
            Err(e) => d.error = Some(e.to_string()),
        }
        if let Ok(l) = list {
            d.sessions = l;
        }
    });
}

pub fn view(s: &State) -> El {
    use crate::ui::{icons, theme as th};
    use rune::tok::TextStyle;
    let m = &s.meeting;
    let d = &m.shown;
    let busy = d.live || d.starting;
    let red = th::red();
    let amber = th::amber();

    let header = row()
        .w_full()
        .px(18.0)
        .py(16.0)
        .gap(12.0)
        .center_y()
        .fill(th::glass())
        .border(1.0, th::border_subtle())
        .child(
            row()
                .size(36.0, 36.0)
                .radius(9.0)
                .center()
                .fill(amber.alpha(0.12))
                .border(1.0, amber.alpha(0.3))
                .child(rune::icon(&icons::RADIO, 17.0).color(amber)),
        )
        .child(
            col()
                .gap(2.0)
                .child(
                    text("Transcripción de Reuniones")
                        .font(TextStyle::new(14.0, 700.0))
                        .color(th::foreground()),
                )
                .child(
                    text("Captura · Transcribe · Informa en tiempo real")
                        .font(TextStyle::new(11.0, 400.0))
                        .color(th::text_muted()),
                ),
        );

    // Nombre y botón de iniciar/detener, como la fila superior de la web.
    let action = if d.live {
        row()
            .key("meet/stop")
            .gap(8.0)
            .center_y()
            .px(18.0)
            .py(11.0)
            .radius(10.0)
            .fill(red)
            .child(rune::icon(&icons::MIC_OFF, 15.0).color(rune::Color::WHITE))
            .child(
                text(if d.stopping { "Cerrando…" } else { "Detener" })
                    .font(TextStyle::new(13.0, 700.0))
                    .color(rune::Color::WHITE),
            )
            .tap(|s: &mut State| stop(s))
    } else {
        row()
            .key("meet/start")
            .gap(8.0)
            .center_y()
            .px(18.0)
            .py(11.0)
            .radius(10.0)
            .fill(amber.alpha(if busy { 0.5 } else { 1.0 }))
            .shadow(0.0, 14.0, amber.alpha(0.25))
            .child(rune::icon(&icons::MIC, 15.0).color(th::on_amber()))
            .child(
                text(if d.starting { "Abriendo…" } else { "Iniciar" })
                    .font(TextStyle::new(13.0, 700.0))
                    .color(th::on_amber()),
            )
            .tap(|s: &mut State| start(s))
    };
    let system = row()
        .key("meet/system")
        .gap(8.0)
        .center_y()
        .child(
            rune::el()
                .size(18.0, 18.0)
                .radius(4.0)
                .fill(if m.include_system {
                    th::blue()
                } else {
                    th::muted()
                })
                .center()
                .child(
                    text(if m.include_system { "✓" } else { "" })
                        .font(type_::LABEL)
                        .color(rune::Color::WHITE),
                ),
        )
        .child(
            text("Incluir el audio del equipo (la otra parte de una videollamada)")
                .font(type_::LABEL)
                .color(th::muted_foreground()),
        )
        .tap(|s: &mut State| {
            if !s.meeting.shown.live && !s.meeting.shown.starting {
                s.meeting.include_system = !s.meeting.include_system
            }
        });
    let mut top = col()
        .w_full()
        .px(18.0)
        .py(14.0)
        .gap(10.0)
        .border(1.0, th::border_subtle())
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(col().grow(1.0).min_w(0.0).child(field(
                    &m.title,
                    "meet/title".into(),
                    "Nombre de la reunión...",
                    |s: &mut State| &mut s.meeting.title,
                )))
                .child(action),
        )
        .child(system);
    if d.live {
        top = top.child(
            text("● Grabando y transcribiendo")
                .font(type_::LABEL)
                .color(red),
        );
    }
    if let Some(e) = &d.error {
        top = top.child(text(e.clone()).font(type_::LABEL).color(red).wrap());
    }
    if let Some(n) = &d.notice {
        top = top.child(text(n.clone()).font(type_::LABEL).color(th::green()).wrap());
    }

    // Transcripción en vivo o el estado vacío.
    let body = if d.segments.is_empty() {
        col()
            .key("meet/transcript")
            .w_full()
            .grow(1.0)
            .center()
            .gap(12.0)
            .child(
                row()
                    .size(64.0, 64.0)
                    .radius(16.0)
                    .center()
                    .fill(th::card())
                    .border(1.0, th::border_base())
                    .child(rune::icon(&icons::MIC_OFF, 26.0).color(th::muted_foreground())),
            )
            .child(
                text(if d.live {
                    "Esperando las primeras palabras…"
                } else {
                    "Inicia una sesión para ver la transcripción en tiempo real"
                })
                .font(type_::BODY.lh(20.0))
                .color(th::muted_foreground())
                .wrap(),
            )
    } else {
        let mut t = col().key("meet/transcript").w_full().padding(18.0).gap(8.0);
        for (seq, line) in d
            .segments
            .iter()
            .rev()
            .take(200)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            t = t.child(
                text(line.clone())
                    .key(format!("meet/seg/{seq}"))
                    .font(type_::BODY.lh(21.0))
                    .color(th::foreground())
                    .wrap(),
            );
        }
        t.scroll(ScrollSpec { stick_bottom: true })
            .grow(1.0)
            .min_h(0.0)
    };

    // Historial a la derecha: título, estado y, si hay fragmentos, el informe.
    let mut past = col().w_full().gap(0.0).child(
        row()
            .w_full()
            .px(16.0)
            .py(14.0)
            .gap(8.0)
            .center_y()
            .border(1.0, th::border_subtle())
            .child(rune::icon(&icons::HISTORY, 14.0).color(th::muted_foreground()))
            .child(
                text(format!("HISTORIAL ({})", d.sessions.len()))
                    .font(TextStyle::new(10.5, 600.0).tracking(1.2))
                    .color(th::muted_foreground()),
            ),
    );
    for sess in d.sessions.iter().take(30) {
        let id = sess.id.clone();
        let generating = d.generating.as_deref() == Some(sess.id.as_str());
        let mut c = col()
            .w_full()
            .px(16.0)
            .py(12.0)
            .gap(6.0)
            .border(1.0, th::border_subtle())
            .child(
                text(if sess.title.is_empty() {
                    "Sin título".to_string()
                } else {
                    sess.title.clone()
                })
                .font(TextStyle::new(13.0, 600.0))
                .color(th::foreground())
                .wrap(),
            )
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        row()
                            .px(8.0)
                            .py(2.0)
                            .radius(6.0)
                            .fill(th::glass_strong())
                            .child(
                                text(status_label(&sess.status).to_string())
                                    .font(TextStyle::new(10.0, 500.0))
                                    .color(th::muted_foreground()),
                            ),
                    )
                    .child(
                        text(format!("{} fragmentos", sess.segments))
                            .font(TextStyle::new(10.5, 400.0))
                            .color(th::text_muted()),
                    ),
            );
        if sess.status != "active" && sess.segments > 0 {
            c = c.child(
                row()
                    .key(format!("meet/report/{}", sess.id))
                    .px(10.0)
                    .py(6.0)
                    .radius(8.0)
                    .fill(amber.alpha(0.15))
                    .border(1.0, amber.alpha(0.35))
                    .center()
                    .child(
                        text(if generating {
                            "Generando…"
                        } else if sess.report_path.is_some() {
                            "Regenerar informe"
                        } else {
                            "Generar informe"
                        })
                        .font(TextStyle::new(11.5, 600.0))
                        .color(amber),
                    )
                    .tap(move |s: &mut State| make_report(s, id.clone())),
            );
        }
        if let Some(p) = &sess.report_path {
            c = c.child(
                text(format!("Informe: {p}"))
                    .font(TextStyle::new(10.0, 400.0).mono())
                    .color(th::text_muted())
                    .wrap(),
            );
        }
        past = past.child(c);
    }
    if d.sessions.is_empty() && d.loaded {
        past = past.child(
            text("Aún no hay reuniones")
                .font(type_::LABEL)
                .color(th::muted_foreground())
                .px(16.0)
                .py(14.0),
        );
    }

    let compact = s.content_width() < 850.0;
    let history = col()
        .key("meet/history")
        .border(1.0, th::border_subtle())
        .child(
            past.scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0),
        );
    let content = if compact {
        col()
            .w_full()
            .gap(12.0)
            .child(
                col()
                    .w_full()
                    .min_h(340.0)
                    .child(top)
                    .child(body.min_h(180.0)),
            )
            .child(history.w_full().h(220.0))
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .grow(1.0)
            .min_h(0.0)
    } else {
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(col().grow(1.0).min_w(0.0).h_full().child(top).child(body))
            .child(history.w(288.0).h_full())
    };
    col().w_full().grow(1.0).min_h(0.0).padding(16.0).child(
        col()
            .key("meet/card")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .radius(14.0)
            .fill(th::card())
            .border(1.0, th::border_subtle())
            .child(header)
            .child(content),
    )
}

fn status_label(s: &str) -> &str {
    match s {
        "active" => "en curso",
        "stopped" => "terminada",
        "report_ready" => "con informe",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_build_the_transcript_and_closing_frees_the_state() {
        let mut d = Data {
            live: true,
            stopping: true,
            ..Default::default()
        };
        let closed = apply_events(
            &mut d,
            vec![
                MEvent::Segment {
                    seq: 0,
                    speaker: None,
                    text: "hola".into(),
                },
                MEvent::Segment {
                    seq: 1,
                    speaker: None,
                    text: String::new(),
                }, // una transcripción vacía no ensucia la lista
                MEvent::Segment {
                    seq: 2,
                    speaker: None,
                    text: "adiós".into(),
                },
                MEvent::Error("sin modelo de voz".into()),
                MEvent::Stopped { segments: 2 },
            ],
        );
        assert!(
            !closed && d.live,
            "hasta que no cierra el socket sigue en curso"
        );
        assert_eq!(
            d.segments,
            [(0, "hola".to_string()), (2, "adiós".to_string())]
        );
        assert_eq!(d.error.as_deref(), Some("sin modelo de voz"));
        assert_eq!(
            d.notice.as_deref(),
            Some("Reunión terminada: 2 fragmentos transcritos")
        );
        assert!(apply_events(&mut d, vec![MEvent::Closed]));
        assert!(!d.live && !d.stopping);
    }
}
