//! Las páginas de trabajo de la web que faltaban: Notas, Cron Jobs (tareas programadas) y API Client.
//! Los datos y las llamadas viven en `system`; aquí solo el estado de la vista y el diseño.

use crate::ui::text;
use crate::ui::Tap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, text_input, Color, El, TextEdit};
use serde_json::json;

use super::{icons, theme as th};
use crate::settings::api::request;
use crate::settings::view::{button, cardx, chip, field, header, icon_box, mono, pill};
use crate::system::{
    date_label, http_payload, parse_http_reply, when_label, CronAction, HttpReply, Part, Task,
};
use crate::State;

// ─── estado ──────────────────────────────────────────────────────────────────

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

/// Qué tareas se enseñan en Cron Jobs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CronFilter {
    #[default]
    All,
    Active,
    Paused,
}

impl CronFilter {
    fn keep(self, t: &Task) -> bool {
        match self {
            CronFilter::All => true,
            CronFilter::Active => t.status == "active",
            CronFilter::Paused => t.status == "paused",
        }
    }
}

/// Lo que ya devolvió el servidor a la última petición del API Client.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub reply: Result<HttpReply, String>,
    pub ms: u128,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReqTab {
    #[default]
    Headers,
    Query,
    Body,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResTab {
    #[default]
    Body,
    Headers,
    Raw,
}

pub struct ApiUi {
    pub method: &'static str,
    pub url: TextEdit,
    /// Una por línea: `Clave: valor`.
    pub headers: TextEdit,
    /// Una por línea: `clave=valor`.
    pub query: TextEdit,
    pub body: TextEdit,
    pub req_tab: ReqTab,
    pub res_tab: ResTab,
    pub sending: bool,
    pub error: Option<String>,
    pub answer: Option<Answer>,
    inbox: Arc<Mutex<Option<Answer>>>,
}

impl Default for ApiUi {
    fn default() -> Self {
        Self {
            method: "GET",
            url: TextEdit::new(""),
            headers: TextEdit::new("Accept: application/json"),
            query: TextEdit::new(""),
            body: TextEdit::new(""),
            req_tab: ReqTab::default(),
            res_tab: ResTab::default(),
            sending: false,
            error: None,
            answer: None,
            inbox: Default::default(),
        }
    }
}

/// «Clave: valor» (o «clave=valor») por línea → pares activos. Las líneas vacías o sin separador se ignoran.
pub fn parse_pairs(t: &str, sep: char) -> Vec<(String, String, bool)> {
    t.lines()
        .filter_map(|l| {
            l.split_once(sep)
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string(), true))
        })
        .filter(|(k, _, _)| !k.is_empty())
        .collect()
}

impl ApiUi {
    /// Manda la petición al servidor, que la hace por nosotros (como la web). No hace nada si falta la URL.
    pub fn send(&mut self, conn: Option<&crate::settings::api::Conn>) {
        if self.sending {
            return;
        }
        let payload = http_payload(
            self.method,
            &self.url.text,
            &parse_pairs(&self.headers.text, ':'),
            &parse_pairs(&self.query.text, '='),
            &self.body.text,
        );
        let payload = match payload {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e.into());
                return;
            }
        };
        let Some(conn) = conn.cloned() else {
            self.error = Some("Sin conexión con el servidor".into());
            return;
        };
        self.error = None;
        self.sending = true;
        let inbox = self.inbox.clone();
        std::thread::spawn(move || {
            let t = Instant::now();
            let reply = request(&conn, "POST", "/api/http-request", Some(payload))
                .map(|v| parse_http_reply(&v))
                .map_err(|e| format!("{e:?}"));
            *inbox.lock().unwrap() = Some(Answer {
                reply,
                ms: t.elapsed().as_millis(),
            });
        });
    }

    /// Recoge la respuesta si ya llegó; devuelve si la vista debe repintarse.
    pub fn sync(&mut self) -> bool {
        let Some(a) = self.inbox.lock().unwrap().take() else {
            return false;
        };
        self.sending = false;
        self.answer = Some(a);
        true
    }
}

#[derive(Default)]
pub struct Work {
    pub cron_filter: CronFilter,
    /// Tarea con el historial desplegado.
    pub history_open: Option<String>,
    /// Tarea a la espera de confirmar el borrado.
    pub confirm_delete: Option<String>,
    /// Nota desplegada (id).
    pub note_open: Option<String>,
    pub api: ApiUi,
}

