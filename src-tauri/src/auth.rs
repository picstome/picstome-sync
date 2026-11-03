use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct StoredAuth {
    token: String,
}

#[derive(Debug, Clone)]
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Option<String> {
        let contents = fs::read_to_string(&self.path).ok()?;
        let stored: StoredAuth = serde_json::from_str(&contents).ok()?;
        let token = stored.token.trim().to_string();
        (!token.is_empty()).then_some(token)
    }

    pub fn save(&self, token: Option<&str>) -> std::io::Result<()> {
        match token {
            Some(token) if !token.trim().is_empty() => {
                if let Some(parent) = self.path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let data = serde_json::to_vec(&StoredAuth {
                    token: token.trim().to_string(),
                })?;
                let tmp = self.path.with_extension("json.tmp");
                {
                    let mut options = fs::OpenOptions::new();
                    options.write(true).create(true).truncate(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600);
                    }
                    let mut file = options.open(&tmp)?;
                    file.write_all(&data)?;
                    file.sync_all()?;
                }
                fs::rename(&tmp, &self.path)
            }
            _ => match fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            },
        }
    }
}

pub fn new_nonce() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_long_enough_for_the_backend() {
        let nonce = new_nonce();
        assert_eq!(nonce.len(), 64, "backend requires 43-128 chars");
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(nonce, new_nonce());
    }

    #[test]
    fn token_store_roundtrip_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("auth.json"));
        assert_eq!(store.load(), None);
        store.save(Some("secret-token")).unwrap();
        assert_eq!(store.load().as_deref(), Some("secret-token"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        store.save(None).unwrap();
        assert_eq!(store.load(), None);
        store.save(None).unwrap();
    }
}
