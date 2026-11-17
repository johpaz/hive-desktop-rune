//! HiveLive: la voz de Hive. Un escenario a pantalla completa (panal, BIA y polen, dibujado en la GPU) con la
//! interfaz flotando encima: cabecera, telemetría real de la sesión, mando de llamada y transcripción.
//! Mismo diseño que `VoiceConsole.tsx` de la web.

use crate::ui::text;
use crate::ui::Tap;
use rune::tok::{radius, type_, TextStyle};
use rune::{col, row, El};

use super::icons;
use super::theme::vx;
use crate::live::stage::{LIVE_MARK, SLOT_KEY};
use crate::live::state::{short_model, Bia, LANGS, VOICES};
use crate::{Screen, State};

const PANEL_W: f32 = 336.0;
fn head_height(w: f32) -> f32 {
    if w < 600.0 {
        112.0
    } else {
        72.0
    }
}

fn mono(size: f32, tracking: f32) -> TextStyle {
    TextStyle::new(size, 500.0).mono().tracking(tracking)
}

/// Cristal de la capa flotante: tinta del escenario, casi opaca, con borde tenue.
fn glass() -> El {
    col()
        .fill(vx::ink().alpha(0.78))
        .border(1.0, vx::faint())
        .radius(14.0)
}

fn label(s: &str) -> El {
    text(s.to_uppercase()).font(mono(9.5, 1.9)).color(vx::dim())
}

/// Por debajo de este ancho de escenario la telemetría arranca plegada y el texto de ayuda se oculta.
const NARROW_STAGE: f32 = 960.0;