// ─── piezas ──────────────────────────────────────────────────────────────────

fn page(content: El) -> El {
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

fn banner(s: &State) -> El {
    let d = &s.sys.shown;
    let mut b = col().w_full().gap(6.0);
    if let Some(e) = &d.error {
        b = b.child(text(e.clone()).font(type_::LABEL).color(th::red()).wrap());
    }
    if let Some(n) = &d.notice {
        b = b.child(text(n.clone()).font(type_::LABEL).color(th::green()).wrap());
    }
    b
}

fn panel_head(
    icon: &'static rune::Icon,
    tone: Color,
    title: &str,
    sub: &str,
    right: Option<El>,
) -> El {
    row()
        .w_full()
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(icon_box(icon, tone, 40.0))
        .child(
            col()
                .grow(1.0)
                .shrink()
                .gap(1.0)
                .child(
                    text(title.to_string())
                        .font(TextStyle::new(18.0, 700.0))
                        .color(th::foreground()),
                )
                .child(
                    text(sub.to_string())
                        .font(TextStyle::new(12.5, 400.0))
                        .color(th::muted_foreground()),
                ),
        )
        .children(right)
}

fn empty(msg: &str) -> El {
    col().w_full().padding(36.0).center().child(
        text(msg.to_string())
            .font(type_::BODY)
            .color(th::text_muted()),
    )
}

fn ghost_button(
    key: String,
    icon: &'static rune::Icon,
    label: &str,
    tone: Color,
    on: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .gap(7.0)
        .center_y()
        .px(12.0)
        .py(7.0)
        .radius(8.0)
        .border(1.0, th::border_base())
        .fill(th::glass())
        .child(rune::icon(icon, 13.0).color(tone))
        .child(
            text(label.to_string())
                .font(TextStyle::new(12.0, 500.0))
                .color(th::foreground()),
        )
        .tap(on)
}

// ─── Notas ───────────────────────────────────────────────────────────────────

pub fn notes(s: &State) -> El {
    let d = &s.sys.shown;
    let mut list = col().w_full().gap(10.0);
    if d.notes.is_empty() {
        list = list.child(empty(if d.notes_loaded {
            "Aún no hay notas. Los agentes las guardan aquí cuando necesitan recordar algo."
        } else {
            "Cargando notas…"
        }));
    }
    for n in &d.notes {
        let open = s.work.note_open.as_deref() == Some(n.id.as_str());
        let id = n.id.clone();
        let tag = n.thread_tag();
        list = list.child(
            cardx()
                .key(format!("note/{}", n.id))
                .w_full()
                .padding(16.0)
                .gap(10.0)
                .child(
                    text(n.key.clone())
                        .font(TextStyle::new(14.0, 700.0))
                        .color(th::foreground()),
                )
                .child(
                    text(if open {
                        n.value.clone()
                    } else {
                        one_line(&n.value, 340)
                    })
                    .font(TextStyle::new(12.5, 400.0).lh(19.0))
                    .color(th::muted_foreground())
                    .wrap()
                    .selectable(true),
                )
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(
                            row()
                                .gap(8.0)
                                .children((!tag.is_empty()).then(|| chip(&tag, th::amber())))
                                .children(
                                    (!n.source.is_empty())
                                        .then(|| chip(&n.source, th::muted_foreground())),
                                ),
                        )
                        .child(
                            text(date_label(n.created_at))
                                .font(TextStyle::new(10.5, 400.0).mono())
                                .color(th::text_muted()),
                        ),
                )
                .tap(move |s: &mut State| {
                    s.work.note_open = if s.work.note_open.as_deref() == Some(id.as_str()) {
                        None
                    } else {
                        Some(id.clone())
                    }
                }),
        );
    }
    let panel = cardx()
        .w_full()
        .padding(22.0)
        .gap(16.0)
        .child(panel_head(
            &icons::FILE,
            th::amber(),
            "Notas",
            "Scratchpad del agente",
            None,
        ))
        .child(list);
    page(
        col()
            .w_full()
            .gap(22.0)
            .child(header(
                "Memoria operativa",
                "Notas",
                "del Sistema",
                "Registro de información persistente y recordatorios del enjambre.",
                Some(refresh_notes(s)),
            ))
            .child(banner(s))
            .child(panel),
    )
}

