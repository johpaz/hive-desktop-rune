//! Las páginas del sistema, con el diseño de la web: Dashboard (el centro de control), Ética, Perfil,
//! Pantalla y audio, y Actualizaciones. Los datos vienen de `system` y de `audio_prefs`.

use crate::ui::text;
use crate::ui::Tap;
use hive_media::{AudioDevice, Capture, Direction};
use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, Color, El, LineCap, Shape, TextEdit};

use super::{icons, theme as th};
use crate::office::model::{derive, DeskState};
use crate::settings::view::{cardx, chip, field, grid, header, icon_box, mono, pill, short};
use crate::system::{describe_reason, hour_label};
use crate::State;

// ─── estado de las páginas ───────────────────────────────────────────────────

/// El formulario del perfil: se rellena con el usuario del servidor y se edita con «Editar».
#[derive(Default)]
pub struct ProfileForm {
    pub name: TextEdit,
    pub occupation: TextEdit,
    pub email: TextEdit,
    pub language: TextEdit,
    pub timezone: TextEdit,
    pub notes: TextEdit,
    pub editing: bool,
    /// El usuario con el que se rellenó (para no pisar lo escrito en cada lectura).
    pub filled_for: Option<crate::system::User>,
}

impl ProfileForm {
    pub fn fill(&mut self, u: &crate::system::User) {
        self.name = TextEdit::new(u.name.clone());
        self.occupation = TextEdit::new(u.occupation.clone());
        self.email = TextEdit::new(u.email.clone());
        self.language = TextEdit::new(u.language.clone());
        self.timezone = TextEdit::new(u.timezone.clone());
        self.notes = TextEdit::new(u.notes.clone());
        self.filled_for = Some(u.clone());
    }

    /// Rellena el formulario cuando llega un usuario distinto o cambiado, salvo que se esté editando.
    pub fn follow(&mut self, user: Option<&crate::system::User>) {
        if let Some(u) = user {
            if !self.editing && self.filled_for.as_ref() != Some(u) {
                self.fill(u);
            }
        }
    }
}

/// Una prueba de micrófono en curso: abre la captura unos segundos y mide el nivel.
pub struct MicTest {
    _capture: Capture,
    started: std::time::Instant,
}

#[derive(Default)]
pub struct AudioUi {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
    pub prefs: crate::audio_prefs::Prefs,
    /// Cámaras del sistema que de verdad entregan imagen.
    pub cameras: Vec<hive_media::CameraDevice>,
    /// Lista desplegada: 0 = ninguna, 1 = micrófono, 2 = salida, 3 = cámara.
    pub open: u8,
    /// Qué aparato cambió la persona y aún no se ha aplicado a la llamada en curso (1, 2 o 3).
    pub changed: Option<u8>,
    pub mic_test: Option<MicTest>,
    /// Pico del micrófono durante la prueba, de 0 a 1 (`None` = sin probar).
    pub mic_peak: Option<f32>,
    pub message: Option<String>,
}

const MIC_TEST_SECS: u64 = 3;

/// La entrada `default_*` que PipeWire publica repite la opción «Predeterminado del sistema»: se quita.
pub fn without_system_default(list: Vec<AudioDevice>) -> Vec<AudioDevice> {
    list.into_iter()
        .filter(|d| !d.id.ends_with("_default"))
        .collect()
}

impl AudioUi {
    /// Lee los aparatos del sistema y la elección guardada.
    pub fn load(&mut self) {
        self.inputs =
            without_system_default(crate::voice::devices(Direction::Input).unwrap_or_default());
        self.outputs =
            without_system_default(crate::voice::devices(Direction::Output).unwrap_or_default());
        self.cameras = hive_media::list_cameras().unwrap_or_default();
        self.prefs = crate::audio_prefs::load();
        self.open = 0;
        self.changed = None;
    }

    pub fn choose(&mut self, input: bool, id: Option<String>) {
        if input {
            self.prefs.input = id;
        } else {
            self.prefs.output = id;
        }
        crate::audio_prefs::save(&self.prefs);
        self.open = 0;
        self.changed = Some(if input { 1 } else { 2 });
    }

    pub fn choose_camera(&mut self, id: Option<String>) {
        self.prefs.camera = id;
        crate::audio_prefs::save(&self.prefs);
        self.open = 0;
        self.changed = Some(3);
    }

    pub fn start_mic_test(&mut self) {
        self.mic_peak = Some(0.0);
        self.message = None;
        match Capture::start(self.prefs.input.as_deref(), None, |_| {}) {
            Ok(c) => {
                self.mic_test = Some(MicTest {
                    _capture: c,
                    started: std::time::Instant::now(),
                })
            }
            Err(e) => {
                self.mic_peak = None;
                self.message = Some(e.message);
            }
        }
    }

    /// Avanza la prueba del micrófono; devuelve si la vista debe repintarse.
    pub fn tick(&mut self) -> bool {
        let Some(t) = &self.mic_test else {
            return false;
        };
        let level = t._capture.level();
        self.mic_peak = Some(self.mic_peak.unwrap_or(0.0).max(level));
        if t.started.elapsed().as_secs() >= MIC_TEST_SECS {
            self.mic_test = None; // se suelta el micrófono al terminar
        }
        true
    }

    /// Suena el tono de prueba por la salida elegida, en un hilo.
    pub fn play_test(&self) {
        let out = self.prefs.output.clone();
        std::thread::spawn(move || {
            if let Ok(p) = hive_media::Player::start(out.as_deref(), None) {
                p.enqueue(&crate::voice::test_tone());
                std::thread::sleep(std::time::Duration::from_millis(600));
            }
        });
    }
}

// ─── piezas ──────────────────────────────────────────────────────────────────

fn label(t: &str) -> El {
    text(t.to_uppercase())
        .font(mono(9.5, 1.6))
        .color(th::muted_foreground())
}

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

