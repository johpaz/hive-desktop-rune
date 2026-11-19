//! Colores y medidas del tema oscuro de la web (`packages/hive-ui/src/index.css`, clase `.dark`).
//! Las variables de la web son HSL: se convierten aquí tal cual para que coincidan al píxel.

#![allow(dead_code)] // son los tokens de diseño de la web: no todos se usan en cada pantalla

use rune::Color;

/// HSL (h en grados, s y l en 0–100) a un `Color` de Rune.
pub fn hsl(h: f32, s: f32, l: f32) -> Color {
    hsla(h, s, l, 1.0)
}

pub fn hsla(h: f32, s: f32, l: f32, a: f32) -> Color {
    let (s, l) = (s / 100.0, l / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8
    );
    rune::color::hex(&hex).unwrap_or(Color::WHITE).alpha(a)
}

use std::sync::atomic::{AtomicBool, Ordering};

/// Tema activo: `false` = oscuro (el de la web), `true` = crema.
static LIGHT: AtomicBool = AtomicBool::new(false);

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

fn pick(dark: Color, light: Color) -> Color {
    if is_light() {
        light
    } else {
        dark
    }
}

/// Los tokens de `rune::tok` (los que usan Ajustes, A2UI y Reuniones) siguen al tema activo.
pub fn tok() -> rune::tok::Theme {
    rune::tok::theme(!is_light())
}

pub fn accent_tok() -> Color {
    rune::tok::accent(0, !is_light())
}

// Oscuro: `.dark` de `index.css`. Crema: misma estructura, pero con fondos cálidos, tarjetas más claras que
// el fondo y texto casi negro, para que los contrastes se mantengan (texto ≥ 4,5:1 sobre las tarjetas).
pub fn background() -> Color {
    pick(hsl(220.0, 15.0, 8.0), hsl(42.0, 38.0, 94.0))
}
pub fn foreground() -> Color {
    pick(hsl(40.0, 10.0, 90.0), hsl(30.0, 28.0, 11.0))
}
pub fn card() -> Color {
    pick(hsl(220.0, 15.0, 11.0), hsl(42.0, 45.0, 98.0))
}
pub fn muted() -> Color {
    pick(hsl(220.0, 12.0, 16.0), hsl(40.0, 28.0, 88.0))
}
pub fn muted_foreground() -> Color {
    pick(hsl(220.0, 8.0, 55.0), hsl(30.0, 12.0, 36.0))
}
pub fn border() -> Color {
    pick(hsl(220.0, 12.0, 18.0), hsl(38.0, 24.0, 78.0))
}
pub fn sidebar() -> Color {
    pick(hsl(220.0, 15.0, 10.0), hsl(40.0, 32.0, 90.0))
}
pub fn sidebar_foreground() -> Color {
    pick(hsl(40.0, 10.0, 80.0), hsl(30.0, 22.0, 20.0))
}
pub fn sidebar_accent() -> Color {
    pick(hsl(220.0, 12.0, 14.0), hsl(38.0, 30.0, 84.0))
}
pub fn sidebar_border() -> Color {
    pick(hsl(220.0, 12.0, 16.0), hsl(38.0, 24.0, 78.0))
}
/// Fondo del elemento activo de la barra lateral.
pub fn nav_active_fill() -> Color {
    pick(hsla(217.0, 60.0, 14.0, 1.0), hsla(221.0, 83.0, 53.0, 0.12))
}

pub fn amber() -> Color {
    pick(hsl(38.0, 92.0, 50.0), hsl(32.0, 92.0, 40.0))
}
pub fn blue() -> Color {
    pick(hsl(217.0, 91.0, 60.0), hsl(221.0, 83.0, 45.0))
}
pub fn blue_dark() -> Color {
    hsl(221.0, 83.0, 53.0)
}
pub fn purple() -> Color {
    pick(hsl(271.0, 81.0, 66.0), hsl(271.0, 70.0, 50.0))
}
pub fn green() -> Color {
    pick(hsl(142.0, 71.0, 45.0), hsl(142.0, 70.0, 32.0))
}
pub fn red() -> Color {
    pick(hsl(0.0, 84.0, 60.0), hsl(0.0, 74.0, 46.0))
}
pub fn cyan() -> Color {
    hsl(187.0, 95.0, 42.0)
}
/// Texto sobre un botón de acento (ámbar).
pub fn on_amber() -> Color {
    hsl(30.0, 10.0, 5.0)
}