pub fn view(s: &State) -> El {
    let (w, h) = (s.live.viewport.0.max(240.0), s.live.viewport.1.max(480.0));
    let mut overlay = vec![
        header(s, w),
        telemetry(s, w, h),
        transcript(s, w, h),
        dock(s, w, h),
        hint(w, h),
        view_controls(s, h),
    ];
    if let Some(error) = &s.live.error {
        overlay.push(
            glass()
                .key("live/error")
                .absolute(16.0, head_height(w) + 16.0)
                .w((w - 32.0).min(600.0))
                .padding(14.0)
                .child(
                    text(error.clone())
                        .font(type_::LABEL)
                        .color(crate::ui::theme::red())
                        .wrap(),
                ),
        );
    }
    if s.live.scene_loading || s.live.scene_error.is_some() {
        let message = s.live.scene_error.as_deref().unwrap_or(
            "Preparando el escenario 3D de BIA… Los controles de voz ya están disponibles.",
        );
        let mut loading = glass()
            .key("live/scene-loading")
            .absolute(76.0, head_height(w) + 76.0)
            .w((w - 108.0).min(420.0))
            .padding(12.0)
            .gap(8.0)
            .child(text(message).font(type_::LABEL).color(vx::fg(0.9)));
        if s.live.scene_error.is_some() {
            loading = loading.child(
                row()
                    .key("live/scene-retry")
                    .py(6.0)
                    .child(
                        text("Reintentar cargar el escenario")
                            .font(type_::LABEL)
                            .color(vx::gold()),
                    )
                    .tap(|s: &mut State| {
                        s.live.scene_retry = s.live.scene_retry.wrapping_add(1);
                        s.live.scene_error = None;
                    }),
            );
        }
        overlay.push(loading);
    }
    let pw = (if w < NARROW_STAGE {
        156.0_f32
    } else {
        224.0_f32
    })
    .min(((w - 102.0) / 2.0).max(60.0));
    for (i, (name, key, preview, screen)) in [
        (
            "Cámara",
            "live/camera-preview",
            &s.live.camera_preview,
            false,
        ),
        (
            "Pantalla compartida",
            "live/screen-preview",
            &s.live.screen_preview,
            true,
        ),
    ]
    .into_iter()
    .filter(|(_, _, p, _)| p.active)
    .enumerate()
    {
        let mut card = glass()
            .key(key)
            .absolute(76.0 + i as f32 * (pw + 10.0), head_height(w) + 76.0)
            .w(pw)
            .padding(8.0)
            .gap(5.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .child(
                        text(name)
                            .font(mono(10.0, 0.0))
                            .color(vx::fg(0.9))
                            .grow(1.0),
                    )
                    .child(
                        row()
                            .key(format!("{key}/stop"))
                            .size(24.0, 24.0)
                            .center()
                            .child(text("×").color(vx::gold()))
                            .tap(move |s: &mut State| crate::toggle_media(s, screen)),
                    ),
            );
        card = card.child(if preview.has_image {
            rune::el()
                .key(format!("{key}/image"))
                .w_full()
                .h((pw * 0.5625).min(if h < 560.0 { 60.0 } else { pw * 0.5625 }))
                .pixels_fit(&preview.pixels, rune::SurfaceFit::Fit)
        } else {
            row()
                .w_full()
                .h((pw * 0.5625).min(if h < 560.0 { 60.0 } else { pw * 0.5625 }))
                .center()
                .child(
                    text("Esperando fotogramas…")
                        .font(mono(10.0, 0.0))
                        .color(vx::dim()),
                )
        });
        overlay.push(card.child(text(preview.status()).font(mono(9.0, 0.0)).color(vx::dim())));
    }
    let stage = overlay
        .into_iter()
        .fold(rune::effect(LIVE_MARK), |slot, e| slot.child(e))
        .key(SLOT_KEY)
        .w_full()
        .h(h)
        .min_h(480.0)
        .radius(16.0)
        .fill(vx::ink())
        // Como la web (OrbitControls): arrastrar desplaza la vista, la rueda acerca o aleja, con Shift gira, y doble clic recentra.
        // Con el retrato (que no es 3D) los gestos no hacen nada.
        .on_drag(|s: &mut State, d: &mut rune::Drag, _| {
            let l = &mut s.live;
            match d.phase {
                rune::Phase::Down => {
                    l.dragging = true;
                    l.drag_last = 0.0;
                    l.drag_last_y = 0.0;
                }
                phase => {
                    if l.avatar_model {
                        l.pan_by(d.dx - l.drag_last, d.dy - l.drag_last_y);
                    }
                    l.drag_last = d.dx;
                    l.drag_last_y = d.dy;
                    if phase == rune::Phase::Up {
                        l.dragging = false;
                        if d.clicks >= 2 && d.dx.abs() < 4.0 && d.dy.abs() < 4.0 && !d.cancelled() {
                            l.reset_view();
                        }
                    }
                }
            }
        })
        .on_wheel(|s: &mut State, e: rune::WheelEvent, _| {
            if s.win.1 - super::theme::TOPBAR_H - 32.0 < 480.0 {
                return false;
            }
            if s.live.avatar_model {
                if e.mods.shift {
                    s.live.drag_yaw(-(e.dx + e.dy));
                } else {
                    s.live.zoom_by((e.dy / 40.0).clamp(-3.0, 3.0));
                }
            }
            true
        });
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .padding(16.0)
        .child(stage)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
}

fn header(s: &State, w: f32) -> El {
    let now = std::time::Instant::now();
    let bia = s.live.bia(s.voice_on, s.voice_live, now);
    let tone = match bia {
        Bia::Idle | Bia::Connecting => vx::dim(),
        Bia::Listening => vx::honey(),
        Bia::Working => vx::violet(),
        Bia::Speaking => vx::cyan(),
        Bia::Error => super::theme::red(),
    };
    let mut state = col().shrink().max_w(w * 0.38).gap(4.0).child(
        row()
            .gap(9.0)
            .center_y()
            .child(rune::el().size(7.0, 7.0).radius(3.5).fill(tone))
            .child(
                text(bia.label("BIA").to_uppercase())
                    .font(mono(10.5, 2.1))
                    .color(tone),
            ),
    );
    if s.voice_live {
        if let Some(m) = &s.live.model {
            state = state.child(
                text(format!("{} · {}", short_model(m), s.live.voice_id()))
                    .font(mono(10.0, 0.5))
                    .color(vx::dim()),
            );
        }
    }
    glass()
        .key("live/head")
        .absolute(16.0, 16.0)
        .w(w - 32.0)
        .h(head_height(w))
        .px(18.0)
        .py(12.0)
        .gap(2.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .justify_between()
                .center_y()
                .child(
                    col()
                        .shrink()
                        .grow(1.0)
                        .gap(5.0)
                        .child(text("HIVELIVE").font(mono(10.5, 3.6)).color(vx::gold()))
                        .child(
                            text("BIA — la voz de Hive, coordinando tu enjambre de agentes")
                                .font(TextStyle::new(14.5, 400.0).mono())
                                .color(vx::fg(0.86)),
                        ),
                )
                .child(state),
        )
}