fn meter(frac: f32, tone: Color) -> El {
    let f = frac.clamp(0.0, 1.0);
    row()
        .w_full()
        .h(6.0)
        .radius(3.0)
        .fill(th::muted())
        .children((f > 0.0).then(|| col().grow(f.max(0.02)).h(6.0).radius(3.0).fill(tone)))
        .children((f < 1.0).then(|| col().grow((1.0 - f).max(0.0001)).h(6.0)))
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

// ─── Dashboard ───────────────────────────────────────────────────────────────

/// Cuántos especialistas hay en cada estado, a partir de los agentes y del grafo en vivo (como el Dashboard de la web).
pub fn swarm_counts(s: &State) -> (usize, usize, usize, usize) {
    let (_, desks) = derive(dashboard_agents(s), &s.office.graph.nodes);
    let n = |f: &dyn Fn(DeskState) -> bool| desks.iter().filter(|d| f(d.state)).count();
    (
        n(&|x| x == DeskState::Idle),
        n(&|x| matches!(x, DeskState::Thinking | DeskState::ToolCall)),
        n(&|x| x == DeskState::Stuck),
        n(&|x| matches!(x, DeskState::Disabled | DeskState::Archived)),
    )
}

fn dashboard_agents(s: &State) -> &[crate::settings::api::Agent] {
    if crate::chat_only(s) {
        &s.office.agents
    } else {
        &s.settings.shown.agents
    }
}

fn stat_card(name: &str, value: String, icon: &'static rune::Icon, tone: Color, trend: &str) -> El {
    cardx()
        .w(300.0)
        .min_w(260.0)
        .grow(1.0)
        .padding(22.0)
        .gap(14.0)
        .border(1.0, th::blue().alpha(0.2))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    text(name.to_uppercase())
                        .font(mono(11.0, 2.0))
                        .color(th::blue()),
                )
                .child(
                    row()
                        .size(32.0, 32.0)
                        .radius(8.0)
                        .center()
                        .fill(th::glass_strong())
                        .border(1.0, th::border_subtle())
                        .child(rune::icon(icon, 15.0).color(tone)),
                ),
        )
        .child(
            text(value)
                .font(TextStyle::new(32.0, 900.0))
                .color(th::foreground()),
        )
        .child(
            row()
                .gap(6.0)
                .center_y()
                .child(
                    text("↗")
                        .font(TextStyle::new(10.0, 700.0))
                        .color(th::green()),
                )
                .child(
                    text(trend.to_uppercase())
                        .font(mono(9.5, 1.4))
                        .color(th::text_muted()),
                ),
        )
}

fn metric(
    icon: &'static rune::Icon,
    name: &str,
    value: String,
    tone: Color,
    frac: f32,
    note: String,
) -> El {
    col()
        .w(190.0)
        .min_w(170.0)
        .grow(1.0)
        .padding(14.0)
        .gap(10.0)
        .radius(10.0)
        .fill(th::glass())
        .border(1.0, th::border_base())
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(rune::icon(icon, 14.0).color(tone))
                        .child(
                            text(name.to_string())
                                .font(TextStyle::new(11.5, 400.0))
                                .color(th::muted_foreground())
                                .wrap(),
                        ),
                )
                .child(
                    text(value)
                        .font(TextStyle::new(17.0, 700.0).mono())
                        .color(tone),
                ),
        )
        .child(meter(frac, tone))
        .child(
            text(note)
                .font(TextStyle::new(11.5, 400.0))
                .color(th::muted_foreground())
                .wrap(),
        )
}

fn fmt_tokens(n: u64) -> String {
    crate::office::hud::format_token_count(n as f64)
}

fn money(v: f64) -> String {
    if v <= 0.0 {
        "$0.00".into()
    } else if v < 0.01 {
        format!("${v:.4}")
    } else {
        format!("${v:.2}")
    }
}

/// El gráfico de actividad: un área azul bajo una línea, estirada al ancho de la tarjeta.
fn activity_chart(data: &[(String, u64)]) -> El {
    if data.is_empty() {
        return col().w_full().h(200.0).center().child(
            text("Cargando actividad…")
                .font(type_::BODY)
                .color(th::text_muted()),
        );
    }
    let max = data.iter().map(|d| d.1).max().unwrap_or(1).max(1) as f32;
    let n = data.len();
    let (w, h) = (1000.0f32, 200.0f32);
    let pt = |i: usize, c: u64| {
        [
            if n > 1 {
                i as f32 / (n - 1) as f32 * w
            } else {
                w / 2.0
            },
            h - 8.0 - (c as f32 / max) * (h - 24.0),
        ]
    };
    let line: Vec<[f32; 2]> = data.iter().enumerate().map(|(i, d)| pt(i, d.1)).collect();
    let mut area = line.clone();
    area.push([line.last().map_or(w, |p| p[0]), h]);
    area.push([line[0][0], h]);
    let grid_color = if th::is_light() {
        Color::BLACK.alpha(0.06)
    } else {
        Color::WHITE.alpha(0.05)
    };
    let mut shape = Shape::new().view_box(w, h);
    for k in 0..4 {
        let y = 8.0 + k as f32 * (h - 16.0) / 3.0;
        shape = shape.line([0.0, y], [w, y], 1.0, LineCap::Butt, grid_color);
    }
    shape = shape
        .fill(area, th::blue().alpha(0.14))
        .stroke(line, 3.0, th::blue());
    let labels = row()
        .w_full()
        .justify_between()
        .child(
            text(hour_label(&data[0].0))
                .font(mono(9.5, 0.6))
                .color(th::text_muted()),
        )
        .child(
            text(hour_label(&data[n / 2].0))
                .font(mono(9.5, 0.6))
                .color(th::text_muted()),
        )
        .child(
            text(hour_label(&data[n - 1].0))
                .font(mono(9.5, 0.6))
                .color(th::text_muted()),
        );
    col()
        .w_full()
        .gap(6.0)
        .child(rune::canvas(shape).w_full().h(h))
        .child(labels)
}

fn health_bar(name: &str, n: usize, total: usize, tone: Color) -> El {
    let f = if total == 0 {
        0.0
    } else {
        n as f32 / total as f32
    };
    row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            text(name.to_uppercase())
                .font(mono(10.0, 0.8))
                .color(th::muted_foreground())
                .w(96.0),
        )
        .child(col().grow(1.0).child(meter(f, tone)))
        .child(
            text(n.to_string())
                .font(TextStyle::new(12.0, 700.0).mono())
                .color(th::foreground())
                .w(24.0),
        )
}

