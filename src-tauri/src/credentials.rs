//! Credential storage in the OS keychain.
//!
//! Lives in the desktop layer rather than `netdiag-core` on purpose. The keyring
//! crate pulls in libdbus on Linux, and the engine crate's value is that it
//! builds and its tests run with no system dependencies — which is what lets CI
//! validate the per-OS scan logic cheaply on all three platforms.
//!
//! The engine therefore never reads a password; it is handed one.

use netdiag_core::unifi::UnifiConfig;

const SERVICE: &str = "dev.localnetdiag.app";

fn entry(config: &UnifiConfig) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &config.credential_id())
        .map_err(|e| format!("keychain unavailable: {e}"))
}

pub fn store(config: &UnifiConfig, password: &str) -> Result<(), String> {
    entry(config)?
        .set_password(password)
        .map_err(|e| format!("could not store password: {e}"))
}

pub fn load(config: &UnifiConfig) -> Result<String, String> {
    entry(config)?.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => {
            "No password stored for this controller — re-enter it in Setup & Status".to_string()
        }
        other => format!("could not read password: {other}"),
    })
}

pub fn clear(config: &UnifiConfig) -> Result<(), String> {
    match entry(config)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(other) => Err(format!("could not clear password: {other}")),
    }
}

/* ----------------------------------------------------------------- assistant */

/// The assistant's model API key.
///
/// Same store, same rule as the controller password: the engine is handed the
/// key and never reads it. Keyed by the provider rather than the model, so
/// switching model does not orphan the entry.
fn assist_entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, id).map_err(|e| format!("keychain unavailable: {e}"))
}

pub fn store_assist_key(id: &str, key: &str) -> Result<(), String> {
    assist_entry(id)?
        .set_password(key)
        .map_err(|e| format!("could not store the API key: {e}"))
}

/// Reads the key, or `None` when none is stored.
///
/// Absence is not an error: the local provider needs no key at all, and the
/// caller decides whether one was required.
pub fn load_assist_key(id: &str) -> Result<Option<String>, String> {
    match assist_entry(id)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(other) => Err(format!("could not read the API key: {other}")),
    }
}

pub fn clear_assist_key(id: &str) -> Result<(), String> {
    match assist_entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(other) => Err(format!("could not clear the API key: {other}")),
    }
}
