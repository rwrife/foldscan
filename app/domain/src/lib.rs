//! FoldScan local-first import domain core.
//!
//! Scope of this crate (issue #5, Rust domain layer slice):
//! - device/session manifest parsing with protocol version negotiation;
//! - bounded input validation applied before allocation;
//! - canonical relative-path safety for untrusted device files;
//! - SHA-256 checksum verification of imported captures;
//! - duplicate-ID rejection within and across sessions;
//! - a `FOLDSCAN/` removable-media import pipeline that never writes to or
//!   deletes anything on the device;
//! - versioned processing-recipe documents with a closed operation
//!   vocabulary and content digests;
//! - originals/derivatives export modeling, reorder/remove-without-delete,
//!   collision-checked export layout planning, and a versioned portable
//!   export manifest (`foldscan.export/0.1`) with order-sensitive integrity
//!   digests;
//! - a filesystem export executor that materializes a validated plan under
//!   temporary-write/verify/finalize semantics and writes the manifest last.
//!
//! Evidence category: this is software code with unit/fixture tests on
//! synthetic fixtures. It is not physical device integration evidence.

pub mod checksum;
pub mod error;
pub mod executor;
pub mod export;
pub mod import;
pub mod limits;
pub mod manifest;
pub mod paths;
pub mod recipe;
pub mod version;

pub use error::{Category, DomainError};
pub use export::{
    export_sessions_from_import, plan_export, remove_from, reorder, ContentKind, ExportManifest,
    ExportPage, ExportPlan, ExportSession,
};
pub use import::{import_session, import_volume, ImportPlan, ImportedCapture, ImportedSession};
pub use manifest::{CaptureEntry, DeviceManifest, SessionManifest};
pub use recipe::{OpKind, ProcessingRecipe, RecipeOp};
pub use version::{KNOWN_MINOR, SUPPORTED_MAJOR};