fn refresh_notes(s: &State) -> El {
    let busy = s.sys.busy();
    row()
        .key("notes/refresh")
        .gap(8.0)
        .center_y()
        .px(14.0)
        .py(9.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(rune::icon(&icons::REFRESH, 13.0).color(th::muted_foreground()))
        .child(
            text(if busy {
                "ACTUALIZANDO…"
            } else {
                "ACTUALIZAR"
            })
            .font(mono(10.5, 1.2))
            .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            if let Some(c) = s.settings.conn.clone() {
                s.sys.refresh(&c, Part::Notes);
            }
        })
}

/// Una sola línea de hasta `max` caracteres, con «…» si se corta.
pub fn one_line(t: &str, max: usize) -> String {
    let flat: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>().trim_end())
    }
}

// ─── Cron Jobs ───────────────────────────────────────────────────────────────

fn status_chip(t: &Task) -> El {
    match t.status.as_str() {
        "active" => chip("ACTIVA", th::green()),
        "paused" => chip("PAUSADA", th::amber()),
        "completed" => chip("COMPLETADA", th::blue()),
        other => chip(&other.to_uppercase(), th::muted_foreground()),
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "recurring" => "RECURRENTE",
        "one_shot" => "UNA VEZ",
        "interval" => "INTERVALO",
        _ => "TAREA",
    }
}

