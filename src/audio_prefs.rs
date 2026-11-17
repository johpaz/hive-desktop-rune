//! El micrófono, la salida y la cámara que eligió la persona en «Pantalla y audio». Se guardan en un archivo
//! local (`~/.config/hive-desktop-rune/audio.json`, sin secretos); vacío = el predeterminado del sistema.
//! Un archivo anterior, sin `camera`, sigue valiendo.

use serde_json::{json, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    pub input: Option<String>,
    pub output: Option<String>,
    pub camera: Option<String>,
}

fn path() -> Option<std::path::PathBuf> {
    Some(crate::ui::theme::config_dir()?.join("audio.json"))
}

pub fn parse(text: &str) -> Prefs {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let pick = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    Prefs {
        input: pick("input"),
        output: pick("output"),
        camera: pick("camera"),
    }
}

pub fn to_json(p: &Prefs) -> String {
    json!({"input": p.input.clone().unwrap_or_default(), "output": p.output.clone().unwrap_or_default(), "camera": p.camera.clone().unwrap_or_default()}).to_string()
}

pub fn load() -> Prefs {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn save(p: &Prefs) {
    if let Some(path) = path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, to_json(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_empty_means_the_system_default() {
        let p = Prefs {
            input: Some("pipewire:alsa_input.usb-x".into()),
            output: None,
            camera: Some("1".into()),
        };
        let back = parse(&to_json(&p));
        assert_eq!(back, p);
        assert_eq!(
            parse(""),
            Prefs::default(),
            "un archivo vacío o dañado no rompe nada"
        );
        assert_eq!(parse("no es json"), Prefs::default());
        assert_eq!(parse(r#"{"input":"","output":"o"}"#), Prefs { input: None, output: Some("o".into()), camera: None }, "una cadena vacía es «predeterminado»; sin `camera` (archivo anterior) tampoco hay cámara elegida");
    }
}
