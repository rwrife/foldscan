//! Stable error categories aligned with the FoldScan protocol error model.
//!
//! Categories intentionally mirror `docs/protocol.md` § Error model so that a
//! host-side import failure can be reported without inventing new vocabulary.

use thiserror::Error;

/// Stable, machine-readable failure categories from the protocol draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Protocol/schema major version is not understood. Fail safely, keep files.
    UnsupportedVersion,
    /// Structure, bounds, path, checksum-format, or duplicate-ID violation.
    InvalidRequest,
    /// File bytes do not match the manifest checksum or declared size.
    ChecksumMismatch,
    /// A referenced file could not be read or was missing.
    StorageUnavailable,
    /// Catch-all that must never carry secrets or full private paths.
    InternalError,
}

/// A validation/import failure with a stable category and a bounded,
/// human-readable diagnostic. Diagnostics must never contain image content,
/// OCR text, credentials, or absolute host paths.
#[derive(Debug, Error)]
#[error("{category:?}: {message}")]
pub struct DomainError {
    pub category: Category,
    pub message: String,
}

impl DomainError {
    pub fn new(category: Category, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    pub fn unsupported_version(message: impl Into<String>) -> Self {
        Self::new(Category::UnsupportedVersion, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(Category::InvalidRequest, message)
    }

    pub fn checksum_mismatch(message: impl Into<String>) -> Self {
        Self::new(Category::ChecksumMismatch, message)
    }

    pub fn storage_unavailable(message: impl Into<String>) -> Self {
        Self::new(Category::StorageUnavailable, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Category::InternalError, message)
    }
}
