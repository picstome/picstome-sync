use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use picstome_sync_core::atomic::write_bytes;
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Entry names in the OS credential store. The service is the bundle
/// identifier so the user can recognise it in Keychain/Credential Manager.
const KEYRING_SERVICE: &str = "com.picstome.sync";
const KEYRING_USER: &str = "picstome-api-token";
/// Escape hatch for headless machines, CI and scripts: keep the 0600 file.
const DISABLE_KEYRING_ENV: &str = "PICSTOME_DISABLE_KEYRING";

/// A broken Secret Service must not freeze the window at startup. macOS is
/// exempt: a Keychain prompt is user-driven and can legitimately wait.
#[cfg(not(target_os = "macos"))]
const CREDENTIAL_TIMEOUT: Option<Duration> = Some(Duration::from_secs(10));
#[cfg(target_os = "macos")]
const CREDENTIAL_TIMEOUT: Option<Duration> = None;

#[derive(Serialize, Deserialize)]
struct StoredAuth {
    token: String,
}

/// The subset of a secure store this app needs. Keeping it behind a trait lets
/// the migration and fallback rules be tested without touching the real OS
/// keychain (which would prompt, or fail on headless CI).
trait SecretBackend: Send + Sync {
    /// `false` means "do not even try"; the file fallback is used directly.
    fn enabled(&self) -> bool {
        true
    }
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, token: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

struct KeyringBackend;

impl SecretBackend for KeyringBackend {
    fn get(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| error.to_string())?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, token: &str) -> Result<(), String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|entry| entry.set_password(token))
            .map_err(|error| error.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| error.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Used when `PICSTOME_DISABLE_KEYRING=1` is set.
struct FileOnlyBackend;

impl SecretBackend for FileOnlyBackend {
    fn enabled(&self) -> bool {
        false
    }
    fn get(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn set(&self, _token: &str) -> Result<(), String> {
        Err("keyring disabled".into())
    }
    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

fn keyring_disabled() -> bool {
    std::env::var(DISABLE_KEYRING_ENV)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Stores the API token in the OS credential store and falls back to the
/// historical `auth.json` (mode 0600) when no secure store is available, so a
/// headless Linux machine can still sign in.
#[derive(Clone)]
pub struct TokenStore {
    path: PathBuf,
    backend: Arc<dyn SecretBackend>,
}

impl std::fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the token; the path is not a secret.
        f.debug_struct("TokenStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl TokenStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let backend: Arc<dyn SecretBackend> = if keyring_disabled() {
            Arc::new(FileOnlyBackend)
        } else {
            Arc::new(KeyringBackend)
        };
        Self {
            path: path.into(),
            backend,
        }
    }

    #[cfg(test)]
    fn with_backend(path: impl Into<PathBuf>, backend: Arc<dyn SecretBackend>) -> Self {
        Self {
            path: path.into(),
            backend,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run a keyring operation on its own thread.
    ///
    /// The Linux zbus-backed Secret Service can deadlock if it is called from
    /// the main thread while a Tokio runtime exists; the crate's own docs ask
    /// for a dedicated thread. Calls are infrequent (startup, login, logout),
    /// so creating one short-lived thread per operation is fine.
    fn call_backend<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&dyn SecretBackend) -> Result<T, String> + Send + 'static,
    {
        let backend = Arc::clone(&self.backend);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("picstome-secret".into())
            .spawn(move || {
                let _ = sender.send(operation(backend.as_ref()));
            })
            .map_err(|error| format!("could not start the credential thread: {error}"))?;
        match CREDENTIAL_TIMEOUT {
            Some(timeout) => receiver
                .recv_timeout(timeout)
                .map_err(|_| "the credential store did not answer in time".to_string())?,
            None => receiver
                .recv()
                .map_err(|error| format!("credential thread failed: {error}"))?,
        }
    }

    fn read_file(&self) -> Option<String> {
        let contents = fs::read_to_string(&self.path).ok()?;
        let stored: StoredAuth = serde_json::from_str(&contents).ok()?;
        let token = stored.token.trim().to_string();
        (!token.is_empty()).then_some(token)
    }

    fn write_file(&self, token: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec(&StoredAuth {
            token: token.trim().to_string(),
        })?;
        write_bytes(&self.path, &data)
    }

    fn remove_file(&self) -> std::io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Load the token: OS credential store first, then the legacy file. A file
    /// left by an older build is migrated into the credential store and
    /// removed on success.
    pub fn load(&self) -> Option<String> {
        if self.backend.enabled() {
            match self.call_backend(|backend| backend.get()) {
                Ok(Some(token)) => {
                    let token = token.trim().to_string();
                    if !token.is_empty() {
                        return Some(token);
                    }
                }
                Ok(None) => {
                    if let Some(token) = self.read_file() {
                        let for_store = token.clone();
                        match self.call_backend(move |backend| backend.set(&for_store)) {
                            Ok(()) => {
                                if let Err(error) = self.remove_file() {
                                    eprintln!(
                                        "The token is in the OS credential store but the old \
                                         auth.json could not be removed: {error}"
                                    );
                                }
                            }
                            Err(error) => eprintln!(
                                "Could not move the API token to the OS credential store; \
                                 keeping the 0600 file: {error}"
                            ),
                        }
                        return Some(token);
                    }
                }
                Err(error) => eprintln!(
                    "Could not read the OS credential store; using the 0600 fallback: {error}"
                ),
            }
        }
        self.read_file()
    }

    /// Store (`Some`) or forget (`None`) the token.
    pub fn save(&self, token: Option<&str>) -> std::io::Result<()> {
        match token {
            Some(token) if !token.trim().is_empty() => {
                let token = token.trim().to_string();
                if self.backend.enabled() {
                    let for_store = token.clone();
                    match self.call_backend(move |backend| backend.set(&for_store)) {
                        Ok(()) => {
                            // The secure store won: do not leave a second copy
                            // of the secret on disk.
                            if let Err(error) = self.remove_file() {
                                eprintln!(
                                    "The token is in the OS credential store but the old \
                                     auth.json could not be removed: {error}"
                                );
                            }
                            return Ok(());
                        }
                        Err(error) => eprintln!(
                            "Could not store the API token in the OS credential store; \
                             writing the 0600 fallback: {error}"
                        ),
                    }
                }
                self.write_file(&token)
            }
            _ => {
                if self.backend.enabled() {
                    if let Err(error) = self.call_backend(|backend| backend.delete()) {
                        // The caller revokes the token server-side before
                        // logging out, so a stale entry that cannot be removed
                        // is already useless.
                        eprintln!(
                            "Could not remove the API token from the OS credential store: {error}"
                        );
                    }
                }
                self.remove_file()
            }
        }
    }

    /// Same as [`TokenStore::save`], off the async runtime. Keyring calls can
    /// block on the OS prompt and must not stall a Tauri worker.
    pub async fn save_async(&self, token: Option<String>) -> std::io::Result<()> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.save(token.as_deref()))
            .await
            .map_err(|error| std::io::Error::other(format!("token store task failed: {error}")))?
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
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryBackend {
        token: Mutex<Option<String>>,
        fail: bool,
    }

    impl MemoryBackend {
        fn failing() -> Self {
            Self {
                token: Mutex::new(None),
                fail: true,
            }
        }

        fn token(&self) -> Option<String> {
            self.token
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    impl SecretBackend for MemoryBackend {
        fn get(&self) -> Result<Option<String>, String> {
            if self.fail {
                return Err("store unavailable".into());
            }
            Ok(self.token())
        }
        fn set(&self, token: &str) -> Result<(), String> {
            if self.fail {
                return Err("store unavailable".into());
            }
            *self
                .token
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(token.to_string());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            if self.fail {
                return Err("store unavailable".into());
            }
            *self
                .token
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            Ok(())
        }
    }

    fn legacy_file(dir: &std::path::Path, token: &str) -> PathBuf {
        let path = dir.join("auth.json");
        fs::write(
            &path,
            serde_json::to_vec(&StoredAuth {
                token: token.into(),
            })
            .unwrap(),
        )
        .unwrap();
        path
    }

    #[test]
    fn nonce_is_long_enough_for_the_backend() {
        let nonce = new_nonce();
        assert_eq!(nonce.len(), 64, "backend requires 43-128 chars");
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(nonce, new_nonce());
    }

    #[test]
    fn stores_in_the_keyring_without_leaving_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Arc::new(MemoryBackend::default());
        let store = TokenStore::with_backend(dir.path().join("auth.json"), backend.clone());

        assert_eq!(store.load(), None);
        store.save(Some("secret-token")).unwrap();
        assert_eq!(backend.token().as_deref(), Some("secret-token"));
        assert_eq!(store.load().as_deref(), Some("secret-token"));
        assert!(
            !store.path().exists(),
            "no plaintext copy after a keyring save"
        );

        store.save(None).unwrap();
        assert_eq!(backend.token(), None);
        assert_eq!(store.load(), None);
        assert!(!store.path().exists());
    }

    #[test]
    fn falls_back_to_a_0600_file_when_the_keyring_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::with_backend(
            dir.path().join("auth.json"),
            Arc::new(MemoryBackend::failing()),
        );

        store.save(Some("secret-token")).unwrap();
        assert_eq!(store.load().as_deref(), Some("secret-token"));
        assert!(store.path().is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn migrates_a_legacy_file_into_the_keyring_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = legacy_file(dir.path(), "legacy-token");
        let backend = Arc::new(MemoryBackend::default());
        let store = TokenStore::with_backend(path.clone(), backend.clone());

        assert_eq!(store.load().as_deref(), Some("legacy-token"));
        assert_eq!(backend.token().as_deref(), Some("legacy-token"));
        assert!(!path.exists(), "the migrated file is removed");
    }

    #[test]
    fn keeps_the_legacy_file_when_the_keyring_cannot_accept_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = legacy_file(dir.path(), "legacy-token");
        let store = TokenStore::with_backend(path.clone(), Arc::new(MemoryBackend::failing()));

        assert_eq!(store.load().as_deref(), Some("legacy-token"));
        assert!(path.exists(), "do not lose the token if migration fails");
    }

    #[test]
    fn disabling_the_keyring_uses_the_file_directly() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            TokenStore::with_backend(dir.path().join("auth.json"), Arc::new(FileOnlyBackend));

        store.save(Some("file-token")).unwrap();
        // `load` must read the file even though the backend is disabled.
        assert_eq!(store.load().as_deref(), Some("file-token"));
        assert!(store.path().is_file());
    }
}
