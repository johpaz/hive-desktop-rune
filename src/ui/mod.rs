//! Diseño de la app: el mismo de la web (`hive-ui`), tema oscuro.
//!
//! - `theme`: colores y medidas sacados de las variables CSS de `index.css`.
//! - `icons`: los iconos Lucide que usa la web.
//! - `shell`: barra lateral y cabecera comunes a todas las pantallas.
//! - `live`: la pantalla HiveLive (BIA, la voz de Hive).

pub mod agents;
pub mod icons;
pub mod live;
pub mod panel;
pub mod shell;
pub mod system_pages;
pub mod theme;
pub mod work_pages;

/// Un elemento que se puede pulsar, con la respuesta que tiene en la web: cursor de mano, un velo al pasar el puntero,
/// otro más fuerte al pulsar y un leve encogimiento mientras se mantiene (los efectos `hover`, `press` y `press_scale` de Rune).
pub trait Tap {
    fn tap(self, f: impl Fn(&mut crate::State) + 'static) -> Self;
}

impl Tap for rune::El {
    fn tap(self, f: impl Fn(&mut crate::State) + 'static) -> Self {
        self.cursor(rune::Cursor::Pointer)
            .hover(theme::hover_veil())
            .press(theme::press_veil())
            .press_scale(0.98)
            .on_press(f)
    }
}

/// Labels may wrap, including long model IDs and paths, inside their allocated width.
/// Single-line editors keep their own horizontal caret scrolling.
pub(crate) fn text(content: impl Into<String>) -> rune::El {
    rune::text(content).wrap_anywhere().shrink()
}