fn task_card(s: &State, t: &Task) -> El {
    let (id_p, id_t, id_h, id_d, id_c) = (
        t.id.clone(),
        t.id.clone(),
        t.id.clone(),
        t.id.clone(),
        t.id.clone(),
    );
    let paused = t.status == "paused";
    let open = s.work.history_open.as_deref() == Some(t.id.as_str());
    let confirming = s.work.confirm_delete.as_deref() == Some(t.id.as_str());
    let key = |k: &str| format!("cron/{k}/{}", t.id);
    let run = |id: String, a: CronAction| {
        move |s: &mut State| {
            if let Some(c) = s.settings.conn.clone() {
                s.sys.cron_action(&c, &id, a);
            }
        }
    };
    let mut meta = row().gap(8.0).center_y().flex_wrap().child(
        row()
            .px(9.0)
            .py(2.0)
            .radius(9.0)
            .fill(th::purple().alpha(0.12))
            .border(1.0, th::purple().alpha(0.3))
            .child(
                text(kind_label(&t.kind))
                    .font(mono(9.0, 0.8))
                    .color(th::purple()),
            ),
    );
    if !t.cron.is_empty() {
        meta = meta.child(
            row()
                .gap(5.0)
                .center_y()
                .px(9.0)
                .py(2.0)
                .radius(9.0)
                .fill(th::amber().alpha(0.12))
                .border(1.0, th::amber().alpha(0.3))
                .child(rune::icon(&icons::CALENDAR, 10.0).color(th::amber()))
                .child(
                    text(t.cron.clone())
                        .font(mono(10.0, 0.6))
                        .color(th::amber()),
                ),
        );
    }
    if !t.channel.is_empty() {
        meta = meta.child(chip(&t.channel.to_uppercase(), th::muted_foreground()));
    }
    let mut c = cardx()
        .key(key("card"))
        .w_full()
        .padding(16.0)
        .gap(10.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    text(t.name.clone())
                        .font(TextStyle::new(14.5, 700.0))
                        .color(th::foreground())
                        .grow(1.0),
                )
                .child(status_chip(t)),
        )
        .child(meta)
        .child(
            row()
                .gap(10.0)
                .center_y()
                .child(rune::icon(&icons::ACTIVITY, 11.0).color(th::muted_foreground()))
                .child(
                    text(format!("{} runs", t.run_count))
                        .font(TextStyle::new(11.0, 400.0))
                        .color(th::muted_foreground()),
                )
                .children((!t.timezone.is_empty()).then(|| {
                    text(t.timezone.clone())
                        .font(TextStyle::new(10.5, 400.0).mono())
                        .color(th::text_muted())
                })),
        );
    let mut when = row().gap(14.0).flex_wrap();
    if !t.last_run.is_empty() {
        when = when.child(
            text(format!("Último: {}", when_label(&t.last_run)))
                .font(TextStyle::new(11.0, 400.0))
                .color(th::muted_foreground()),
        );
    }
    if !t.next_run.is_empty() && !paused {
        when = when.child(
            text(format!("Próximo: {}", when_label(&t.next_run)))
                .font(TextStyle::new(11.0, 400.0))
                .color(th::amber()),
        );
    }
    c = c.child(when);
    if !t.last_error.is_empty() {
        c = c.child(
            row()
                .w_full()
                .px(10.0)
                .py(6.0)
                .radius(6.0)
                .fill(th::red().alpha(0.1))
                .border(1.0, th::red().alpha(0.3))
                .child(
                    text(t.last_error.clone())
                        .font(TextStyle::new(11.0, 400.0))
                        .color(th::red())
                        .wrap(),
                ),
        );
    }
    let toggle = if paused {
        ghost_button(
            key("resume"),
            &icons::PLAY,
            "Reanudar",
            th::green(),
            run(id_p, CronAction::Resume),
        )
    } else {
        ghost_button(
            key("pause"),
            &icons::PAUSE,
            "Pausar",
            th::foreground(),
            run(id_p, CronAction::Pause),
        )
    };
    let delete = if confirming {
        row()
            .gap(8.0)
            .center_y()
            .child(
                text("¿Eliminar?")
                    .font(TextStyle::new(12.0, 500.0))
                    .color(th::red()),
            )
            .child(button(
                "Sí, eliminar",
                key("delete-yes"),
                th::red().alpha(0.7),
                move |s: &mut State| {
                    s.work.confirm_delete = None;
                    if let Some(c) = s.settings.conn.clone() {
                        s.sys.cron_action(&c, &id_c, CronAction::Delete);
                    }
                },
            ))
            .child(button(
                "Cancelar",
                key("delete-no"),
                th::glass_strong(),
                |s: &mut State| s.work.confirm_delete = None,
            ))
    } else {
        row()
            .key(key("delete"))
            .gap(7.0)
            .center_y()
            .px(10.0)
            .py(7.0)
            .child(rune::icon(&icons::TRASH, 13.0).color(th::muted_foreground()))
            .child(
                text("Eliminar")
                    .font(TextStyle::new(12.0, 400.0))
                    .color(th::muted_foreground()),
            )
            .tap(move |s: &mut State| s.work.confirm_delete = Some(id_d.clone()))
    };
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .flex_wrap()
            .child(toggle)
            .child(ghost_button(
                key("trigger"),
                &icons::ZAP,
                "Ejecutar",
                th::foreground(),
                run(id_t, CronAction::Trigger),
            ))
            .child(
                row()
                    .key(key("history"))
                    .gap(7.0)
                    .center_y()
                    .px(10.0)
                    .py(7.0)
                    .child(rune::icon(&icons::HISTORY, 13.0).color(th::muted_foreground()))
                    .child(
                        text("Historial")
                            .font(TextStyle::new(12.0, 400.0))
                            .color(th::muted_foreground()),
                    )
                    .child(
                        rune::icon(
                            if open {
                                &icons::CHEVRON_UP
                            } else {
                                &icons::CHEVRON_DOWN
                            },
                            12.0,
                        )
                        .color(th::muted_foreground()),
                    )
                    .tap(move |s: &mut State| {
                        if s.work.history_open.as_deref() == Some(id_h.as_str()) {
                            s.work.history_open = None;
                        } else {
                            s.work.history_open = Some(id_h.clone());
                            if let Some(c) = s.settings.conn.clone() {
                                s.sys.read_history(&c, &id_h);
                            }
                        }
                    }),
            )
            .child(delete),
    );
    if open {
        let runs = s.sys.shown.history.get(&t.id);
        let mut h = col()
            .w_full()
            .gap(6.0)
            .padding(10.0)
            .radius(8.0)
            .fill(th::glass())
            .border(1.0, th::border_subtle());
        match runs {
            None => {
                h = h.child(
                    text("Cargando historial…")
                        .font(type_::LABEL)
                        .color(th::text_muted()),
                )
            }
            Some(r) if r.is_empty() => {
                h = h.child(
                    text("Todavía no se ha ejecutado.")
                        .font(type_::LABEL)
                        .color(th::text_muted()),
                )
            }
            Some(r) => {
                for x in r {
                    let ok = x.status == "success";
                    let detail = if !x.error.is_empty() {
                        x.error.clone()
                    } else {
                        one_line(&x.response, 140)
                    };
                    h = h.child(
                        row()
                            .w_full()
                            .gap(10.0)
                            .center_y()
                            .child(rune::el().size(7.0, 7.0).radius(3.5).fill(if ok {
                                th::green()
                            } else {
                                th::red()
                            }))
                            .child(
                                text(when_label(&x.started))
                                    .font(TextStyle::new(11.0, 500.0).mono())
                                    .color(th::foreground()),
                            )
                            .child(
                                text(format!("{:.1} s", x.duration_ms as f64 / 1000.0))
                                    .font(TextStyle::new(10.5, 400.0).mono())
                                    .color(th::text_muted()),
                            )
                            .child(
                                text(detail)
                                    .font(TextStyle::new(11.0, 400.0))
                                    .color(if ok {
                                        th::muted_foreground()
                                    } else {
                                        th::red()
                                    })
                                    .grow(1.0),
                            ),
                    );
                }
            }
        }
        c = c.child(h);
    }
    c
}