pub fn dashboard(s: &State) -> El {
    let d = &s.sys.shown;
    let stats = d.stats.clone().unwrap_or_default();
    let rust = stats.runtime == "rust" || crate::chat_only(s);
    let measured = !rust || stats.process_metrics_available;
    let online = s.chat_ready;
    let (idle, working, stuck, disabled) = swarm_counts(s);
    let agents = dashboard_agents(s).len();
    let last_hour = d.activity.last().map_or(0, |a| a.1);
    let total_msgs: u64 = d.activity.iter().map(|a| a.1).sum();

    // Cabecera: logo, título y el estado y la versión a la derecha.
    let version = match &d.version {
        Some(v) => row()
            .gap(10.0)
            .center_y()
            .child(if v.status == "local" {
                chip("Versión local", th::blue())
            } else if v.up_to_date() {
                chip("Actualizado", th::green())
            } else {
                chip(&format!("Disponible v{}", v.latest), th::amber())
            })
            .child(
                text(format!(
                    "v{} · {}",
                    v.current,
                    if v.installation == "binary" {
                        "Binario"
                    } else {
                        v.installation.as_str()
                    }
                ))
                .font(TextStyle::new(11.5, 400.0).mono())
                .color(th::muted_foreground()),
            ),
        None => row().child(text("—").font(type_::BODY).color(th::text_muted())),
    };
    let head = row().w_full().justify_between().items_center().gap(20.0).flex_wrap()
        .child(row().shrink().gap(16.0).center_y()
            .child(row().size(96.0, 96.0).radius(16.0).center().fill(th::glass()).border(1.0, th::border_base()).child(rune::el().size(80.0, 80.0).pixels_fit(&s.logo, rune::SurfaceFit::Fit)))
            .child(col().shrink().gap(6.0)
                .child(row().gap(8.0).center_y().child(rune::el().size(6.0, 6.0).radius(3.0).fill(th::blue())).child(text("SISTEMA OPERATIVO HIVE").font(mono(10.0, 2.0)).color(th::blue())))
                .child(row().w_full().gap(10.0).flex_wrap().child(text("Centro de").font(TextStyle::new(32.0, 800.0)).color(th::foreground())).child(text("Control").font(TextStyle::new(32.0, 800.0).italic()).color(th::blue())))
                .child(text("Monitoreo en tiempo real del enjambre y estado de los nodos cognitivos.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()))))
        .child(row().gap(22.0).center_y()
            .child(col().gap(5.0).items_end().child(label("Estado Hive")).child(row().gap(7.0).center_y().child(rune::el().size(7.0, 7.0).radius(3.5).fill(if online { th::blue() } else { th::red() })).child(text(if online { "EN LÍNEA" } else { "DESCONECTADO" }).font(mono(10.0, 1.0)).color(if online { th::blue() } else { th::red() }))))
            .child(col().w(1.0).h(40.0).fill(th::border_subtle()))
            .child(col().gap(5.0).items_end().child(label("Versión")).child(version)));

    let cards = grid()
        .child(stat_card(
            "Agentes activos",
            agents.to_string(),
            &icons::BOT,
            th::blue(),
            &format!("{working} trabajando"),
        ))
        .child(stat_card(
            "Sesiones canvas",
            stats.connections.to_string(),
            &icons::LAYERS,
            th::amber(),
            "Live",
        ))
        .child(stat_card(
            "Mensajes / 1h",
            last_hour.to_string(),
            &icons::ACTIVITY,
            th::green(),
            "Última hora",
        ));

    // Monitor del proceso.
    let monitor = cardx()
        .key("dash/monitor")
        .w_full()
        .padding(20.0)
        .gap(16.0)
        .child(
            row()
                .gap(10.0)
                .center_y()
                .child(rune::icon(&icons::ACTIVITY, 16.0).color(th::amber()))
                .child(
                    text("Monitor del Proceso Hive")
                        .font(TextStyle::new(15.0, 600.0))
                        .color(th::foreground()),
                ),
        )
        .child(
            grid()
                .child(metric(
                    &icons::ZAP,
                    "CPU del proceso",
                    if measured {
                        format!("{:.2}%", stats.cpu)
                    } else {
                        "—".into()
                    },
                    th::green(),
                    (stats.cpu / 100.0) as f32,
                    format!("{} núcleos disponibles", stats.cores),
                ))
                .child(metric(
                    &icons::CHAT,
                    "Actividad reciente",
                    stats.recent_messages.to_string(),
                    th::green(),
                    stats.recent_messages as f32 / 20.0,
                    "mensajes últimos 5 min".into(),
                ))
                .child(metric(
                    &icons::HEXAGON,
                    "Memoria Hive (RSS)",
                    if measured {
                        format!("{} MB", stats.rss.round())
                    } else {
                        "—".into()
                    },
                    th::green(),
                    (stats.rss / 1024.0) as f32,
                    if rust {
                        "Proceso del gateway Rust".into()
                    } else {
                        format!("Heap JS: {} MB usados", stats.heap_used.round())
                    },
                ))
                .child(if rust {
                    metric(
                        &icons::CPU,
                        "Turnos en ejecución",
                        stats.active_runs.to_string(),
                        th::blue(),
                        0.0,
                        "Solicitudes de chat activas".into(),
                    )
                    .key("dash/native-runs")
                } else {
                    metric(
                        &icons::CPU,
                        "Heap JS",
                        format!("{} MB", stats.heap_used.round()),
                        th::blue(),
                        (stats.heap_used / (stats.heap_used + stats.external).max(1.0)) as f32,
                        format!("External: {} MB", stats.external.round()),
                    )
                })
                .child(metric(
                    &icons::NETWORK,
                    "Sesiones WS activas",
                    stats.connections.to_string(),
                    th::purple(),
                    stats.connections as f32 / 10.0,
                    "conexiones WebSocket".into(),
                )),
        );

    // Uso y costos de API (últimas 24 h).
    let usage = d.usage.clone().unwrap_or_default();
    let mut providers = col().w_full().gap(8.0);
    for (name, tokens, cost) in usage.providers.iter().take(6) {
        providers = providers.child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    text(name.to_uppercase())
                        .font(mono(10.0, 0.8))
                        .color(th::muted_foreground())
                        .w(110.0),
                )
                .child(col().grow(1.0).child(meter(
                    *tokens as f32 / usage.tokens.max(1) as f32,
                    th::blue(),
                )))
                .child(
                    text(fmt_tokens(*tokens))
                        .font(TextStyle::new(11.5, 600.0).mono())
                        .color(th::foreground())
                        .w(56.0),
                )
                .child(
                    text(money(*cost))
                        .font(TextStyle::new(11.5, 600.0).mono())
                        .color(th::amber())
                        .w(60.0),
                ),
        );
    }
    let tile = |name: &str, value: String, tone: Color| {
        col()
            .grow(1.0)
            .w(120.0)
            .px(14.0)
            .py(10.0)
            .gap(4.0)
            .child(label(name))
            .child(
                text(value)
                    .font(TextStyle::new(17.0, 700.0).mono())
                    .color(tone),
            )
    };
    let usage_card = cardx()
        .key("dash/usage")
        .w_full()
        .padding(20.0)
        .gap(14.0)
        .child(
            row().w_full().justify_between().center_y().child(
                row()
                    .gap(10.0)
                    .center_y()
                    .child(
                        text("$")
                            .font(TextStyle::new(16.0, 800.0))
                            .color(th::amber()),
                    )
                    .child(
                        col()
                            .gap(1.0)
                            .child(
                                text("Uso y Costos de API")
                                    .font(TextStyle::new(15.0, 600.0))
                                    .color(th::foreground()),
                            )
                            .child(
                                text("Tokens consumidos y costos por provider · últimas 24 h")
                                    .font(TextStyle::new(11.0, 400.0))
                                    .color(th::text_muted()),
                            ),
                    ),
            ),
        )
        .child(
            row()
                .w_full()
                .gap(0.0)
                .flex_wrap()
                .child(tile(
                    "Tokens totales",
                    fmt_tokens(usage.tokens),
                    th::foreground(),
                ))
                .child(tile("Entrada", fmt_tokens(usage.input), th::foreground()))
                .child(tile("Salida", fmt_tokens(usage.output), th::foreground()))
                .child(tile("Costo estimado", money(usage.cost), th::amber()))
                .child(tile(
                    "Ahorro TOON",
                    fmt_tokens(usage.toon_saved_tokens),
                    th::green(),
                )),
        )
        .child(providers)
        .children(rust.then(|| {
            text("Consumo confirmado; las solicitudes interrumpidas pueden no estar incluidas.")
                .font(type_::LABEL)
                .color(th::text_muted())
                .wrap()
        }))
        .children(usage.cost_incomplete.then(|| {
            text("Costo incompleto: faltan tarifas o mediciones de uso.")
                .font(type_::LABEL)
                .color(th::amber())
                .wrap()
        }))
        .children(d.usage.as_ref().is_some_and(|u| u.tokens == 0).then(|| {
            text("Aún no hay consumo registrado en este periodo.")
                .font(type_::LABEL)
                .color(th::muted_foreground())
        }));

    // Gráficos: actividad del enjambre y su salud.
    let total = idle + working + stuck + disabled;
    let activity = cardx()
        .key("dash/activity")
        .w(520.0_f32.min(s.content_width() - 56.0))
        .min_w(420.0_f32.min(s.content_width() - 56.0))
        .grow(2.0)
        .padding(20.0)
        .gap(14.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    col()
                        .gap(3.0)
                        .child(
                            text("ACTIVIDAD DEL ENJAMBRE")
                                .font(mono(11.5, 2.0))
                                .color(th::blue()),
                        )
                        .child(
                            text(
                                format!("Últimas 12 horas · {total_msgs} mensajes").to_uppercase(),
                            )
                            .font(mono(9.5, 1.2))
                            .color(th::text_muted()),
                        ),
                )
                .child(rune::icon(&icons::ACTIVITY, 18.0).color(th::blue())),
        )
        .child(activity_chart(&d.activity));
    let attention = d.proposals.len();
    let mut health = cardx()
        .key("dash/health")
        .w(300.0)
        .min_w(260.0)
        .grow(1.0)
        .padding(20.0)
        .gap(12.0)
        .child(
            text("SALUD DEL ENJAMBRE")
                .font(mono(11.5, 2.0))
                .color(th::blue()),
        )
        .child(health_bar("Idle", idle, total, th::muted_foreground()))
        .child(health_bar("Trabajando", working, total, th::blue()))
        .children((stuck > 0).then(|| health_bar("Atascado", stuck, total, th::red())))
        .children(
            (disabled > 0)
                .then(|| health_bar("Deshabilitado", disabled, total, th::muted_foreground())),
        )
        .child(col().w_full().h(1.0).fill(th::border_subtle()))
        .child(
            row()
                .w_full()
                .gap(16.0)
                .child(
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(label("Necesitan atención"))
                        .child(
                            text(attention.to_string())
                                .font(TextStyle::new(20.0, 700.0).mono())
                                .color(if attention > 0 { th::red() } else { th::blue() }),
                        ),
                )
                .child(
                    col().grow(1.0).gap(4.0).child(label("Uptime")).child(
                        text(if stats.uptime.is_empty() {
                            "—".to_string()
                        } else {
                            stats.uptime.clone()
                        })
                        .font(TextStyle::new(20.0, 700.0).mono())
                        .color(th::blue()),
                    ),
                ),
        );
    for p in d.proposals.iter().take(4) {
        health = health.child(
            row()
                .gap(6.0)
                .child(text("●").font(TextStyle::new(10.0, 700.0)).color(th::red()))
                .child(
                    text(format!("{} — {}", p.agent, describe_reason(&p.reason)))
                        .font(TextStyle::new(10.5, 400.0))
                        .color(th::muted_foreground())
                        .wrap()
                        .grow(1.0),
                ),
        );
    }
    let charts = row()
        .w_full()
        .gap(16.0)
        .flex_wrap()
        .child(activity)
        .child(health);

    page(
        col()
            .w_full()
            .gap(22.0)
            .child(head)
            .child(banner(s))
            .child(cards)
            .child(monitor)
            .child(usage_card)
            .child(charts),
    )
}

// ─── Ética ───────────────────────────────────────────────────────────────────

pub fn ethics(s: &State) -> El {
    let list = &s.sys.shown.ethics;
    let mut g = grid();
    for e in list {
        let open = s.ethic_open.as_deref() == Some(e.id.as_str());
        let id = e.id.clone();
        let mut c = cardx()
            .key(format!("ethic/{}", e.id))
            .w(360.0)
            .min_w(300.0)
            .padding(20.0)
            .gap(12.0)
            .border(
                1.0,
                if e.active {
                    th::blue().alpha(0.3)
                } else {
                    th::border_subtle()
                },
            )
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .items_start()
                    .child(icon_box(&icons::SHIELD, th::blue(), 46.0))
                    .child(
                        row()
                            .gap(6.0)
                            .center_y()
                            .child(
                                text(if e.active { "ACTIVO" } else { "INACTIVO" })
                                    .font(mono(9.0, 1.0))
                                    .color(if e.active {
                                        th::blue()
                                    } else {
                                        th::text_muted()
                                    }),
                            )
                            .child(rune::el().size(6.0, 6.0).radius(3.0).fill(if e.active {
                                th::blue()
                            } else {
                                th::muted()
                            })),
                    ),
            )
            .child(
                text(e.name.to_uppercase())
                    .font(TextStyle::new(17.0, 800.0))
                    .color(th::foreground())
                    .wrap(),
            )
            .child(
                text(short(&e.description, 80))
                    .font(TextStyle::new(12.0, 400.0))
                    .color(th::muted_foreground())
                    .wrap(),
            )
            .child(col().w_full().h(1.0).fill(th::border_subtle()))
            .child(
                row()
                    .gap(22.0)
                    .child(
                        row()
                            .gap(6.0)
                            .items_end()
                            .child(text("ID").font(mono(8.5, 1.2)).color(th::text_muted()))
                            .child(
                                text(e.id.clone())
                                    .font(TextStyle::new(12.0, 600.0).mono())
                                    .color(th::foreground()),
                            ),
                    )
                    .child(
                        row()
                            .gap(6.0)
                            .items_end()
                            .child(
                                text("PRECEDENCIA")
                                    .font(mono(8.5, 1.2))
                                    .color(th::text_muted()),
                            )
                            .child(
                                text(if e.active { "ALTA" } else { "—" })
                                    .font(TextStyle::new(12.0, 600.0))
                                    .color(th::foreground()),
                            ),
                    ),
            )
            .tap(move |s: &mut State| {
                s.ethic_open = if s.ethic_open.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id.clone())
                }
            });
        if open {
            c = c.child(
                col()
                    .w_full()
                    .padding(12.0)
                    .radius(8.0)
                    .fill(th::glass())
                    .border(1.0, th::border_subtle())
                    .child(
                        text(short(&e.content, 1800))
                            .font(TextStyle::new(11.0, 400.0).mono().lh(17.0))
                            .color(th::foreground())
                            .wrap(),
                    ),
            );
        }
        g = g.child(c);
    }
    page(col().w_full().gap(20.0)
        .child(header("Protocolos de conducta", "Ética", "& Alineación", "Configura los límites morales y el comportamiento base de tus agentes.", Some(crate::settings::view::refresh_button(s, "Actualizar"))))
        .child(banner(s))
        .child(row().w_full().justify_between().center_y().child(row().gap(10.0).center_y().child(text("PROTOCOLOS ÉTICOS").font(TextStyle::new(15.0, 800.0).tracking(2.4)).color(th::blue())).child(chip(&format!("{} registros", list.len()), th::blue()))))
        .child(text("Define la capa moral y los límites operacionales de tus agentes inteligentes. Pulsa uno para leer sus lineamientos.").font(type_::LABEL).color(th::muted_foreground()))
        .children((list.is_empty() && s.sys.shown.ethics_loaded).then(|| text("No hay protocolos éticos").font(type_::LABEL).color(th::muted_foreground())))
        .child(g))
}

