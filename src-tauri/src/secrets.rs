//! Chaves de API no Credential Manager do Windows. A tela nunca recebe a chave inteira.

use keyring_core::{Entry, Error};

const SERVICE: &str = "copiloto-entrevistas";

pub fn init() -> Result<(), String> {
    let store = windows_native_keyring_store::Store::new().map_err(|e| e.to_string())?;
    keyring_core::set_default_store(store);
    Ok(())
}

fn entry(provider: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, provider).map_err(|e| e.to_string())
}

pub fn save(provider: &str, key: &str) -> Result<(), String> {
    entry(provider)?.set_password(key.trim()).map_err(|e| e.to_string())
}

pub fn load(provider: &str) -> Result<Option<String>, String> {
    match entry(provider)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete(provider: &str) -> Result<(), String> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// `••••a1b2`: o suficiente pra reconhecer qual chave está salva.
pub fn mask(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("••••{tail}")
}