/// Cristal translúcido: en oscuro, blanco al 3 % / 5 %; en crema, negro al 3 % / 5 %.
/// El velo al pasar el puntero y al pulsar: blanco tenue sobre oscuro, negro tenue sobre crema.
pub fn hover_veil() -> Color {
    pick(Color::WHITE.alpha(0.07), Color::BLACK.alpha(0.05))
}
pub fn press_veil() -> Color {
    pick(Color::WHITE.alpha(0.12), Color::BLACK.alpha(0.09))
}
pub fn glass() -> Color {
    pick(Color::WHITE.alpha(0.03), Color::BLACK.alpha(0.03))
}
pub fn glass_strong() -> Color {
    pick(Color::WHITE.alpha(0.05), Color::BLACK.alpha(0.05))
}
pub fn border_subtle() -> Color {
    pick(Color::WHITE.alpha(0.05), Color::BLACK.alpha(0.08))
}
pub fn border_base() -> Color {
    pick(Color::WHITE.alpha(0.10), Color::BLACK.alpha(0.14))
}
pub fn border_strong() -> Color {
    pick(Color::WHITE.alpha(0.20), Color::BLACK.alpha(0.26))
}
pub fn text_muted() -> Color {
    pick(hsla(40.0, 10.0, 90.0, 0.40), hsla(30.0, 28.0, 11.0, 0.55))
}

/// La carpeta de preferencias locales de la app (no guarda secretos): `$XDG_CONFIG_HOME` o `~/.config`.
pub fn config_dir() -> Option<std::path::PathBuf> {
    // Las pruebas nunca tocan las preferencias reales de quien las ejecuta.
    if cfg!(test) {
        return None;
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("hive-desktop-rune"))
}

/// Dónde se guarda la elección entre oscuro y crema.
fn pref_path() -> Option<std::path::PathBuf> {
    Some(config_dir()?.join("theme"))
}

/// Carga la elección guardada; `HIVE_THEME=light|dark` manda sobre ella (útil para capturas y pruebas).
pub fn load() {
    let forced = std::env::var("HIVE_THEME").ok();
    let saved = pref_path().and_then(|p| std::fs::read_to_string(p).ok());
    let value = forced.or(saved).unwrap_or_default();
    set_light(matches!(value.trim(), "light" | "crema"));
}

pub fn save() {
    if let Some(p) = pref_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, if is_light() { "light" } else { "dark" });
    }
}

/// Paleta de HiveLive (`voice.css`): grafito y oro, con el cian de Hive.
pub mod vx {
    use rune::{color::oklcha, Color};
    pub fn honey() -> Color {
        oklcha(0.82, 0.15, 78.0, 1.0)
    }
    pub fn gold() -> Color {
        oklcha(0.78, 0.13, 82.0, 1.0)
    }
    pub fn cyan() -> Color {
        oklcha(0.82, 0.13, 218.0, 1.0)
    }
    pub fn violet() -> Color {
        oklcha(0.68, 0.18, 292.0, 1.0)
    }
    /// Fondo de los paneles de HiveLive: azul noche, o crema en el tema claro.
    pub fn ink() -> Color {
        if super::is_light() {
            oklcha(0.975, 0.012, 85.0, 1.0)
        } else {
            oklcha(0.16, 0.03, 258.0, 1.0)
        }
    }
    pub fn dim() -> Color {
        fg(0.42)
    }
    pub fn faint() -> Color {
        if super::is_light() {
            Color::BLACK.alpha(0.13)
        } else {
            Color::WHITE.alpha(0.14)
        }
    }
    /// Texto sobre los paneles: blanco con transparencia sobre el oscuro, tinta sobre el crema.
    pub fn fg(a: f32) -> Color {
        if super::is_light() {
            super::foreground().alpha((a + 0.2).min(1.0))
        } else {
            Color::WHITE.alpha(a)
        }
    }
    /// Relleno tenue de controles (el 5 % de blanco o de negro).
    pub fn wash(a: f32) -> Color {
        if super::is_light() {
            Color::BLACK.alpha(a)
        } else {
            Color::WHITE.alpha(a)
        }
    }
}