pub fn cron(s: &State) -> El {
    let d = &s.sys.shown;
    let shown: Vec<&Task> = d
        .tasks
        .iter()
        .filter(|t| !t.internal && s.work.cron_filter.keep(t))
        .collect();
    let f = s.work.cron_filter;
    let filters = row()
        .gap(8.0)
        .child(pill(
            "Todas".into(),
            "cron/f/all".into(),
            f == CronFilter::All,
            |s: &mut State| s.work.cron_filter = CronFilter::All,
        ))
        .child(pill(
            "Activas".into(),
            "cron/f/active".into(),
            f == CronFilter::Active,
            |s: &mut State| s.work.cron_filter = CronFilter::Active,
        ))
        .child(pill(
            "Pausadas".into(),
            "cron/f/paused".into(),
            f == CronFilter::Paused,
            |s: &mut State| s.work.cron_filter = CronFilter::Paused,
        ));
    let mut list = col().w_full().gap(12.0);
    if shown.is_empty() {
        list = list.child(empty(if !d.tasks_loaded {
            "Cargando tareas…"
        } else if d.tasks.iter().any(|t| !t.internal) {
            "Ninguna tarea con este filtro."
        } else {
            "No hay tareas programadas. Pídele a Bee que programe una."
        }));
    }
    for t in shown {
        list = list.child(task_card(s, t));
    }
    let refresh = row()
        .key("cron/refresh")
        .gap(8.0)
        .center_y()
        .px(14.0)
        .py(9.0)
        .radius(10.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(rune::icon(&icons::REFRESH, 13.0).color(th::muted_foreground()))
        .child(
            text(if s.sys.busy() {
                "ACTUALIZANDO…"
            } else {
                "ACTUALIZAR"
            })
            .font(mono(10.5, 1.2))
            .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            if let Some(c) = s.settings.conn.clone() {
                s.sys.refresh(&c, Part::Cron);
            }
        });
    let panel = cardx()
        .w_full()
        .padding(22.0)
        .gap(16.0)
        .child(panel_head(
            &icons::CLOCK,
            th::amber(),
            "Tareas Programadas",
            "Scheduler basado en Croner",
            Some(
                row()
                    .w_full()
                    .gap(10.0)
                    .flex_wrap()
                    .center_y()
                    .child(filters)
                    .child(refresh),
            ),
        ))
        .child(list);
    page(
        col()
            .w_full()
            .gap(22.0)
            .child(header(
                "Automatización temporal",
                "Tareas",
                "Programadas",
                "Gestión de ejecuciones recurrentes y disparadores cron.",
                None,
            ))
            .child(banner(s))
            .child(panel),
    )
}

// ─── API Client ──────────────────────────────────────────────────────────────

fn method_tone(m: &str) -> Color {
    match m {
        "GET" => th::green(),
        "POST" => th::amber(),
        "PUT" | "PATCH" => th::blue(),
        "DELETE" => th::red(),
        _ => th::purple(),
    }
}

fn status_tone(code: u16) -> Color {
    match code {
        200..=299 => th::green(),
        300..=399 => th::amber(),
        _ => th::red(),
    }
}

fn area(
    edit: &TextEdit,
    key: &str,
    hint: &str,
    pick: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
) -> El {
    row()
        .w_full()
        .min_h(120.0)
        .px(10.0)
        .py(8.0)
        .radius(radius::SM)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(
            text_input(edit, TextStyle::new(12.5, 400.0).mono().lh(19.0))
                .key(key.to_string())
                .grow(1.0)
                .color(th::foreground())
                .placeholder(hint, th::text_muted())
                .on_edit(pick),
        )
}