fn cell(name: &str, value: String) -> El {
    col()
        .grow(1.0)
        .w(0.0)
        .px(14.0)
        .py(10.0)
        .gap(5.0)
        .child(label(name))
        .child(
            text(value)
                .font(TextStyle::new(14.0, 500.0).mono())
                .color(vx::fg(0.92)),
        )
}

fn select_field(
    s: &State,
    key: &'static str,
    name: &str,
    value: String,
    which: u8,
    options: Vec<(String, String)>,
    pick: fn(&mut State, usize),
) -> El {
    // El avatar no toca la sesión: se cambia en llamada; la voz y el idioma no (exigirían reconectar).
    let live =
        ((s.voice_live || s.voice_on) && which != 3) || (which == 4 && s.live.catalog.busy());
    let open = s.live.open_select == which && !live;
    let mut c = col().px(12.0).gap(5.0).child(label(name));
    let head = row()
        .key(key)
        .w_full()
        .px(12.0)
        .py(9.0)
        .radius(9.0)
        .fill(vx::wash(0.05))
        .border(1.0, vx::faint())
        .center_y()
        .child(
            text(value)
                .font(TextStyle::new(12.5, 500.0).mono())
                .color(vx::fg(if live { 0.5 } else { 0.92 }))
                .grow(1.0),
        )
        .child(rune::icon(&icons::CHEVRON_DOWN, 14.0).color(vx::dim()));
    c = c.child(if live || options.len() < 2 {
        head
    } else {
        head.tap(move |s: &mut State| {
            s.live.open_select = if s.live.open_select == which {
                0
            } else {
                which
            }
        })
    });
    if open {
        let mut list = col()
            .w_full()
            .radius(9.0)
            .fill(vx::ink())
            .border(1.0, vx::faint())
            .py(4.0);
        for (i, (id, shown)) in options.into_iter().enumerate() {
            list = list.child(
                row()
                    .key(format!("{key}/{id}"))
                    .w_full()
                    .px(12.0)
                    .py(6.0)
                    .child(
                        text(shown)
                            .font(TextStyle::new(12.0, 500.0).mono())
                            .color(vx::fg(0.85)),
                    )
                    .tap(move |s: &mut State| {
                        pick(s, i);
                        s.live.open_select = 0;
                    }),
            );
        }
        c = c.child(list);
    }
    c
}

