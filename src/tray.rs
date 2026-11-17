//! Bandeja del sistema: icono con "Mostrar Hive" y "Salir", como la app de Tauri.
//!
//! En Linux es un StatusNotifierItem por D-Bus: el icono solo se ve si hay un anfitrión de bandeja
//! (KDE, waybar…; GNOME necesita una extensión). Sin anfitrión, cerrar la ventana a la bandeja dejaría la
//! app invisible y sin forma de recuperarla, así que `close_to_tray` lo decide según haya o no anfitrión.

use std::sync::Mutex;

use rune::{AppHandle, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec};

use crate::State;

pub const ID_SHOW: &str = "show";
pub const ID_QUIT: &str = "quit";

/// Lo que hay que hacer ante un evento de la bandeja.
#[derive(Debug, PartialEq)]
pub enum Cmd {
    /// Mostrar y enfocar la ventana, con el token de activación de Wayland si el anfitrión lo dio.
    Show(String),
    Quit,
    Ignore,
}

pub fn command(ev: &TrayEvent) -> Cmd {
    match ev {
        TrayEvent::Activate { token } => Cmd::Show(token.clone().unwrap_or_default()),
        TrayEvent::Menu(id) if id == ID_SHOW => Cmd::Show(String::new()),
        TrayEvent::Menu(id) if id == ID_QUIT => Cmd::Quit,
        _ => Cmd::Ignore,
    }
}

pub fn menu() -> Vec<TrayItem> {
    vec![
        TrayItem::action(ID_SHOW, "Mostrar Hive"),
        TrayItem::Separator,
        TrayItem::action(ID_QUIT, "Salir"),
    ]
}

pub fn icon() -> Option<TrayIcon> {
    let img = image::load_from_memory(include_bytes!("../assets/tray-32.png"))
        .ok()?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Some(TrayIcon::rgba(w, h, img.into_raw()))
}

/// Crea el icono. El objeto debe vivir mientras la app corra: al soltarlo, el icono desaparece.
pub fn spawn(handle: AppHandle<State>) -> Result<Tray, String> {
    let icon = icon().ok_or("no se pudo leer el icono de la bandeja")?;
    let spec = TraySpec::new("io.hiveagents.desktop", "Hive", icon)
        .tooltip("Hive")
        .menu(menu());
    let handle = Mutex::new(handle);
    Tray::new(spec, move |ev| {
        let h = handle.lock().unwrap();
        match command(&ev) {
            Cmd::Show(token) => h.activate(token),
            Cmd::Quit => h.quit(),
            Cmd::Ignore => {}
        }
    })
    .map_err(|e| e.to_string())
}

/// ¿Hay un anfitrión de bandeja en la sesión? `busctl --user list` sin leyenda: un nombre por línea.
pub fn has_watcher(bus_names: &str) -> bool {
    bus_names.lines().any(|l| {
        let name = l.split_whitespace().next().unwrap_or("");
        name == "org.kde.StatusNotifierWatcher" || name == "org.freedesktop.StatusNotifierWatcher"
    })
}

pub fn host_present() -> bool {
    std::process::Command::new("busctl")
        .args(["--user", "list", "--no-legend"])
        .output()
        .map(|o| has_watcher(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

/// ¿Cerrar la ventana la manda a la bandeja en vez de salir? Con anfitrión o forzado con
/// `HIVE_CLOSE_TO_TRAY=1`; `--no-tray` lo desactiva todo.
pub fn close_to_tray(host: bool, force: Option<&str>, disabled: bool) -> bool {
    !disabled && (host || force == Some("1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_click_and_show_open_the_window_and_quit_quits() {
        assert_eq!(
            command(&TrayEvent::Activate {
                token: Some("tok".into())
            }),
            Cmd::Show("tok".into())
        );
        assert_eq!(
            command(&TrayEvent::Activate { token: None }),
            Cmd::Show(String::new())
        );
        assert_eq!(
            command(&TrayEvent::Menu("show".into())),
            Cmd::Show(String::new())
        );
        assert_eq!(command(&TrayEvent::Menu("quit".into())), Cmd::Quit);
        assert_eq!(command(&TrayEvent::Menu("otro".into())), Cmd::Ignore);
    }

    #[test]
    fn the_menu_has_show_a_separator_and_quit_in_that_order() {
        let m = menu();
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], TrayItem::action("show", "Mostrar Hive"));
        assert_eq!(m[1], TrayItem::Separator);
        assert_eq!(m[2], TrayItem::action("quit", "Salir"));
    }

    #[test]
    fn the_icon_decodes_to_rgba() {
        let i = icon().expect("el icono se decodifica");
        assert_eq!(i, TrayIcon::rgba(32, 32, i_pixels()));
    }

    fn i_pixels() -> Vec<u8> {
        image::load_from_memory(include_bytes!("../assets/tray-32.png"))
            .unwrap()
            .to_rgba8()
            .into_raw()
    }

    #[test]
    fn a_tray_host_is_found_by_its_bus_name() {
        let kde =
            "org.freedesktop.DBus 1 dbus-broker\norg.kde.StatusNotifierWatcher 842 gnome-shell\n";
        assert!(has_watcher(kde));
        assert!(has_watcher("org.freedesktop.StatusNotifierWatcher 1 x"));
        assert!(!has_watcher("org.freedesktop.DBus 1 dbus-broker\n:1.5 99 foo\norg.kde.StatusNotifierItem-1-1 5 y"), "un ítem no es un anfitrión");
        assert!(!has_watcher(""));
    }

    #[test]
    fn closing_to_the_tray_needs_a_host_or_an_explicit_override() {
        assert!(close_to_tray(true, None, false));
        assert!(
            !close_to_tray(false, None, false),
            "sin anfitrión, cerrar la ventana sale: si no, la app quedaría invisible"
        );
        assert!(close_to_tray(false, Some("1"), false));
        assert!(!close_to_tray(false, Some("0"), false));
        assert!(
            !close_to_tray(true, Some("1"), true),
            "--no-tray manda sobre todo"
        );
    }
}