pub const SIDEBAR_W: f32 = 256.0;
pub const SIDEBAR_COLLAPSED_W: f32 = 56.0;
pub const TOPBAR_H: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// El tema es global: las pruebas que lo cambian se turnan.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// ¿`c` es `hex` con a lo sumo 2 de diferencia por canal? (los hex de los comentarios del CSS son aproximados)
    fn near(c: Color, hex: &str) -> bool {
        let want = rune::color::hex(hex).unwrap().hex().to_lowercase();
        let got = c.hex().to_lowercase();
        (0..3).all(|k| {
            let a = i32::from_str_radix(&want[1 + 2 * k..3 + 2 * k], 16).unwrap();
            let b = i32::from_str_radix(&got[1 + 2 * k..3 + 2 * k], 16).unwrap();
            (a - b).abs() <= 2
        })
    }

    #[test]
    fn hsl_matches_the_values_the_web_computes() {
        // `--background: 220 15% 8%` es rgb(17, 19, 23).
        assert_eq!(&hsl(220.0, 15.0, 8.0).hex().to_lowercase()[..7], "#111317");
        // Los de marca, contra los hex que documenta la propia `index.css`.
        assert!(near(hsl(38.0, 92.0, 50.0), "#f59e0b"), "ámbar");
        assert!(near(hsl(217.0, 91.0, 60.0), "#3b82f6"), "azul");
        assert!(near(hsl(142.0, 71.0, 45.0), "#22c55e"), "verde");
    }

    #[test]
    fn alpha_survives_the_conversion() {
        assert!((hsla(0.0, 0.0, 100.0, 0.4).hex().len()) >= 7);
    }

    #[test]
    fn the_cream_theme_keeps_text_readable_on_every_surface() {
        let _g = SERIAL.lock().unwrap();
        set_light(true);
        let lum = |c: Color| c.luminance();
        let ratio = |a: Color, b: Color| {
            let (x, y) = (lum(a) + 0.05, lum(b) + 0.05);
            if x > y {
                x / y
            } else {
                y / x
            }
        };
        for (name, text, bg) in [
            ("texto sobre fondo", foreground(), background()),
            ("texto sobre tarjeta", foreground(), card()),
            (
                "texto atenuado sobre fondo",
                muted_foreground(),
                background(),
            ),
            ("texto atenuado sobre tarjeta", muted_foreground(), card()),
            ("menú sobre barra lateral", sidebar_foreground(), sidebar()),
            ("azul activo sobre barra lateral", blue(), sidebar()),
            ("azul sobre el resalte activo", blue(), background()),
        ] {
            assert!(
                ratio(text, bg) >= 4.5,
                "{name}: contraste {:.2} < 4,5",
                ratio(text, bg)
            );
        }
        // Y el fondo es realmente claro y la tarjeta más clara que el fondo, para que se distingan.
        assert!(lum(background()) > 0.7 && lum(card()) > lum(background()));
        set_light(false);
    }

    #[test]
    fn the_dark_theme_keeps_the_webs_contrast_and_is_the_default() {
        let _g = SERIAL.lock().unwrap();
        set_light(false);
        assert!(!is_light());
        assert!(background().luminance() < 0.05);
        assert!(foreground().luminance() > 0.6);
    }
}