pub fn api_client(s: &State) -> El {
    let a = &s.work.api;
    let methods = row().gap(6.0).flex_wrap().children(METHODS.iter().map(|m| {
        let on = a.method == *m;
        let m2: &'static str = m;
        row()
            .key(format!("api/m/{m}"))
            .px(12.0)
            .py(7.0)
            .radius(8.0)
            .center()
            .fill(if on {
                method_tone(m).alpha(0.16)
            } else {
                th::glass_strong()
            })
            .border(
                1.0,
                if on {
                    method_tone(m).alpha(0.6)
                } else {
                    th::border_base()
                },
            )
            .child(text(m.to_string()).font(mono(11.0, 0.8)).color(if on {
                method_tone(m)
            } else {
                th::muted_foreground()
            }))
            .tap(move |s: &mut State| s.work.api.method = m2)
    }));
    let send = row()
        .key("api/send")
        .gap(8.0)
        .center_y()
        .px(18.0)
        .py(9.0)
        .radius(10.0)
        .fill(th::blue_dark())
        .border(1.0, th::blue().alpha(0.6))
        .child(rune::icon(&icons::SEND, 13.0).color(Color::WHITE))
        .child(
            text(if a.sending { "ENVIANDO…" } else { "ENVIAR" })
                .font(mono(10.5, 1.2))
                .color(Color::WHITE),
        )
        .tap(|s: &mut State| {
            let conn = s.settings.conn.clone();
            s.work.api.send(conn.as_ref());
        });
    let url_row = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(row().grow(1.0).child(field(
            &a.url,
            "api/url".into(),
            "https://api.ejemplo.com/recurso",
            |s: &mut State| &mut s.work.api.url,
        )))
        .child(send);
    let rt = a.req_tab;
    let req_tabs = row()
        .gap(8.0)
        .child(pill(
            "Cabeceras".into(),
            "api/rt/headers".into(),
            rt == ReqTab::Headers,
            |s: &mut State| s.work.api.req_tab = ReqTab::Headers,
        ))
        .child(pill(
            "Parámetros".into(),
            "api/rt/query".into(),
            rt == ReqTab::Query,
            |s: &mut State| s.work.api.req_tab = ReqTab::Query,
        ))
        .child(pill(
            "Cuerpo".into(),
            "api/rt/body".into(),
            rt == ReqTab::Body,
            |s: &mut State| s.work.api.req_tab = ReqTab::Body,
        ));
    let editor = match rt {
        ReqTab::Headers => area(
            &a.headers,
            "api/headers",
            "Clave: valor (una por línea)",
            |s: &mut State| &mut s.work.api.headers,
        ),
        ReqTab::Query => area(
            &a.query,
            "api/query",
            "clave=valor (una por línea)",
            |s: &mut State| &mut s.work.api.query,
        ),
        ReqTab::Body => area(
            &a.body,
            "api/body",
            if matches!(a.method, "GET" | "HEAD") {
                "GET y HEAD no llevan cuerpo"
            } else {
                "{\"clave\": \"valor\"}"
            },
            |s: &mut State| &mut s.work.api.body,
        ),
    };
    let request_card = cardx()
        .w_full()
        .padding(22.0)
        .gap(14.0)
        .child(panel_head(
            &icons::GLOBE,
            th::blue(),
            "Petición",
            "El servidor hace la llamada por ti, así no hay límites de origen",
            None,
        ))
        .child(methods)
        .child(url_row)
        .children(
            a.error
                .clone()
                .map(|e| text(e).font(type_::LABEL).color(th::red()).wrap()),
        )
        .child(req_tabs)
        .child(editor);

    let response = cardx()
        .key("api/response")
        .w_full()
        .padding(22.0)
        .gap(14.0)
        .child(match &a.answer {
            None => col()
                .w_full()
                .gap(10.0)
                .child(panel_head(
                    &icons::ACTIVITY,
                    th::green(),
                    "Respuesta",
                    "Aquí aparece lo que devuelva el servidor",
                    None,
                ))
                .child(empty(if a.sending {
                    "Esperando respuesta…"
                } else {
                    "Aún no hay respuesta."
                })),
            Some(Answer { reply: Err(e), .. }) => col()
                .w_full()
                .gap(10.0)
                .child(panel_head(
                    &icons::ACTIVITY,
                    th::red(),
                    "Respuesta",
                    "No se pudo completar la petición",
                    None,
                ))
                .child(text(e.clone()).font(type_::BODY).color(th::red()).wrap()),
            Some(Answer { reply: Ok(r), ms }) => response_view(a, r, *ms),
        });
    page(
        col()
            .w_full()
            .gap(22.0)
            .child(header(
                "Pruebas de servicios",
                "API",
                "Client",
                "Lanza peticiones HTTP y mira la respuesta, sin salir de Hive.",
                None,
            ))
            .child(request_card)
            .child(response),
    )
}