// ─── Perfil ──────────────────────────────────────────────────────────────────

fn hex_avatar(initials: &str) -> El {
    let (w, h) = (84.0f32, 74.0f32);
    let pts = vec![
        [w * 0.25, 1.0],
        [w * 0.75, 1.0],
        [w - 1.0, h / 2.0],
        [w * 0.75, h - 1.0],
        [w * 0.25, h - 1.0],
        [1.0, h / 2.0],
    ];
    col()
        .size(w, h)
        .center()
        .shape(Shape::new().fill(pts, th::amber()))
        .child(
            text(initials.to_string())
                .font(TextStyle::new(24.0, 800.0))
                .color(th::on_amber()),
        )
}

fn section_title(t: &str) -> El {
    row()
        .gap(8.0)
        .center_y()
        .child(
            text("⬢")
                .font(TextStyle::new(10.0, 700.0))
                .color(th::amber()),
        )
        .child(
            text(t.to_uppercase())
                .font(mono(11.0, 2.0))
                .color(th::amber()),
        )
}

fn profile_field(
    editing: bool,
    name: &str,
    icon: &'static rune::Icon,
    edit: &TextEdit,
    key: &str,
    pick: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
) -> El {
    let shown = if editing {
        field(edit, key.to_string(), "", pick)
    } else {
        row()
            .w_full()
            .px(12.0)
            .py(10.0)
            .radius(radius::SM)
            .fill(th::glass_strong())
            .border(1.0, th::border_subtle())
            .child(
                text(if edit.text.is_empty() {
                    "—".to_string()
                } else {
                    edit.text.clone()
                })
                .font(type_::BODY)
                .color(th::muted_foreground()),
            )
    };
    col()
        .grow(1.0)
        .w(0.0)
        .gap(5.0)
        .child(
            row()
                .gap(6.0)
                .center_y()
                .child(rune::icon(icon, 11.0).color(th::text_muted()))
                .child(label(name)),
        )
        .child(shown)
}