fn telemetry(s: &State, w: f32, h: f32) -> El {
    let l = &s.live;
    let now = std::time::Instant::now();
    let narrow = w < NARROW_STAGE;
    let collapsed = if narrow {
        !l.panel_open_narrow
    } else {
        l.panel_collapsed
    };
    let pw = PANEL_W.min(w * 0.34).max(if narrow { 260.0 } else { 0.0 });
    let head = row()
        .gap(10.0)
        .center_y()
        .px(10.0)
        .py(10.0)
        .child(
            row()
                .key("live/panel-toggle")
                .size(30.0, 30.0)
                .radius(8.0)
                .center()
                .fill(vx::wash(0.05))
                .border(1.0, vx::faint())
                .child(rune::icon(&icons::PANEL_RIGHT_CLOSE, 15.0).color(vx::dim()))
                .tap(|s: &mut State| {
                    if s.live.viewport.0 < NARROW_STAGE {
                        s.live.panel_open_narrow = !s.live.panel_open_narrow
                    } else {
                        s.live.panel_collapsed = !s.live.panel_collapsed
                    }
                }),
        )
        .child(text("TELEMETRÍA").font(mono(9.5, 2.4)).color(vx::dim()));
    let mut c = glass()
        .key("live/telemetry")
        .absolute(w - 16.0 - pw, 16.0 + head_height(w))
        .w(pw)
        .child(head);
    if collapsed {
        return c;
    }
    c = c
        .h((h - 16.0 - head_height(w) - 16.0).max(80.0))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    let vision = match (crate::media_flags().0, crate::media_flags().1) {
        (true, true) => "cámara + pantalla",
        (true, false) => "cámara",
        (false, true) => "pantalla",
        _ => "off",
    };
    let money = if l.has_pricing() && l.total_tokens() > 0 {
        format!("${:.4}", l.cost_usd())
    } else {
        "—".into()
    };
    let tokens = if l.total_tokens() > 0 {
        crate::office::hud::format_token_count(l.total_tokens() as f64)
    } else {
        "—".into()
    };
    let ms = l.latency_ms.map_or("—".to_string(), |m| format!("{m} ms"));
    let model_id = if s.voice_on || s.voice_live {
        l.model.as_deref()
    } else {
        l.catalog.selected.as_deref().or(l.model.as_deref())
    };
    let model = model_id.map(short_model).unwrap_or_else(|| "—".into());
    let spec = if l.specialists.is_empty() {
        "—".to_string()
    } else {
        l.specialists.len().to_string()
    };
    let line = || col().w_full().h(1.0).fill(vx::faint());
    let pair = |a: El, b: El| {
        row()
            .w_full()
            .child(a)
            .child(col().w(1.0).h(52.0).fill(vx::faint()))
            .child(b)
    };
    c = c
        .child(line())
        .child(pair(
            cell("Sesión", l.elapsed_label(now)),
            cell("Latencia", ms),
        ))
        .child(line())
        .child(pair(
            cell("Tokens", tokens),
            cell(
                if l.has_pricing() {
                    "Costo estimado"
                } else {
                    "Costo sin tarifa"
                },
                money,
            ),
        ))
        .child(line())
        .child(pair(
            cell("Modelo", model),
            cell(
                "Voz activa",
                if s.voice_live {
                    l.voice_id().to_string()
                } else {
                    "—".into()
                },
            ),
        ))
        .child(line())
        .child(pair(
            cell("Especialistas", spec),
            cell("Visión", vision.to_string()),
        ))
        .child(line())
        .child(
            col()
                .gap(10.0)
                .py(10.0)
                .child(select_field(
                    s,
                    "live/model",
                    "Modelo de voz",
                    l.catalog.label(),
                    4,
                    l.catalog.models.clone(),
                    |s, i| {
                        if let (Some(conn), Some((id, _))) =
                            (s.settings.conn.as_ref(), s.live.catalog.models.get(i))
                        {
                            let id = id.clone();
                            s.live.catalog.select(conn, id);
                        }
                    },
                ))
                .child(select_field(
                    s,
                    "live/avatar",
                    "Avatar",
                    if l.avatar_model {
                        "Modelo 3D (se gira con el ratón)"
                    } else {
                        "Retrato (mejor acabado)"
                    }
                    .to_string(),
                    3,
                    vec![
                        ("modelo".into(), "Modelo 3D (se gira con el ratón)".into()),
                        ("foto".into(), "Retrato (mejor acabado)".into()),
                    ],
                    |s, i| s.live.avatar_model = i == 0,
                ))
                .child(select_field(
                    s,
                    "live/voice",
                    "Voz",
                    format!("{} — {}", l.voice_id(), VOICES[l.voice].1),
                    1,
                    VOICES
                        .iter()
                        .map(|(id, note)| (id.to_string(), format!("{id} — {note}")))
                        .collect(),
                    |s, i| s.live.voice = i,
                ))
                .child(select_field(
                    s,
                    "live/lang",
                    "Idioma y acento",
                    LANGS[l.lang].1.to_string(),
                    2,
                    LANGS
                        .iter()
                        .map(|(id, name)| (id.to_string(), name.to_string()))
                        .collect(),
                    |s, i| s.live.lang = i,
                )),
        )
        .child(line());
    if let Some(error) = &l.catalog.error {
        c = c.child(
            col()
                .key("live/model-error")
                .px(12.0)
                .gap(4.0)
                .child(
                    text(error.clone())
                        .font(TextStyle::new(11.0, 400.0).mono())
                        .color(vx::gold())
                        .wrap(),
                )
                .child(
                    row()
                        .key("live/model-retry")
                        .py(5.0)
                        .child(
                            text("Reintentar cargar modelos")
                                .font(mono(11.0, 0.0))
                                .color(vx::gold()),
                        )
                        .tap(|s: &mut State| {
                            if let Some(conn) = s.settings.conn.as_ref() {
                                s.live.catalog.refresh(conn);
                            }
                        }),
                ),
        );
    }
    // Lo que hace la colmena por dentro: los últimos hitos reales, o la explicación mientras no hay ninguno.
    let mut hive = col()
        .key("live/hive")
        .grow(1.0)
        .min_h(0.0)
        .px(14.0)
        .py(10.0)
        .gap(6.0)
        .child(label("Actividad de la colmena"));
    if l.milestones.is_empty() {
        hive = hive.child(text("Cuando BIA delegue trabajo verás aquí cada paso: qué especialista lo tomó, qué herramienta usó y si quedó verificado.").font(TextStyle::new(11.5, 400.0).mono()).color(vx::dim()).wrap());
    }
    for (i, m) in l.milestones.iter().rev().take(6).enumerate() {
        let (glyph, tone) = match m.kind.as_str() {
            "delegated" | "worker_started" => ("→", vx::violet()),
            "verified" => ("✓", super::theme::green()),
            "failed" => ("!", super::theme::red()),
            "group_ready" => ("◆", vx::cyan()),
            _ => ("·", vx::dim()),
        };
        hive = hive.child(
            row()
                .key(format!("live/ms/{i}"))
                .gap(8.0)
                .child(text(glyph).font(mono(12.0, 0.0)).color(tone))
                .child(
                    text(if m.agent.is_empty() {
                        m.label.clone()
                    } else {
                        format!("{} · {}", m.agent, m.label)
                    })
                    .font(TextStyle::new(11.5, 400.0).mono())
                    .color(vx::fg(0.82))
                    .wrap()
                    .grow(1.0),
                ),
        );
    }
    c.child(hive)
}

