//! FoldScan local-first import domain core.
//!
//! Scope of this crate (issue #5, Rust domain layer slice):
//! - device/session manifest parsing with protocol version negotiation;
//! - bounded input validation applied before allocation;
//! - canonical relative-path safety for untrusted device files;
//! - SHA-256 checksum verification of imported captures;
//! - duplicate-ID rejection within and across sessions;
//! - a `FOLDSCAN/` removable-media import pipeline that never writes to or
//!   deletes anything on the device.
//!
//! Evidence category: this is software code with unit/integration tests on
//! synthetic fixtures. It is not physical device integration evidence.

pub mod checksum;
pub mod error;
pub mod import;
pub mod limits;
pub mod manifest;
pub mod paths;
pub mod version;

pub use error::{Category, DomainError};
pub use import::{import_session, import_volume, ImportPlan, ImportedCapture, ImportedSession};
pub use manifest::{CaptureEntry, DeviceManifest, SessionManifest};
pub use version::{KNOWN_MINOR, SUPPORTED_MAJOR};