pub fn profile(s: &State) -> El {
    let f = &s.profile;
    let u = s.sys.shown.user.clone().unwrap_or_default();
    let ed = f.editing;
    let left = col()
        .w(264.0)
        .min_w(240.0)
        .padding(22.0)
        .gap(10.0)
        .items_center()
        .border(1.0, th::border_subtle())
        .child(
            row()
                .w_full()
                .justify_between()
                .child(
                    row()
                        .gap(6.0)
                        .center_y()
                        .child(rune::el().size(5.0, 5.0).radius(2.5).fill(th::amber()))
                        .child(text("HIVE OS").font(mono(9.0, 1.6)).color(th::amber())),
                )
                .child(text("V1.0").font(mono(9.0, 1.0)).color(th::text_muted())),
        )
        .child(col().h(18.0))
        .child(hex_avatar(&u.initials()))
        .child(
            text(if u.name.is_empty() {
                "Sin nombre".to_string()
            } else {
                u.name.clone()
            })
            .font(TextStyle::new(17.0, 700.0))
            .color(th::foreground()),
        )
        .child(
            text(if u.occupation.is_empty() {
                "—".to_string()
            } else {
                u.occupation.clone()
            })
            .font(type_::BODY)
            .color(th::muted_foreground())
            .wrap(),
        )
        .child(
            row()
                .gap(6.0)
                .flex_wrap()
                .child(chip(
                    if u.language.is_empty() {
                        "—"
                    } else {
                        &u.language
                    },
                    th::blue(),
                ))
                .child(chip(
                    if u.timezone.is_empty() {
                        "—"
                    } else {
                        &u.timezone
                    },
                    th::blue(),
                )),
        );
    let access = row()
        .w_full()
        .px(14.0)
        .py(12.0)
        .gap(12.0)
        .center_y()
        .radius(12.0)
        .fill(th::glass())
        .border(1.0, th::border_base())
        .child(icon_box(&icons::SHIELD, th::amber(), 36.0))
        .child(
            col()
                .gap(2.0)
                .grow(1.0)
                .child(
                    text("Acceso con contraseña")
                        .font(TextStyle::new(13.0, 700.0))
                        .color(th::foreground()),
                )
                .child(
                    text(if s.sys.shown.has_credentials {
                        format!("Protegido con contraseña · {}", u.email)
                    } else {
                        format!("Sin contraseña · {}", u.email)
                    })
                    .font(TextStyle::new(11.0, 400.0))
                    .color(th::muted_foreground())
                    .wrap(),
                ),
        );
    let edit_btn = row()
        .key("profile/edit")
        .gap(7.0)
        .center_y()
        .px(14.0)
        .py(7.0)
        .radius(9.0)
        .fill(th::glass_strong())
        .border(1.0, th::border_base())
        .child(rune::icon(&icons::PENCIL, 12.0).color(th::muted_foreground()))
        .child(
            text(if ed { "Editando" } else { "Editar" })
                .font(TextStyle::new(12.0, 600.0))
                .color(th::foreground()),
        )
        .tap(|s: &mut State| {
            s.profile.editing = !s.profile.editing;
            if !s.profile.editing {
                let u = s.sys.shown.user.clone();
                s.profile.filled_for = None;
                s.profile.follow(u.as_ref());
            }
        });
    let right = col()
        .grow(1.0)
        .min_w(0.0)
        .child(
            row()
                .w_full()
                .px(20.0)
                .py(14.0)
                .gap(12.0)
                .flex_wrap()
                .justify_between()
                .center_y()
                .border(1.0, th::border_subtle())
                .child(
                    row()
                        .shrink()
                        .grow(1.0)
                        .gap(12.0)
                        .center_y()
                        .child(icon_box(&icons::BOT, th::amber(), 34.0))
                        .child(
                            col()
                                .shrink()
                                .gap(2.0)
                                .child(
                                    text("PERFIL DE USUARIO")
                                        .font(mono(12.0, 2.0))
                                        .color(th::amber()),
                                )
                                .child(
                                    text("Configuración personal y preferencias del agente")
                                        .font(TextStyle::new(10.5, 400.0))
                                        .color(th::text_muted()),
                                ),
                        ),
                )
                .child(edit_btn),
        )
        .child(
            col()
                .w_full()
                .padding(20.0)
                .gap(16.0)
                .child(section_title("Identidad"))
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(profile_field(
                            ed,
                            "Nombre completo",
                            &icons::BOT,
                            &f.name,
                            "profile/name",
                            |s: &mut State| &mut s.profile.name,
                        ))
                        .child(profile_field(
                            ed,
                            "Ocupación",
                            &icons::FILE,
                            &f.occupation,
                            "profile/occupation",
                            |s: &mut State| &mut s.profile.occupation,
                        )),
                )
                .child(row().w_full().child(profile_field(
                    ed,
                    "Correo propio",
                    &icons::SEND,
                    &f.email,
                    "profile/email",
                    |s: &mut State| &mut s.profile.email,
                )))
                .child(
                    text("El coordinador lo usa como destinatario cuando dices «envíame».")
                        .font(TextStyle::new(10.5, 400.0))
                        .color(th::text_muted()),
                )
                .child(section_title("Acceso y seguridad"))
                .child(access)
                .child(section_title("Localización"))
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(profile_field(
                            ed,
                            "Idioma",
                            &icons::GLOBE,
                            &f.language,
                            "profile/language",
                            |s: &mut State| &mut s.profile.language,
                        ))
                        .child(profile_field(
                            ed,
                            "Zona horaria",
                            &icons::CLOCK,
                            &f.timezone,
                            "profile/timezone",
                            |s: &mut State| &mut s.profile.timezone,
                        )),
                )
                .child(section_title("Notas para el agente"))
                .child(row().w_full().child(profile_field(
                    ed,
                    "Preferencias",
                    &icons::FILE_TEXT,
                    &f.notes,
                    "profile/notes",
                    |s: &mut State| &mut s.profile.notes,
                )))
                .children(ed.then(|| {
                    row()
                        .gap(10.0)
                        .child(
                            row()
                                .key("profile/save")
                                .px(18.0)
                                .py(10.0)
                                .radius(10.0)
                                .fill(th::amber())
                                .center()
                                .child(
                                    text("Guardar")
                                        .font(TextStyle::new(13.0, 700.0))
                                        .color(th::on_amber()),
                                )
                                .tap(|s: &mut State| save_profile(s)),
                        )
                        .child(
                            row()
                                .key("profile/cancel")
                                .px(18.0)
                                .py(10.0)
                                .radius(10.0)
                                .fill(th::glass_strong())
                                .border(1.0, th::border_base())
                                .center()
                                .child(
                                    text("Cancelar")
                                        .font(TextStyle::new(13.0, 500.0))
                                        .color(th::foreground()),
                                )
                                .tap(|s: &mut State| {
                                    s.profile.editing = false;
                                    let u = s.sys.shown.user.clone();
                                    s.profile.filled_for = None;
                                    s.profile.follow(u.as_ref());
                                }),
                        )
                })),
        );
    let card = cardx()
        .key("profile/card")
        .w_full()
        .clip_rounded(16.0)
        .child(if s.content_width() < 850.0 {
            col()
                .w_full()
                .child(left.w_full().min_w(0.0))
                .child(right.w_full())
        } else {
            row().w_full().child(left).child(right)
        });
    page(
        col()
            .w_full()
            .gap(20.0)
            .child(header(
                "Identidad del operador",
                "Perfil",
                "de Usuario",
                "Gestiona tu información personal y preferencias de la interfaz.",
                None,
            ))
            .child(banner(s))
            .child(card),
    )
}