fn transcript(s: &State, w: f32, h: f32) -> El {
    // Las dos últimas frases de la conversación hablada.
    let lines: Vec<String> = s
        .msgs
        .iter()
        .filter(|m| m.id.starts_with('v'))
        .rev()
        .take(2)
        .map(|m| m.text.trim().to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let body = if lines.is_empty() {
        text("La transcripción aparece aquí mientras hablas.")
            .font(TextStyle::new(12.0, 400.0).mono())
            .color(vx::dim())
    } else {
        text(lines.join("  ·  "))
            .font(TextStyle::new(12.0, 400.0).mono())
            .color(vx::fg(0.88))
            .wrap()
    };
    glass()
        .key("live/transcript")
        .absolute(16.0, h - 16.0 - 60.0)
        .w((w * 0.3).clamp(200.0, 416.0))
        .h(60.0)
        .px(18.0)
        .py(8.0)
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(body)
                .scroll(rune::ScrollSpec { stick_bottom: true }),
        )
}

fn aux(
    key: &'static str,
    ico: &'static rune::Icon,
    on: bool,
    enabled: bool,
    press: fn(&mut State),
) -> El {
    let c = if on {
        vx::honey()
    } else {
        vx::fg(if enabled { 0.8 } else { 0.35 })
    };
    let b = row()
        .key(key)
        .size(42.0, 42.0)
        .radius(21.0)
        .center()
        .fill(vx::ink().alpha(0.8))
        .border(
            1.0,
            if on {
                vx::honey().alpha(0.6)
            } else {
                vx::faint()
            },
        )
        .child(rune::icon(ico, 17.0).color(c));
    if enabled {
        b.tap(press)
    } else {
        b
    }
}

