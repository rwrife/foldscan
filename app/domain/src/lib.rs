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
//!   temporary-write/verify/finalize semantics and writes the manifest last,
//!   accepting originals as file-backed sources and processed derivatives as
//!   either host files or in-memory bytes from the processing pipeline;
//! - a deterministic grayscale processing core behind a `Processor`
//!   interface (rotation, strict crop, bilinear quadrilateral rectification,
//!   fixed-point illumination correction) whose output is bit-stable across
//!   IEEE-754 hosts and locked by golden fixtures;
//! - a bounded PNG encode/decode boundary for `GrayFrame` (8-bit grayscale
//!   subset, std-only), with the decoder hardened against untrusted device
//!   bytes and verified against independently encoded/decoded fixtures;
//! - a deterministic PDF export writer that serializes an ordered page set
//!   as image XObjects (std-only), self-verifying structure and payload
//!   fidelity before returning, and cross-checked with an independent PDF
//!   parser at fixture-generation time;
//! - a versioned OCR result document (`foldscan.ocr/0.1`) with closed
//!   failure/skip vocabularies, per-mille numeric confidence, bounded and
//!   control-character-free recognized text, and a provider-interface seam
//!   for a future offline engine (no engine is included).
//!
//! Evidence category: this is software code with unit/fixture tests on
//! synthetic fixtures. It is not physical device integration evidence.

pub mod checksum;
pub mod error;
pub mod executor;
pub mod export;
pub mod image;
pub mod import;
pub mod limits;
pub mod manifest;
pub mod ocr;
pub mod paths;
pub mod pdf;
pub mod png;
pub mod processing;
pub mod recipe;
pub mod version;

pub use error::{Category, DomainError};
pub use export::{
    export_sessions_from_import, plan_export, remove_from, reorder, ContentKind, ExportManifest,
    ExportPage, ExportPlan, ExportSession,
};
pub use image::GrayFrame;
pub use import::{import_session, import_volume, ImportPlan, ImportedCapture, ImportedSession};
pub use manifest::{CaptureEntry, DeviceManifest, SessionManifest};
pub use ocr::{
    OcrBlock, OcrFailureCode, OcrProvider, OcrResult, OcrSkipReason, OcrStatus,
    CONFIDENCE_PER_MILLE_MAX, MAX_LANGUAGE_ID_LEN, MAX_OCR_BLOCKS, MAX_OCR_DOCUMENT_BYTES,
    MAX_OCR_LANGUAGES, MAX_OCR_TEXT_CHARS, OCR_SCHEMA_PREFIX,
};
pub use pdf::export_pdf;
pub use processing::{
    apply_recipe, auto_balance, estimate_illumination, flatten_illumination, perspective,
    shadow_lift, DeterministicProcessor, ProcessedFrame, Processor,
};
pub use recipe::{OpKind, ProcessingRecipe, RecipeOp};
pub use version::{KNOWN_MINOR, SUPPORTED_MAJOR};
