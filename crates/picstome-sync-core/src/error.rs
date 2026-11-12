use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("api error ({code}): {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("download hash mismatch for {0}")]
    HashMismatch(String),
    #[error("download hash mismatch for {path} (expected {expected}, got {actual}, {size} bytes on disk)")]
    HashMismatchDetail {
        path: String,
        expected: String,
        actual: String,
        size: u64,
    },
    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn code(&self) -> &str {
        match self {
            CoreError::Io(_) => "io_error",
            CoreError::InvalidData(_) => "invalid_data",
            CoreError::Network(_) => "network_error",
            CoreError::Api { code, .. } => code,
            CoreError::HashMismatch(_) => "hash_mismatch",
            CoreError::HashMismatchDetail { .. } => "hash_mismatch",
            CoreError::Other(_) => "error",
        }
    }

    pub fn status(&self) -> u16 {
        match self {
            CoreError::Api { status, .. } => *status,
            CoreError::Io(_) => 0,
            _ => 0,
        }
    }
}

impl From<reqwest::Error> for CoreError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return CoreError::Network("request timed out".to_string());
        }
        CoreError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(err: serde_json::Error) -> Self {
        CoreError::InvalidData(err.to_string())
    }
}

impl From<url::ParseError> for CoreError {
    fn from(err: url::ParseError) -> Self {
        CoreError::InvalidData(err.to_string())
    }
}

/// Serializable error for Tauri commands.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub message: String,
    pub code: String,
    pub status: u16,
}

impl From<CoreError> for ErrorPayload {
    fn from(err: CoreError) -> Self {
        Self {
            message: err.to_string(),
            code: err.code().to_string(),
            status: err.status(),
        }
    }
}

impl fmt::Display for ErrorPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ErrorPayload {}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> CoreError {
        CoreError::Api {
            status: 401,
            code: "unauthorized".into(),
            message: "nope".into(),
        }
    }

    #[test]
    fn maps_error_codes_and_statuses() {
        assert_eq!(api().code(), "unauthorized");
        assert_eq!(api().status(), 401);
        assert_eq!(CoreError::Io(std::io::Error::other("x")).code(), "io_error");
        assert_eq!(CoreError::InvalidData("x".into()).code(), "invalid_data");
        assert_eq!(CoreError::Network("x".into()).code(), "network_error");
        assert_eq!(CoreError::HashMismatch("x".into()).code(), "hash_mismatch");
        assert_eq!(CoreError::Other("x".into()).code(), "error");
        assert_eq!(CoreError::Other("x".into()).status(), 0);
    }

    #[test]
    fn converts_from_json_and_url_errors() {
        let json: serde_json::Error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        assert!(matches!(CoreError::from(json), CoreError::InvalidData(_)));
        let url: url::ParseError = url::Url::parse("not a url").unwrap_err();
        assert!(matches!(CoreError::from(url), CoreError::InvalidData(_)));
    }

    #[tokio::test]
    async fn converts_from_reqwest_errors() {
        let client = reqwest::Client::new();
        let err = client
            .get("http://127.0.0.1:1/nope")
            .send()
            .await
            .unwrap_err();
        assert!(matches!(CoreError::from(err), CoreError::Network(_)));
    }

    #[test]
    fn error_payload_preserves_message_code_and_status() {
        let payload = ErrorPayload::from(api());
        assert_eq!(payload.message, "api error (unauthorized): nope");
        assert_eq!(payload.code, "unauthorized");
        assert_eq!(payload.status, 401);
        assert_eq!(payload.to_string(), payload.message);
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["code"], "unauthorized");
        assert_eq!(json["status"], 401);
    }
}