fn dock(s: &State, w: f32, h: f32) -> El {
    let live = s.voice_live;
    let (cam, scr) = crate::media_flags();
    let call_icon = if live {
        &icons::PHONE_OFF
    } else if s.voice_on {
        &icons::LOADER
    } else {
        &icons::MIC
    };
    let call_tone = if live {
        super::theme::red()
    } else {
        vx::cyan()
    };
    let call = row()
        .key("voice")
        .size(72.0, 72.0)
        .radius(36.0)
        .center()
        .fill(vx::ink().alpha(0.85))
        .border(1.5, call_tone.alpha(0.7))
        .shadow(0.0, 18.0, call_tone.alpha(if live { 0.35 } else { 0.2 }))
        .child(rune::icon(call_icon, 26.0).color(call_tone))
        .tap(|s: &mut State| crate::toggle_voice(s));
    let mut buttons = row()
        .gap(12.0)
        .center_y()
        .child(call)
        .child(aux(
            "voice/camera",
            if cam {
                &icons::VIDEO
            } else {
                &icons::VIDEO_OFF
            },
            cam,
            live,
            |s| crate::toggle_media(s, false),
        ))
        .child(aux(
            "voice/screen",
            if scr {
                &icons::MONITOR
            } else {
                &icons::MONITOR_OFF
            },
            scr,
            live,
            |s| crate::toggle_media(s, true),
        ))
        // Elegir micrófono, cámara y salida: abre «Pantalla y audio», y lo que se cambie allí se aplica a la llamada en curso.
        .child(aux("voice/devices", &icons::SLIDERS, false, true, |s| {
            crate::open_screen(s, crate::Screen::Settings);
            crate::settings::view::select_tab(s, crate::settings::Tab::Display);
        }));
    let mut n = 4.0;
    if live {
        buttons = buttons.child(aux("voice/interrupt", &icons::VOLUME_X, false, true, |s| {
            crate::interrupt_voice(s)
        }));
        n += 1.0;
    }
    let row_w = 72.0 + (n - 1.0) * 42.0 + (n - 1.0) * 12.0;
    let mut c = col()
        .key("live/dock")
        .absolute((w - row_w) / 2.0, h - 16.0 - 62.0 - 12.0 - 72.0)
        .w(row_w)
        .gap(12.0)
        .child(buttons);
    if let Some(e) = crate::media_error() {
        c = c.child(
            text(e)
                .font(TextStyle::new(11.0, 500.0).mono())
                .color(super::theme::red())
                .wrap(),
        );
    }
    c
}

/// Los botones de la vista (como las teclas y los gestos de la web): girar, acercar, alejar y recentrar. Solo con el modelo 3D.
fn view_controls(s: &State, h: f32) -> El {
    let btn = |key: &'static str, ico: &'static rune::Icon, f: fn(&mut State)| {
        row()
            .key(key)
            .size(34.0, 34.0)
            .radius(10.0)
            .center()
            .fill(vx::ink().alpha(0.8))
            .border(1.0, vx::faint())
            .child(rune::icon(ico, 15.0).color(vx::fg(0.8)))
            .tap(f)
    };
    let c = col()
        .key("live/view")
        .absolute(28.0, (h * 0.5 - 100.0).max(100.0))
        .gap(8.0);
    if !s.live.avatar_model {
        return c;
    }
    c.child(btn("live/zoom-in", &icons::PLUS, |s| s.live.zoom_by(2.0)))
        .child(btn("live/zoom-out", &icons::MINUS, |s| {
            s.live.zoom_by(-2.0)
        }))
        .child(btn("live/rot-l", &icons::ARROW_LEFT, |s| {
            s.live.drag_yaw(-25.0)
        }))
        .child(btn("live/rot-r", &icons::ARROW_RIGHT, |s| {
            s.live.drag_yaw(25.0)
        }))
        .child(btn("live/reset", &icons::CROSSHAIR, |s| {
            s.live.reset_view()
        }))
}

/// El texto de ayuda, en su propia pastilla centrada bajo los botones.
fn hint(w: f32, h: f32) -> El {
    if w < NARROW_STAGE {
        return col();
    }
    let pill_w = 260.0;
    glass().key("live/hint").absolute((w - pill_w) / 2.0, h - 16.0 - 62.0).w(pill_w).h(62.0).radius(24.0).px(16.0).center()
        .child(text("BIA coordina la colmena entera mientras hablas. Pídele lo mismo que le pedirías por escrito.").font(TextStyle::new(10.5, 400.0).mono()).color(vx::dim()).wrap())
}

#[allow(dead_code)]
fn _screen(_: Screen, _: f32) -> f32 {
    radius::SM + type_::LABEL.size
}
