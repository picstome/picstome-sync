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