fn response_view(a: &ApiUi, r: &HttpReply, ms: u128) -> El {
    let (tone, code) = match r.status {
        Some(c) => (status_tone(c), format!("{} {}", c, r.status_text)),
        None => (th::red(), "Sin estado".into()),
    };
    let bad = r.error.clone();
    let rt = a.res_tab;
    let tabs = row()
        .gap(8.0)
        .child(pill(
            "Cuerpo".into(),
            "api/res/body".into(),
            rt == ResTab::Body,
            |s: &mut State| s.work.api.res_tab = ResTab::Body,
        ))
        .child(pill(
            format!("Cabeceras ({})", r.headers.len()),
            "api/res/headers".into(),
            rt == ResTab::Headers,
            |s: &mut State| s.work.api.res_tab = ResTab::Headers,
        ))
        .child(pill(
            "Crudo".into(),
            "api/res/raw".into(),
            rt == ResTab::Raw,
            |s: &mut State| s.work.api.res_tab = ResTab::Raw,
        ));
    let mono_text = |t: String| {
        text(t)
            .font(TextStyle::new(12.0, 400.0).mono().lh(18.0))
            .color(th::foreground())
            .wrap()
            .selectable(true)
    };
    let content = match rt {
        ResTab::Body => mono_text(if r.body.is_empty() {
            "(sin cuerpo)".into()
        } else {
            r.body.clone()
        }),
        ResTab::Headers => {
            let mut c = col().w_full().gap(4.0);
            for (k, v) in &r.headers {
                c = c.child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(
                            text(k.clone())
                                .font(TextStyle::new(12.0, 600.0).mono())
                                .color(th::blue())
                                .w(160.0)
                                .shrink(),
                        )
                        .child(
                            text(v.clone())
                                .font(TextStyle::new(12.0, 400.0).mono())
                                .color(th::foreground())
                                .wrap()
                                .grow(1.0)
                                .selectable(true),
                        ),
                );
            }
            c
        }
        ResTab::Raw => {
            let mut raw = format!("HTTP {code}\n");
            for (k, v) in &r.headers {
                raw.push_str(&format!("{k}: {v}\n"));
            }
            raw.push('\n');
            raw.push_str(&r.body);
            mono_text(raw)
        }
    };
    col()
        .w_full()
        .gap(14.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(icon_box(&icons::ACTIVITY, tone, 40.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            text("Respuesta")
                                .font(TextStyle::new(18.0, 700.0))
                                .color(th::foreground()),
                        )
                        .child(
                            row().gap(10.0).center_y().child(chip(&code, tone)).child(
                                text(format!("{ms} ms"))
                                    .font(TextStyle::new(11.0, 400.0).mono())
                                    .color(th::muted_foreground()),
                            ),
                        ),
                ),
        )
        .children(bad.map(|e| text(e).font(type_::LABEL).color(th::red()).wrap()))
        .child(tabs)
        .child(
            col()
                .w_full()
                .padding(14.0)
                .radius(8.0)
                .fill(th::glass())
                .border(1.0, th::border_subtle())
                .child(content),
        )
}

/// El cuerpo que se mandaría, para pruebas.
#[allow(dead_code)]
pub fn preview_payload(a: &ApiUi) -> Result<serde_json::Value, &'static str> {
    http_payload(
        a.method,
        &a.url.text,
        &parse_pairs(&a.headers.text, ':'),
        &parse_pairs(&a.query.text, '='),
        &a.body.text,
    )
}

#[allow(dead_code)]
fn _unused() -> serde_json::Value {
    json!({})
}