/// Valida el formulario y manda el cambio al servidor.
pub fn save_profile(s: &mut State) {
    let f = &s.profile;
    match crate::system::profile_patch(
        &f.name.text,
        &f.occupation.text,
        &f.email.text,
        &f.language.text,
        &f.timezone.text,
        &f.notes.text,
    ) {
        Err(e) => s.sys.data.lock().unwrap().error = Some(e.to_string()),
        Ok(patch) => {
            let (Some(conn), Some(user)) = (s.settings.conn.clone(), s.sys.shown.user.clone())
            else {
                s.sys.data.lock().unwrap().error =
                    Some("El perfil aún no se ha leído del servidor".into());
                return;
            };
            {
                let mut d = s.sys.data.lock().unwrap();
                d.error = None;
                d.notice = None;
            }
            s.sys.save_profile(&conn, &user.id, patch);
            s.profile.editing = false;
            s.profile.filled_for = None; // al llegar el usuario guardado se vuelve a rellenar
        }
    }
}

// ─── Pantalla y audio ────────────────────────────────────────────────────────

/// Un selector desplegable de aparatos: la opción «predeterminado» y una por aparato.
fn picker(
    a: &AudioUi,
    key: &'static str,
    which: u8,
    shown: String,
    options: Vec<(Option<String>, String)>,
    choose: fn(&mut State, Option<String>),
) -> El {
    let mut c = col().w_full().gap(6.0).child(
        row()
            .key(key)
            .w_full()
            .px(12.0)
            .py(10.0)
            .radius(radius::SM)
            .fill(th::glass_strong())
            .border(1.0, th::border_base())
            .center_y()
            .child(
                text(shown)
                    .font(type_::BODY)
                    .color(th::foreground())
                    .grow(1.0),
            )
            .child(rune::icon(&icons::CHEVRON_DOWN, 14.0).color(th::muted_foreground()))
            .tap(move |s: &mut State| s.audio.open = if s.audio.open == which { 0 } else { which }),
    );
    if a.open == which {
        let mut l = col()
            .w_full()
            .radius(radius::SM)
            .fill(th::card())
            .border(1.0, th::border_base())
            .py(4.0);
        for (i, (id, name)) in options.into_iter().enumerate() {
            l = l.child(
                row()
                    .key(format!("{key}/{i}"))
                    .w_full()
                    .px(12.0)
                    .py(8.0)
                    .child(text(name).font(type_::BODY).color(th::foreground()))
                    .tap(move |s: &mut State| choose(s, id.clone())),
            );
        }
        c = c.child(l);
    }
    c
}

