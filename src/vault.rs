//! Token de acceso recordado, solo si la persona lo pide («Recordarme»).
//!
//! Vive en el llavero del sistema (Secret Service en Linux, Keychain en macOS, Credential Manager en
//! Windows), nunca en un archivo. Si el llavero no está disponible, la sesión funciona igual: el token
//! se queda en memoria y la próxima vez se vuelve a pedir.

const SERVICE: &str = "io.hiveagents.desktop.rune";
const ACCOUNT: &str = "access-token";

pub trait Vault: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, token: &str) -> Result<(), String>;
    fn clear(&self);
}

/// El llavero del sistema. `account` permite que las pruebas usen una entrada propia.
pub struct SystemVault {
    account: String,
}

impl Default for SystemVault {
    fn default() -> Self {
        SystemVault {
            account: ACCOUNT.into(),
        }
    }
}

impl SystemVault {
    #[cfg(test)]
    pub fn with_account(account: &str) -> Self {
        SystemVault {
            account: account.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(SERVICE, &self.account)
    }
}

impl Vault for SystemVault {
    fn load(&self) -> Option<String> {
        self.entry()
            .ok()?
            .get_password()
            .ok()
            .filter(|t| !t.is_empty())
    }

    fn save(&self, token: &str) -> Result<(), String> {
        self.entry()
            .and_then(|e| e.set_password(token))
            .map_err(|e| format!("No se pudo guardar en el llavero: {e}"))
    }

    fn clear(&self) {
        if let Ok(e) = self.entry() {
            let _ = e.delete_credential();
        }
    }
}

/// Dónde se busca y se guarda el token. Las pruebas lo cambian por una memoria.
static VAULT: std::sync::Mutex<Option<std::sync::Arc<dyn Vault>>> = std::sync::Mutex::new(None);

pub fn current() -> std::sync::Arc<dyn Vault> {
    VAULT
        .lock()
        .unwrap()
        .get_or_insert_with(|| std::sync::Arc::new(SystemVault::default()))
        .clone()
}

#[cfg(test)]
pub fn install(v: std::sync::Arc<dyn Vault>) {
    *VAULT.lock().unwrap() = Some(v);
}

#[cfg(test)]
#[derive(Default)]
pub struct MemVault(pub std::sync::Mutex<Option<String>>);

#[cfg(test)]
impl Vault for MemVault {
    fn load(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, token: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(token.to_string());
        Ok(())
    }
    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usa el llavero real con una entrada de prueba y un valor falso; la borra al terminar.
    /// `cargo test -- --ignored system_vault`
    #[test]
    #[ignore = "toca el llavero del sistema (entrada de prueba, se borra)"]
    fn system_vault_round_trips_and_clears() {
        let v = SystemVault::with_account("test-entry-delete-me");
        v.clear();
        assert_eq!(v.load(), None);
        v.save("token-falso-de-prueba").unwrap();
        assert_eq!(v.load().as_deref(), Some("token-falso-de-prueba"));
        v.save("otro-valor").unwrap();
        assert_eq!(
            v.load().as_deref(),
            Some("otro-valor"),
            "guardar de nuevo reemplaza"
        );
        v.clear();
        assert_eq!(v.load(), None);
    }
}
