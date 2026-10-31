//! Error type shared by the core crate. Every error carries a stable machine code and a retry class
//! (the same vocabulary the HOI4 Factory tool envelope uses) so the MCP layer never invents either.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What a caller may do after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    /// Retrying with identical inputs is safe.
    SafeSameInputs,
    /// Inputs are fine but the state they depend on changed; re-read and rebuild the request.
    RefreshPreconditions,
    /// The outcome of an earlier call is unknown; reconcile before spending again.
    ReconcileFirst,
    /// A human or a policy has to authorize the action first.
    RequiresAuthorization,
    /// Retrying cannot help.
    Terminal,
}

/// Core error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A request argument is malformed or out of range.
    #[error("{0}")]
    Invalid(String),
    /// A path escapes the project or uses a forbidden form.
    #[error("path rejected: {0}")]
    PathRejected(String),
    /// A named thing does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A file or record already exists and must not be overwritten.
    #[error("already exists: {0}")]
    Exists(String),
    /// A policy or gate refuses the action. `code` is stable.
    #[error("refused ({code}): {message}")]
    Refused {
        /// Stable code, for example `install_gate_review_missing`.
        code: &'static str,
        /// Human readable explanation.
        message: String,
        /// What the caller may do next.
        retry: RetryClass,
    },
    /// Filesystem failure.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path (display form).
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// Image decode or encode failure.
    #[error("image error: {0}")]
    Image(String),
    /// Ledger corruption or lock failure.
    #[error("ledger error: {0}")]
    Ledger(String),
    /// Configuration could not be read.
    #[error("config error: {0}")]
    Config(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Build an I/O error with its path.
    pub fn io(path: impl fmt::Display, source: std::io::Error) -> Self {
        Error::Io { path: path.to_string(), source }
    }

    /// Build a refusal.
    pub fn refused(code: &'static str, message: impl Into<String>, retry: RetryClass) -> Self {
        Error::Refused { code, message: message.into(), retry }
    }

    /// Stable machine code.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Invalid(_) => "invalid_argument",
            Error::PathRejected(_) => "path_rejected",
            Error::NotFound(_) => "not_found",
            Error::Exists(_) => "already_exists",
            Error::Refused { code, .. } => code,
            Error::Io { .. } => "io_error",
            Error::Image(_) => "image_error",
            Error::Ledger(_) => "ledger_error",
            Error::Config(_) => "config_error",
        }
    }

    /// Retry class.
    pub fn retry_class(&self) -> RetryClass {
        match self {
            Error::Refused { retry, .. } => *retry,
            Error::Io { .. } => RetryClass::SafeSameInputs,
            Error::Exists(_) => RetryClass::RefreshPreconditions,
            _ => RetryClass::Terminal,
        }
    }
}
