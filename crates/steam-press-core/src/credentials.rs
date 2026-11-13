//! Local encrypted credential vault.
//!
//! The vault uses AES-256-GCM with a key generated on this machine. Secrets are
//! never logged, never included in errors and never leave the machine except to
//! the configured downloader.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use rand::{rngs::OsRng, RngCore};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::Paths;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedCredentials {
    pub username: String,
    pub password: Option<String>,
}

pub fn generate_key() -> String {
    let mut key = [0u8; KEY_LEN];
    OsRng.fill_bytes(&mut key);
    STANDARD.encode(key)
}

fn decode_key(key_base64: &str) -> Result<[u8; KEY_LEN], String> {
    let bytes = STANDARD
        .decode(key_base64.trim())
        .map_err(|_| "clé d'identifiants invalide".to_string())?;
    if bytes.len() != KEY_LEN {
        return Err("la clé d'identifiants doit faire 32 octets".to_string());
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes);
    Ok(key)
}

pub fn encrypt(plaintext: &str, key_base64: &str) -> Result<String, String> {
    let key = decode_key(key_base64)?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "initialisation du chiffrement impossible".to_string())?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| "chiffrement des identifiants impossible".to_string())?;

    let mut payload = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(payload))
}

pub fn decrypt(encoded: &str, key_base64: &str) -> Result<String, String> {
    let key = decode_key(key_base64)?;
    let payload = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "fichier d'identifiants invalide".to_string())?;
    if payload.len() <= NONCE_LEN {
        return Err("fichier d'identifiants tronqué".to_string());
    }

    let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "initialisation du déchiffrement impossible".to_string())?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| "identifiants illisibles : mauvaise clé ou fichier corrompu".to_string())?;

    String::from_utf8(plaintext).map_err(|_| "identifiants non UTF-8".to_string())
}

#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn new(paths: &Paths) -> Self {
        Self {
            path: paths.credentials_file(),
        }
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub fn save(&self, credentials: &SavedCredentials, key: &str) -> Result<(), String> {
        if credentials.username.trim().is_empty() {
            return Err("nom d'utilisateur vide".to_string());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let payload = format!(
            "{}\n{}",
            credentials.username.trim(),
            credentials.password.as_deref().unwrap_or("")
        );
        let encoded = encrypt(&payload, key)?;
        write_private(&self.path, &encoded).map_err(|error| error.to_string())
    }

    pub fn load(&self, key: &str) -> Result<Option<SavedCredentials>, String> {
        if !self.exists() {
            return Ok(None);
        }
        let encoded = fs::read_to_string(&self.path).map_err(|error| error.to_string())?;
        if encoded.trim().is_empty() {
            return Ok(None);
        }
        let plaintext = decrypt(&encoded, key)?;
        let mut lines = plaintext.lines();
        let username = lines.next().unwrap_or("").trim().to_string();
        if username.is_empty() {
            return Ok(None);
        }
        let password = lines
            .next()
            .map(str::to_string)
            .filter(|password| !password.is_empty());
        Ok(Some(SavedCredentials { username, password }))
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::File::create(path)?;
        file.write_all(contents.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_round_trip_and_wrong_key_fails() {
        let key = generate_key();
        let encoded = encrypt("s3cret", &key).unwrap();
        assert!(!encoded.contains("s3cret"));
        assert_eq!(decrypt(&encoded, &key).unwrap(), "s3cret");
        assert!(decrypt(&encoded, &generate_key()).is_err());
    }

    #[test]
    fn store_round_trip_and_clear() {
        let dir = crate::test_support::temp_dir("credentials");
        let store = CredentialStore::at(dir.join("credentials.bin"));
        let key = generate_key();
        let credentials = SavedCredentials {
            username: "alice".into(),
            password: Some("hunter2".into()),
        };
        store.save(&credentials, &key).unwrap();
        assert_eq!(store.load(&key).unwrap(), Some(credentials));
        store.clear().unwrap();
        assert_eq!(store.load(&key).unwrap(), None);
    }

    #[test]
    fn malformed_vault_is_an_error() {
        let key = generate_key();
        assert!(decrypt("not-base64", &key).is_err());
        assert!(decrypt(&STANDARD.encode([0u8; 4]), &key).is_err());
    }
}
