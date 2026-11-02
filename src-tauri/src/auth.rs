use rand::RngCore;

const SERVICE: &str = "com.picstome.sync";
const USER: &str = "token";

fn entry() -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, USER).ok()
}

pub fn load_token() -> Option<String> {
    let entry = entry()?;
    entry.get_password().ok().filter(|token| !token.is_empty())
}

pub fn save_token(token: Option<&str>) {
    let Some(entry) = entry() else {
        return;
    };
    match token {
        Some(token) if !token.is_empty() => {
            if let Err(err) = entry.set_password(token) {
                eprintln!("Could not store token in the OS keychain: {err}");
            }
        }
        _ => {
            let _ = entry.delete_credential();
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
}