fn device_picker(a: &AudioUi, input: bool) -> El {
    let (list, current, which, key): (&Vec<AudioDevice>, &Option<String>, u8, &'static str) =
        if input {
            (&a.inputs, &a.prefs.input, 1, "audio/in")
        } else {
            (&a.outputs, &a.prefs.output, 2, "audio/out")
        };
    let shown = match current {
        Some(id) => list
            .iter()
            .find(|d| &d.id == id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "Aparato no disponible (se usa el del sistema)".into()),
        None => "Predeterminado del sistema".into(),
    };
    if list.is_empty() {
        return text(if input {
            "No se detectó ningún micrófono."
        } else {
            "No se detectó ninguna salida de audio."
        })
        .font(type_::BODY)
        .color(th::muted_foreground());
    }
    let mut options: Vec<(Option<String>, String)> =
        vec![(None, "Predeterminado del sistema".into())];
    options.extend(list.iter().map(|d| {
        (
            Some(d.id.clone()),
            if d.is_default {
                format!("{} (predeterminado)", d.name)
            } else {
                d.name.clone()
            },
        )
    }));
    let choose: fn(&mut State, Option<String>) = if input {
        |s, id| s.audio.choose(true, id)
    } else {
        |s, id| s.audio.choose(false, id)
    };
    picker(a, key, which, shown, options, choose)
}

fn camera_picker(a: &AudioUi) -> El {
    if a.cameras.is_empty() {
        return text("No se detectó ninguna cámara.")
            .font(type_::BODY)
            .color(th::muted_foreground());
    }
    let shown = match &a.prefs.camera {
        Some(id) => a
            .cameras
            .iter()
            .find(|c| &c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Cámara no disponible (se usa la primera)".into()),
        None => "Primera cámara disponible".into(),
    };
    let mut options: Vec<(Option<String>, String)> =
        vec![(None, "Primera cámara disponible".into())];
    options.extend(
        a.cameras
            .iter()
            .map(|c| (Some(c.id.clone()), c.name.clone())),
    );
    picker(a, "audio/cam", 3, shown, options, |s, id| {
        s.audio.choose_camera(id)
    })
}

pub fn display(s: &State) -> El {
    let a = &s.audio;
    let testing = a.mic_test.is_some();
    let mic = cardx().key("audio/mic").w_full().padding(22.0).gap(14.0)
        .child(row().gap(10.0).center_y().child(rune::icon(&icons::MIC, 16.0).color(th::muted_foreground())).child(text("Micrófono").font(TextStyle::new(16.0, 600.0)).color(th::foreground())))
        .child(text("Por dónde te oye la colmena en HiveLive. Si hay una llamada en curso, el cambio se aplica al momento.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()).wrap())
        .child(device_picker(a, true))
        .child(meter(a.mic_peak.unwrap_or(0.0), if a.mic_peak.unwrap_or(0.0) > 0.05 { th::green() } else { th::muted_foreground() }))
        .child(row().gap(12.0).center_y()
            .child(row().key("audio/mic-test").gap(8.0).center_y().px(16.0).py(9.0).radius(9.0).border(1.0, th::border_base()).fill(th::glass())
                .child(rune::icon(&icons::MIC, 13.0).color(th::foreground())).child(text(if testing { "Escuchando…" } else { "Probar micrófono" }).font(TextStyle::new(13.0, 500.0)).color(th::foreground()))
                .tap(|s: &mut State| if s.audio.mic_test.is_none() { s.audio.start_mic_test() }))
            .children(a.mic_peak.filter(|p| !testing && *p <= 0.02).map(|_| text("No llega señal de este micrófono. Comprueba que esté conectado y sin silenciar, o elige otro.").font(TextStyle::new(11.5, 400.0)).color(th::red()).wrap().grow(1.0)))
            .children(a.mic_peak.filter(|p| !testing && *p > 0.02).map(|_| text("Se oye bien.").font(TextStyle::new(11.5, 500.0)).color(th::green()))))
        .children(a.message.clone().map(|m| text(m).font(type_::LABEL).color(th::red()).wrap()));
    let out = cardx().key("audio/out").w_full().padding(22.0).gap(14.0)
        .child(row().gap(10.0).center_y().child(rune::icon(&icons::VOLUME_X, 16.0).color(th::muted_foreground())).child(text("Salida de la voz en vivo").font(TextStyle::new(16.0, 600.0)).color(th::foreground())))
        .child(text("Por dónde se oye a la colmena en HiveLive. La lista la reporta el sistema operativo; el cambio se aplica al momento si hay una llamada.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()).wrap())
        .child(device_picker(a, false))
        .child(row().key("audio/test").gap(8.0).center_y().px(16.0).py(9.0).radius(9.0).border(1.0, th::border_base()).fill(th::glass())
            .child(text("▷").font(TextStyle::new(13.0, 700.0)).color(th::foreground())).child(text("Probar sonido").font(TextStyle::new(13.0, 500.0)).color(th::foreground()))
            .tap(|s: &mut State| s.audio.play_test()));
    let cam = cardx().key("audio/camera").w_full().padding(22.0).gap(14.0)
        .child(row().gap(10.0).center_y().child(rune::icon(&icons::VIDEO, 16.0).color(th::muted_foreground())).child(text("Cámara").font(TextStyle::new(16.0, 600.0)).color(th::foreground())))
        .child(text("Qué cámara ve la colmena cuando la compartes en HiveLive. Solo aparecen las que entregan imagen; si ya la tienes encendida, el cambio se aplica al momento.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()).wrap())
        .child(camera_picker(a));
    let dark = !th::is_light();
    let theme = cardx().key("audio/theme").w_full().padding(22.0).gap(14.0)
        .child(row().gap(10.0).center_y().child(rune::icon(if dark { &icons::MOON } else { &icons::SUN }, 16.0).color(th::muted_foreground())).child(text("Tema").font(TextStyle::new(16.0, 600.0)).color(th::foreground())))
        .child(text("Oscuro, el de siempre, o crema, más claro y con contraste. También se cambia con el botón de la cabecera.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()).wrap())
        .child(row().gap(8.0)
            .child(pill("Oscuro".into(), "theme/dark".into(), dark, |_: &mut State| { th::set_light(false); th::save(); }))
            .child(pill("Crema".into(), "theme/light".into(), !dark, |_: &mut State| { th::set_light(true); th::save(); })));
    page(
        col()
            .w_full()
            .gap(20.0)
            .child(header(
                "Apariencia y sonido",
                "Pantalla",
                "& Audio",
                "Elige el tema, el micrófono, la cámara y por dónde se oye la voz en vivo.",
                None,
            ))
            .child(theme)
            .child(mic)
            .child(out)
            .child(cam),
    )
}

// ─── Actualizaciones ─────────────────────────────────────────────────────────

pub fn updates(s: &State) -> El {
    let v = s.sys.shown.version.clone();
    let mono_line = |t: String| {
        text(t)
            .font(TextStyle::new(13.0, 400.0).mono())
            .color(th::muted_foreground())
    };
    let status = match &v {
        Some(v) if v.up_to_date() => text("Estás al día.")
            .font(TextStyle::new(13.0, 600.0))
            .color(th::green()),
        Some(v) if v.status == "update-available" && !v.latest.is_empty() => {
            text(format!("Hay una versión nueva del servidor: {}.", v.latest))
                .font(TextStyle::new(13.0, 600.0))
                .color(th::amber())
        }
        Some(_) => text("No se ha comprobado si hay una versión nueva.")
            .font(type_::BODY)
            .color(th::muted_foreground()),
        None => text("Comprobando…")
            .font(type_::BODY)
            .color(th::muted_foreground()),
    };
    let server = cardx().key("update/server").w_full().padding(22.0).gap(10.0)
        .child(text("Servidor instalado").font(TextStyle::new(16.0, 600.0)).color(th::foreground()))
        .child(mono_line(match &v { Some(v) => format!("Hive {} · {}", v.current, v.installation), None => "—".into() }))
        .child(status)
        .child(text("El servidor (hive-gateway-rust) lleva su propia versión: va empaquetado junto a esta app y se actualiza al instalar una versión nueva del paquete.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()).wrap());
    let app = cardx().key("update/app").w_full().padding(22.0).gap(10.0)
        .child(text("Esta app").font(TextStyle::new(16.0, 600.0)).color(th::foreground()))
        .child(mono_line(format!("Hive Agents (Rune) {}", env!("CARGO_PKG_VERSION"))))
        .child(text("Se instala desde el paquete .tar.gz (packaging/linux/install.sh), sin administrador. Para actualizar, instala el paquete nuevo encima: no se toca ~/.hive.").font(type_::BODY.lh(20.0)).color(th::muted_foreground()).wrap());
    page(
        col()
            .w_full()
            .gap(20.0)
            .child(header(
                "Mantenimiento",
                "Actualizaciones",
                "",
                "Consulta tu versión y cómo instalar una nueva.",
                Some(crate::settings::view::refresh_button(
                    s,
                    "Actualizar información",
                )),
            ))
            .child(banner(s))
            .child(server)
            .child(app),
    )
}
