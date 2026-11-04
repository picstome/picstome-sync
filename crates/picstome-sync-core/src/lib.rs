//! Core sync engine for Picstome Sync.
//!
//! This crate intentionally has no Tauri dependency so it can be tested with a
//! plain `cargo test -p picstome-sync-core` on any machine. The desktop shell
//! lives in `src-tauri`.

pub mod api;
pub mod atomic;
pub mod config;
pub mod diff;
pub mod error;
pub mod identity;
pub mod links;
pub mod localfiles;
pub mod mock;
pub mod model;
pub mod sanitize;
pub mod sync;
pub mod transfer;

pub use api::ApiClient;
pub use config::Store;
pub use error::{CoreError, CoreResult, ErrorPayload};
pub use model::*;
