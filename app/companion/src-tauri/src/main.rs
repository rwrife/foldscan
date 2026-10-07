#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FoldScan companion desktop shell (issues #39, #41, #43, #47, and #49).
//!
//! Scope: a buildable, linted, tested Tauri 2 shell that depends on the
//! `foldscan-domain` crate by path and exposes:
//! - [`companion_status`]: returns a versioned status document.
//! - [`import_volume_summary`]: runs bounded validation/import over a volume root
//!   and returns a structured, presentation-safe summary (including each
//!   verified capture, in manifest order, for the UI review plan) or a
//!   structured failure.
//! - [`preview_export_plan`]: validates one reviewed session's ordered
//!   capture selection against a *fresh* bounded import and returns the
//!   canonical export layout (planned files + manifest digest) without
//!   writing anything to disk.
//! - [`execute_export_plan`]: runs the durable filesystem executor for one
//!   reviewed session, copying originals from the verified import into a
//!   user-chosen destination parent directory (manifest last, rollback on
//!   failure, never overwriting an existing export root).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use foldscan_domain::executor::{execute_export, ExportSource};
use foldscan_domain::export::{
    export_sessions_from_import, plan_export, ContentKind, ExportManifest,
};
use foldscan_domain::import::{import_volume, ImportPlan, ImportedCapture, ImportedSession};
use foldscan_domain::ocr::{OcrBlock, OcrResult, OcrStatus};
use foldscan_domain::{load_exported_ocr, Category, DomainError};
use serde::Serialize;

/// Schema tag for the status document returned by [`companion_status`].
pub const STATUS_SCHEMA: &str = "foldscan.companion.status/0.1";

/// Schema tag for the import summary returned on success.
pub const IMPORT_SUMMARY_SCHEMA: &str = "foldscan.companion.import_summary/0.1";

/// Schema tag for the read-only export preview returned on success.
pub const EXPORT_PREVIEW_SCHEMA: &str = "foldscan.companion.export_preview/0.1";

/// Schema tag for the executed-export summary returned on success.
pub const EXPORT_EXECUTION_SCHEMA: &str = "foldscan.companion.export_execution/0.1";
pub const OCR_REVIEW_SCHEMA: &str = "foldscan.companion.ocr_review/0.1";
const MAX_REVIEW_DOCUMENTS: usize = 32;
const MAX_REVIEW_BLOCKS_PER_DOCUMENT: usize = 256;
const MAX_REVIEW_TEXT_BYTES: usize = 128 * 1024;

/// Number of leading hex chars of the manifest digest used to make a
/// deterministic, collision-resistant export directory name.
const EXPORT_DIR_DIGEST_PREFIX: usize = 12;

/// Versioned status document the UI renders as text. Keeping it structured
/// (never a free-text blob) means the UI layer can label each field for
/// assistive technology without string parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompanionStatus {
    /// Schema tag, e.g. `foldscan.companion.status/0.1`.
    pub schema: String,
    /// Version of this shell crate.
    pub app_version: String,
    /// Protocol `{major}.{minor}` the linked `foldscan-domain` crate was
    /// written against. Read from the domain crate's own constants, so the
    /// shell cannot drift into claiming a protocol the domain does not know.
    pub domain_protocol: String,
}

/// One verified capture exposed for read-only review-plan construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportedCaptureSummary {
    pub capture_id: String,
    pub bytes: u64,
}

/// Compact per-session metrics and manifest-ordered captures for UI review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportedSessionSummary {
    pub session_id: String,
    pub capture_count: usize,
    pub total_bytes: u64,
    pub captures: Vec<ImportedCaptureSummary>,
}

/// Structured summary of an imported volume root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportSummary {
    pub schema: String,
    pub device_id: String,
    pub firmware_version: String,
    pub session_count: usize,
    pub total_captures: usize,
    pub total_bytes: u64,
    pub sessions: Vec<ImportedSessionSummary>,
}

/// Structured failure response for frontend display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportFailure {
    pub category: String,
    pub message: String,
}

/// Result document for [`import_volume_summary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImportResult {
    Ok { summary: ImportSummary },
    Err { error: ImportFailure },
}

/// One canonical file in a read-only export preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPreviewFile {
    pub relative_path: String,
    pub capture_id: Option<String>,
    pub content_kind: String,
}

/// Canonical original-only export layout derived from a reviewed selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPreview {
    pub schema: String,
    pub manifest_schema: String,
    pub session_id: String,
    pub capture_ids: Vec<String>,
    pub files: Vec<ExportPreviewFile>,
    pub manifest_digest: String,
}

/// Tagged result document for [`preview_export_plan`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExportPreviewResult {
    Ok { preview: ExportPreview },
    Err { error: ImportFailure },
}

/// Summary of one executed export, returned on success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportExecutionSummary {
    pub schema: String,
    pub session_id: String,
    /// Capture ids exactly as exported (reviewed order).
    pub capture_ids: Vec<String>,
    /// Absolute path of the export root the executor created.
    pub root: String,
    /// Planned files finalized by the executor, including the manifest.
    pub files_written: usize,
    /// Lowercase-hex SHA-256 of the finalized `export.json` bytes.
    pub manifest_sha256: String,
    /// [`ExportManifest::digest`] of the finalized manifest document.
    pub manifest_digest: String,
}

/// Tagged result document for [`execute_export_plan`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExportExecutionResult {
    Ok { execution: ExportExecutionSummary },
    Err { error: ImportFailure },
}

/// Bounded UI projection without host paths or unreviewed content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcrReviewDocument {
    pub capture_id: String,
    pub requested_languages: Vec<String>,
    pub frame_width: u32,
    pub frame_height: u32,
    pub blocks: Vec<OcrBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcrReview {
    pub schema: String,
    pub documents: Vec<OcrReviewDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OcrReviewResult {
    Ok { review: OcrReview },
    Err { error: ImportFailure },
}

/// Caller supplies the reviewed digest from outside the export directory.
/// Refuse oversized display data rather than silently truncating it.
pub fn review_exported_ocr_path(root: &Path, reviewed_digest: &str) -> OcrReviewResult {
    let docs = match load_exported_ocr(root, reviewed_digest) {
        Ok(docs) => docs,
        Err(err) => {
            return OcrReviewResult::Err {
                error: ImportFailure {
                    category: category_tag(err.category).to_string(),
                    message: err.message,
                },
            }
        }
    };
    project_ocr_review(docs)
}

fn project_ocr_review(docs: Vec<OcrResult>) -> OcrReviewResult {
    if docs.len() > MAX_REVIEW_DOCUMENTS {
        return ocr_review_limit_error();
    }
    let mut total_text_bytes = 0usize;
    let mut projected = Vec::with_capacity(docs.len());
    for doc in docs {
        let OcrStatus::Completed {
            frame_width,
            frame_height,
            blocks,
        } = doc.status
        else {
            return OcrReviewResult::Err {
                error: ImportFailure {
                    category: "internal_error".into(),
                    message: "non-completed OCR document returned by loader".into(),
                },
            };
        };
        total_text_bytes = total_text_bytes
            .saturating_add(blocks.iter().map(|block| block.text.len()).sum::<usize>());
        if blocks.len() > MAX_REVIEW_BLOCKS_PER_DOCUMENT || total_text_bytes > MAX_REVIEW_TEXT_BYTES
        {
            return ocr_review_limit_error();
        }
        projected.push(OcrReviewDocument {
            capture_id: doc.capture_id,
            requested_languages: doc.requested_languages,
            frame_width,
            frame_height,
            blocks,
        });
    }
    OcrReviewResult::Ok {
        review: OcrReview {
            schema: OCR_REVIEW_SCHEMA.into(),
            documents: projected,
        },
    }
}

fn ocr_review_limit_error() -> OcrReviewResult {
    OcrReviewResult::Err {
        error: ImportFailure {
            category: "invalid_request".into(),
            message: "OCR review exceeds companion display limits (32 documents, 256 blocks per document, 128 KiB total text)".into(),
        },
    }
}

fn content_kind_tag(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Original => "original",
        ContentKind::Derivative => "derivative",
        ContentKind::Document => "document",
        ContentKind::Ocr => "ocr",
        ContentKind::OcrText => "ocr_text",
        ContentKind::Recipe => "recipe",
        ContentKind::Manifest => "manifest",
    }
}

/// Build the status document from compile-time crate metadata and the domain
/// crate's protocol constants.
pub fn status() -> CompanionStatus {
    CompanionStatus {
        schema: STATUS_SCHEMA.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        domain_protocol: format!(
            "{}.{}",
            foldscan_domain::version::SUPPORTED_MAJOR,
            foldscan_domain::version::KNOWN_MINOR
        ),
    }
}

fn category_tag(cat: Category) -> &'static str {
    match cat {
        Category::UnsupportedVersion => "unsupported_version",
        Category::InvalidRequest => "invalid_request",
        Category::ChecksumMismatch => "checksum_mismatch",
        Category::StorageUnavailable => "storage_unavailable",
        Category::Cancelled => "cancelled",
        Category::InternalError => "internal_error",
    }
}

pub fn summarize_plan(plan: &ImportPlan) -> ImportSummary {
    let mut total_captures = 0usize;
    let mut total_bytes = 0u64;
    let mut sessions = Vec::with_capacity(plan.sessions.len());

    for s in &plan.sessions {
        let count = s.captures.len();
        let bytes: u64 = s.captures.iter().map(|c| c.bytes).sum();
        let captures = s
            .captures
            .iter()
            .map(|capture| ImportedCaptureSummary {
                capture_id: capture.capture_id.clone(),
                bytes: capture.bytes,
            })
            .collect();
        total_captures = total_captures.saturating_add(count);
        total_bytes = total_bytes.saturating_add(bytes);
        sessions.push(ImportedSessionSummary {
            session_id: s.session_id.clone(),
            capture_count: count,
            total_bytes: bytes,
            captures,
        });
    }

    ImportSummary {
        schema: IMPORT_SUMMARY_SCHEMA.to_string(),
        device_id: plan.device_id.clone(),
        firmware_version: plan.firmware_version.clone(),
        session_count: plan.sessions.len(),
        total_captures,
        total_bytes,
        sessions,
    }
}

/// Execute a bounded import query on `volume_root`. Returns a structured outcome
/// so the caller never deals with opaque error codes.
pub fn import_volume_path(volume_root: &Path) -> ImportResult {
    match import_volume(volume_root) {
        Ok(plan) => ImportResult::Ok {
            summary: summarize_plan(&plan),
        },
        Err(err) => ImportResult::Err {
            error: ImportFailure {
                category: category_tag(err.category).to_string(),
                message: err.message,
            },
        },
    }
}

fn invalid_request(message: impl Into<String>) -> DomainError {
    DomainError::invalid_request(message.into())
}

fn map_preview_error(err: DomainError) -> ExportPreviewResult {
    ExportPreviewResult::Err {
        error: ImportFailure {
            category: category_tag(err.category).to_string(),
            message: err.message,
        },
    }
}

fn select_session<'a>(
    sessions: &'a [ImportedSession],
    session_id: &str,
) -> Result<&'a ImportedSession, DomainError> {
    sessions
        .iter()
        .find(|session| session.session_id == session_id)
        .ok_or_else(|| {
            invalid_request(format!(
                "review session {} not found in import summary",
                session_id
            ))
        })
}

/// Validate an ordered capture selection against one imported session and
/// return the selected captures in requested order. Shared by the read-only
/// preview and the real executor bridge so both enforce identical review
/// rules (non-empty, no duplicates, every id present in the import).
fn select_captures(
    imported: &ImportPlan,
    session_id: &str,
    capture_ids: &[String],
) -> Result<Vec<ImportedCapture>, DomainError> {
    if session_id.trim().is_empty() {
        return Err(invalid_request("session_id is empty"));
    }
    if capture_ids.is_empty() {
        return Err(invalid_request(
            "at least one capture must remain in the reviewed export selection",
        ));
    }

    let session = select_session(&imported.sessions, session_id)?;
    let capture_index: HashMap<&str, &ImportedCapture> = session
        .captures
        .iter()
        .map(|capture| (capture.capture_id.as_str(), capture))
        .collect();

    let mut seen = HashSet::new();
    let mut selected = Vec::with_capacity(capture_ids.len());
    for capture_id in capture_ids {
        if !seen.insert(capture_id.as_str()) {
            return Err(invalid_request(format!(
                "reviewed capture list contains duplicate capture_id {}",
                capture_id
            )));
        }
        let capture = capture_index.get(capture_id.as_str()).ok_or_else(|| {
            invalid_request(format!(
                "reviewed capture {} is not present in imported session {}",
                capture_id, session_id
            ))
        })?;
        selected.push((*capture).clone());
    }
    Ok(selected)
}

/// Rebuild the canonical original-only export session from selected captures
/// and plan/manifest it. Deterministic and shared by preview and execution.
fn canonical_original_plan(
    session_id: &str,
    selected: &[ImportedCapture],
) -> Result<(foldscan_domain::export::ExportPlan, ExportManifest), DomainError> {
    let mut export_sessions = export_sessions_from_import(&[ImportedSession {
        session_id: session_id.to_string(),
        captures: selected.to_vec(),
    }]);
    let session = export_sessions
        .first_mut()
        .ok_or_else(|| DomainError::internal("export session synthesis produced no sessions"))?;
    session.document = None;
    session.ocr.clear();
    session.ocr_text = false;

    let plan = plan_export(&export_sessions, &[])?;
    let manifest = ExportManifest::from_plan(&plan, session_id)?;
    Ok((plan, manifest))
}

fn build_export_preview(
    imported: &ImportPlan,
    session_id: &str,
    capture_ids: &[String],
) -> Result<ExportPreview, DomainError> {
    let selected = select_captures(imported, session_id, capture_ids)?;
    let (plan, manifest) = canonical_original_plan(session_id, &selected)?;

    let files = plan
        .files
        .iter()
        .map(|file| {
            let content_kind = content_kind_tag(file.content_kind).to_string();
            ExportPreviewFile {
                relative_path: file.relative_path.clone(),
                capture_id: file.capture_id.clone(),
                content_kind,
            }
        })
        .collect();

    Ok(ExportPreview {
        schema: EXPORT_PREVIEW_SCHEMA.to_string(),
        manifest_schema: manifest.schema.clone(),
        session_id: session_id.to_string(),
        capture_ids: capture_ids.to_vec(),
        files,
        manifest_digest: manifest.digest(),
    })
}

pub fn preview_export_plan_path(
    volume_root: &Path,
    session_id: &str,
    capture_ids: &[String],
) -> ExportPreviewResult {
    let imported = match import_volume(volume_root) {
        Ok(plan) => plan,
        Err(err) => return map_preview_error(err),
    };

    match build_export_preview(&imported, session_id.trim(), capture_ids) {
        Ok(preview) => ExportPreviewResult::Ok { preview },
        Err(err) => map_preview_error(err),
    }
}

fn canonical_pdf_plan(
    session_id: &str,
    selected: &[ImportedCapture],
) -> Result<(foldscan_domain::export::ExportPlan, ExportManifest, Vec<u8>), DomainError> {
    for capture in selected {
        if capture.media_type != "image/png" {
            return Err(invalid_request(format!(
                "PDF export requires image/png captures; capture {} declared media_type {}",
                capture.capture_id, capture.media_type
            )));
        }
    }

    // Limit retained decoded frames separately from the writer's output bound.
    // One decode is already dimension-bounded by the domain codec.
    const MAX_PDF_FRAME_BYTES: usize = 64 * 1024 * 1024;
    let mut retained_pixels = 0usize;
    let mut frames = Vec::with_capacity(selected.len());
    let mut doc_pages = Vec::with_capacity(selected.len());
    for capture in selected {
        use std::io::Read;
        let metadata = std::fs::symlink_metadata(&capture.host_path)
            .map_err(|_| DomainError::storage_unavailable("cannot inspect PDF source"))?;
        if !metadata.file_type().is_file() {
            return Err(invalid_request(
                "PDF source must be a regular non-symlink file",
            ));
        }
        let file = std::fs::File::open(&capture.host_path)
            .map_err(|_| DomainError::storage_unavailable("cannot open PDF source"))?;
        let mut bytes = Vec::new();
        file.take(
            capture
                .bytes
                .min(foldscan_domain::limits::MAX_CAPTURE_BYTES)
                + 1,
        )
        .read_to_end(&mut bytes)
        .map_err(|e| {
            DomainError::storage_unavailable(format!(
                "cannot read capture {} for PDF export: {}",
                capture.capture_id, e
            ))
        })?;
        if bytes.len() as u64 != capture.bytes {
            return Err(invalid_request(format!(
                "capture {} file size changed before PDF export",
                capture.capture_id
            )));
        }
        if foldscan_domain::checksum::sha256_hex(&bytes) != capture.sha256 {
            return Err(DomainError::checksum_mismatch(format!(
                "capture {} checksum mismatch before PDF export",
                capture.capture_id
            )));
        }
        let frame = foldscan_domain::png::decode_png(&bytes).map_err(|e| {
            DomainError::invalid_request(format!(
                "capture {} PNG decode failed for PDF export: {}",
                capture.capture_id, e.message
            ))
        })?;
        retained_pixels = retained_pixels
            .checked_add(frame.pixels.len())
            .ok_or_else(|| invalid_request("PDF decoded frame budget overflow"))?;
        if retained_pixels > MAX_PDF_FRAME_BYTES {
            return Err(invalid_request(
                "PDF decoded frames exceed 64 MiB memory budget",
            ));
        }
        doc_pages.push(foldscan_domain::DocumentPage {
            capture_id: capture.capture_id.clone(),
            width_px: frame.width,
            height_px: frame.height,
        });
        frames.push(frame);
    }

    let pdf_bytes = foldscan_domain::export_pdf(&frames)?;
    let doc = foldscan_domain::SessionDocument {
        media_type: "application/pdf".to_string(),
        sha256: foldscan_domain::checksum::sha256_hex(&pdf_bytes),
        bytes: pdf_bytes.len() as u64,
        pages: doc_pages,
    };

    let mut export_sessions = export_sessions_from_import(&[ImportedSession {
        session_id: session_id.to_string(),
        captures: selected.to_vec(),
    }]);
    let session = export_sessions
        .first_mut()
        .ok_or_else(|| DomainError::internal("export session synthesis produced no sessions"))?;
    session.document = Some(doc);
    session.ocr.clear();
    session.ocr_text = false;

    let plan = plan_export(&export_sessions, &[])?;
    let manifest = ExportManifest::from_plan(&plan, session_id)?;
    Ok((plan, manifest, pdf_bytes))
}

fn build_pdf_export_preview(
    imported: &ImportPlan,
    session_id: &str,
    capture_ids: &[String],
) -> Result<ExportPreview, DomainError> {
    let selected = select_captures(imported, session_id, capture_ids)?;
    let (plan, manifest, _) = canonical_pdf_plan(session_id, &selected)?;

    let files = plan
        .files
        .iter()
        .map(|file| {
            let content_kind = content_kind_tag(file.content_kind).to_string();
            ExportPreviewFile {
                relative_path: file.relative_path.clone(),
                capture_id: file.capture_id.clone(),
                content_kind,
            }
        })
        .collect();

    Ok(ExportPreview {
        schema: EXPORT_PREVIEW_SCHEMA.to_string(),
        manifest_schema: manifest.schema.clone(),
        session_id: session_id.to_string(),
        capture_ids: capture_ids.to_vec(),
        files,
        manifest_digest: manifest.digest(),
    })
}

pub fn preview_pdf_export_plan_path(
    volume_root: &Path,
    session_id: &str,
    capture_ids: &[String],
) -> ExportPreviewResult {
    let imported = match import_volume(volume_root) {
        Ok(plan) => plan,
        Err(err) => return map_preview_error(err),
    };

    match build_pdf_export_preview(&imported, session_id.trim(), capture_ids) {
        Ok(preview) => ExportPreviewResult::Ok { preview },
        Err(err) => map_preview_error(err),
    }
}

fn map_execution_error(err: DomainError) -> ExportExecutionResult {
    ExportExecutionResult::Err {
        error: ImportFailure {
            category: category_tag(err.category).to_string(),
            message: err.message,
        },
    }
}

fn run_export(
    imported: &ImportPlan,
    destination_parent: &Path,
    session_id: &str,
    capture_ids: &[String],
) -> Result<ExportExecutionSummary, DomainError> {
    if destination_parent.as_os_str().is_empty() {
        return Err(invalid_request(
            "destination parent directory is empty; choose a folder first",
        ));
    }

    // Same review rules as the preview: the selection must be a non-empty,
    // duplicate-free subset of a freshly re-verified import. The executor
    // then re-reads originals from these verified host paths only.
    let selected = select_captures(imported, session_id, capture_ids)?;
    let (plan, manifest) = canonical_original_plan(session_id, &selected)?;
    finish_export(
        destination_parent,
        session_id,
        capture_ids,
        &selected,
        &plan,
        &manifest,
        HashMap::new(),
    )
}

fn finish_export(
    destination_parent: &Path,
    session_id: &str,
    capture_ids: &[String],
    selected: &[ImportedCapture],
    plan: &foldscan_domain::ExportPlan,
    manifest: &ExportManifest,
    mut sources: HashMap<String, ExportSource>,
) -> Result<ExportExecutionSummary, DomainError> {
    let digest = manifest.digest();

    // Deterministic, collision-resistant destination: session id (already
    // restricted by the planner to path-safe characters) plus the leading
    // manifest-digest prefix, so re-running the identical reviewed export
    // onto the same parent targets the same root and is refused rather
    // than silently duplicating bytes.
    let root = destination_parent.join(format!(
        "foldscan-export-{}-{}",
        session_id,
        &digest[..EXPORT_DIR_DIGEST_PREFIX]
    ));

    for file in &plan.files {
        if file.content_kind == ContentKind::Original {
            let cid = file
                .capture_id
                .as_deref()
                .ok_or_else(|| DomainError::internal("planned original file without capture_id"))?;
            let capture = selected
                .iter()
                .find(|capture| capture.capture_id == cid)
                .ok_or_else(|| {
                    DomainError::internal("planned original references an unselected capture")
                })?;
            sources.insert(
                file.relative_path.clone(),
                ExportSource::file(capture.host_path.clone()),
            );
        }
    }

    let exec = execute_export(plan, &root, &sources, manifest)?;
    Ok(ExportExecutionSummary {
        schema: EXPORT_EXECUTION_SCHEMA.to_string(),
        session_id: session_id.to_string(),
        capture_ids: capture_ids.to_vec(),
        root: exec.root.display().to_string(),
        files_written: exec.files_written,
        manifest_sha256: exec.manifest_sha256,
        manifest_digest: digest,
    })
}

/// Import, review-validate, and execute one original-only export into
/// `destination_parent`. All refusal paths (bad volume, bad selection,
/// existing destination root) return structured errors; a successful run
/// returns the executor's summary of what was finalized.
pub fn execute_export_plan_path(
    volume_root: &Path,
    destination_parent: &Path,
    session_id: &str,
    capture_ids: &[String],
) -> ExportExecutionResult {
    let imported = match import_volume(volume_root) {
        Ok(plan) => plan,
        Err(err) => return map_execution_error(err),
    };

    match run_export(
        &imported,
        destination_parent,
        session_id.trim(),
        capture_ids,
    ) {
        Ok(execution) => ExportExecutionResult::Ok { execution },
        Err(err) => map_execution_error(err),
    }
}

/// Explicit PNG-only PDF export. Decode/assemble before any destination write.
pub fn execute_pdf_export_plan_path(
    volume_root: &Path,
    destination_parent: &Path,
    session_id: &str,
    capture_ids: &[String],
) -> ExportExecutionResult {
    let result = (|| {
        if destination_parent.as_os_str().is_empty() {
            return Err(invalid_request("destination parent directory is empty"));
        }
        let imported = import_volume(volume_root)?;
        let selected = select_captures(&imported, session_id.trim(), capture_ids)?;
        let (plan, manifest, pdf) = canonical_pdf_plan(session_id.trim(), &selected)?;
        let document = plan
            .files
            .iter()
            .find(|f| f.content_kind == ContentKind::Document)
            .ok_or_else(|| DomainError::internal("PDF plan missing document"))?;
        let sources = HashMap::from([(document.relative_path.clone(), ExportSource::bytes(pdf))]);
        finish_export(
            destination_parent,
            session_id.trim(),
            capture_ids,
            &selected,
            &plan,
            &manifest,
            sources,
        )
    })();
    match result {
        Ok(execution) => ExportExecutionResult::Ok { execution },
        Err(err) => map_execution_error(err),
    }
}

#[tauri::command]
fn execute_pdf_export_plan(
    volume_path: String,
    destination_path: String,
    session_id: String,
    capture_ids: Vec<String>,
) -> ExportExecutionResult {
    execute_pdf_export_plan_path(
        Path::new(volume_path.trim()),
        Path::new(destination_path.trim()),
        session_id.trim(),
        &capture_ids,
    )
}

#[tauri::command]
fn preview_pdf_export_plan(
    volume_path: String,
    session_id: String,
    capture_ids: Vec<String>,
) -> ExportPreviewResult {
    preview_pdf_export_plan_path(
        Path::new(volume_path.trim()),
        session_id.trim(),
        &capture_ids,
    )
}

/// The command registered by the scaffold. It performs no I/O and
/// takes no arguments, so the capability surface stays `core:default`.
#[tauri::command]
fn companion_status() -> CompanionStatus {
    status()
}

/// Bounded removable-media import probe command.
#[tauri::command]
fn import_volume_summary(volume_path: String) -> ImportResult {
    import_volume_path(Path::new(volume_path.trim()))
}

/// Read-only export preview command for one reviewed session.
#[tauri::command]
fn preview_export_plan(
    volume_path: String,
    session_id: String,
    capture_ids: Vec<String>,
) -> ExportPreviewResult {
    preview_export_plan_path(
        Path::new(volume_path.trim()),
        session_id.trim(),
        &capture_ids,
    )
}

/// Durable export execution command for one reviewed session.
#[tauri::command]
fn execute_export_plan(
    volume_path: String,
    destination_path: String,
    session_id: String,
    capture_ids: Vec<String>,
) -> ExportExecutionResult {
    execute_export_plan_path(
        Path::new(volume_path.trim()),
        Path::new(destination_path.trim()),
        session_id.trim(),
        &capture_ids,
    )
}

/// Digest-bound, read-only inspection of exported OCR documents.
#[tauri::command]
fn review_exported_ocr(export_path: String, reviewed_digest: String) -> OcrReviewResult {
    review_exported_ocr_path(Path::new(export_path.trim()), reviewed_digest.trim())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            companion_status,
            import_volume_summary,
            preview_export_plan,
            execute_export_plan,
            preview_pdf_export_plan,
            execute_pdf_export_plan,
            review_exported_ocr
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use foldscan_domain::checksum::sha256_hex;
    use std::fs;
    use tempfile::TempDir;

    const FAKE_JPEG: &[u8] = b"\xFF\xD8\xFF\xE0synthetic-page-image\xFF\xD9";

    fn create_valid_fixture(root: &Path) {
        let foldscan = root.join("FOLDSCAN");
        let session = foldscan.join("sessions").join("sess-001");
        fs::create_dir_all(session.join("captures")).unwrap();

        fs::write(
            foldscan.join("device.json"),
            r#"{"protocol":{"major":0,"minor":1},"device_id":"fs-test-01","firmware_version":"0.1.0-dev","capabilities":["capture"]}"#,
        )
        .unwrap();

        fs::write(session.join("captures").join("cap-1.jpg"), FAKE_JPEG).unwrap();
        let sha = sha256_hex(FAKE_JPEG);
        let bytes = FAKE_JPEG.len() as u64;
        fs::write(
            session.join("session.json"),
            format!(
                r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","created_at":null,"clock_state":"unsynced","captures":[{{"capture_id":"cap-1","relative_path":"captures/cap-1.jpg","media_type":"image/jpeg","bytes":{},"sha256":"{}","width_px":1600,"height_px":1200,"orientation":1,"captured_at":null}}]}}"#,
                bytes, sha
            ),
        )
        .unwrap();
    }

    fn create_preview_fixture(root: &Path) {
        create_valid_fixture(root);
        let session = root.join("FOLDSCAN/sessions/sess-001");
        let second = b"\xFF\xD8\xFF\xE0second-page-image\xFF\xD9";
        fs::write(session.join("captures/cap-2.jpg"), second).unwrap();
        fs::write(
            session.join("session.json"),
            format!(
                r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","created_at":null,"clock_state":"unsynced","captures":[{{"capture_id":"cap-1","relative_path":"captures/cap-1.jpg","media_type":"image/jpeg","bytes":{},"sha256":"{}","width_px":1600,"height_px":1200,"orientation":1,"captured_at":null}},{{"capture_id":"cap-2","relative_path":"captures/cap-2.jpg","media_type":"image/jpeg","bytes":{},"sha256":"{}","width_px":1600,"height_px":1200,"orientation":1,"captured_at":null}}]}}"#,
                FAKE_JPEG.len(),
                sha256_hex(FAKE_JPEG),
                second.len(),
                sha256_hex(second)
            ),
        )
        .unwrap();
    }

    fn create_png_fixture(root: &Path) -> Vec<(String, Vec<u8>, u32, u32)> {
        create_valid_fixture(root);
        let session = root.join("FOLDSCAN/sessions/sess-001");
        let frames = [
            foldscan_domain::GrayFrame::from_pixels(3, 2, vec![0, 30, 60, 90, 120, 150]).unwrap(),
            foldscan_domain::GrayFrame::from_pixels(2, 3, vec![10, 20, 30, 40, 50, 60]).unwrap(),
        ];
        let captures: Vec<_> = frames
            .iter()
            .enumerate()
            .map(|(i, frame)| {
                let id = format!("cap-{}", i + 1);
                let bytes = foldscan_domain::png::encode_png(frame).unwrap();
                fs::write(session.join(format!("captures/{id}.png")), &bytes).unwrap();
                (id, bytes, frame.width, frame.height)
            })
            .collect();
        let entries: Vec<_> = captures.iter().map(|(id, bytes, w, h)| serde_json::json!({
            "capture_id": id, "relative_path": format!("captures/{id}.png"), "media_type": "image/png",
            "bytes": bytes.len(), "sha256": sha256_hex(bytes), "width_px": w, "height_px": h,
            "orientation": 1, "captured_at": null
        })).collect();
        fs::write(
            session.join("session.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": "foldscan.session/0.1", "session_id": "sess-001",
                "created_at": null, "clock_state": "unsynced", "captures": entries
            }))
            .unwrap(),
        )
        .unwrap();
        captures
    }

    #[test]
    fn preview_pdf_export_lists_document_file_and_validates_png_only() {
        let volume = TempDir::new().unwrap();
        create_png_fixture(volume.path());
        let order = vec!["cap-1".to_string(), "cap-2".to_string()];
        let ExportPreviewResult::Ok { preview } =
            preview_pdf_export_plan_path(volume.path(), "sess-001", &order)
        else {
            panic!("preview must succeed for valid PNG session")
        };
        assert_eq!(preview.session_id, "sess-001");
        assert_eq!(preview.capture_ids, order);
        let doc_file = preview
            .files
            .iter()
            .find(|f| f.content_kind == "document")
            .expect("planned document");
        assert_eq!(doc_file.relative_path, "documents/sess-001/session.pdf");
        assert!(doc_file.capture_id.is_none());

        // JPEG volume rejected during preview
        let jpeg_volume = TempDir::new().unwrap();
        create_valid_fixture(jpeg_volume.path());
        let err =
            preview_pdf_export_plan_path(jpeg_volume.path(), "sess-001", &["cap-1".to_string()]);
        assert!(
            matches!(err, ExportPreviewResult::Err { error } if error.category == "invalid_request")
        );
    }

    #[test]
    fn pdf_export_binds_reviewed_order_dimensions_and_preserves_originals() {
        let volume = TempDir::new().unwrap();
        let destination = TempDir::new().unwrap();
        let captures = create_png_fixture(volume.path());
        let order = vec!["cap-2".to_string(), "cap-1".to_string()];
        let ExportExecutionResult::Ok { execution } =
            execute_pdf_export_plan_path(volume.path(), destination.path(), "sess-001", &order)
        else {
            panic!("PNG PDF export must succeed")
        };
        assert_eq!(execution.capture_ids, order);
        let root = Path::new(&execution.root);
        let pdf = fs::read(root.join("documents/sess-001/session.pdf")).unwrap();
        let expected = foldscan_domain::export_pdf(&[
            foldscan_domain::png::decode_png(&captures[1].1).unwrap(),
            foldscan_domain::png::decode_png(&captures[0].1).unwrap(),
        ])
        .unwrap();
        assert_eq!(pdf, expected);
        let manifest = foldscan_domain::ExportManifest::from_json_bytes(
            &fs::read(root.join("export.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.digest(), execution.manifest_digest);
        let ExportPreviewResult::Ok { preview } =
            preview_pdf_export_plan_path(volume.path(), "sess-001", &order)
        else {
            panic!("PDF preview")
        };
        assert_eq!(preview.manifest_digest, manifest.digest());
        assert_eq!(execution.files_written, 4);
        let document = manifest.document.as_ref().unwrap();
        assert_eq!(document.sha256, sha256_hex(&pdf));
        assert_eq!(document.bytes, pdf.len() as u64);
        assert_eq!(
            document
                .pages
                .iter()
                .map(|p| (p.capture_id.as_str(), p.width_px, p.height_px))
                .collect::<Vec<_>>(),
            vec![("cap-2", 2, 3), ("cap-1", 3, 2)]
        );
        for (id, bytes, _, _) in &captures {
            assert_eq!(
                fs::read(
                    volume
                        .path()
                        .join(format!("FOLDSCAN/sessions/sess-001/captures/{id}.png"))
                )
                .unwrap(),
                *bytes
            );
            assert_eq!(
                fs::read(root.join(format!("originals/sess-001/{id}.png"))).unwrap(),
                *bytes
            );
        }
        assert!(matches!(
            execute_pdf_export_plan_path(volume.path(), destination.path(), "sess-001", &order),
            ExportExecutionResult::Err { .. }
        ));
    }

    #[test]
    fn pdf_export_rejects_jpeg_and_corrupt_png_before_destination_mutation() {
        let volume = TempDir::new().unwrap();
        let destination = TempDir::new().unwrap();
        create_valid_fixture(volume.path());
        let ids = vec!["cap-1".to_string()];
        let result =
            execute_pdf_export_plan_path(volume.path(), destination.path(), "sess-001", &ids);
        assert!(
            matches!(result, ExportExecutionResult::Err { error } if error.category == "invalid_request")
        );
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
        let captures = create_png_fixture(volume.path());
        let png = volume
            .path()
            .join("FOLDSCAN/sessions/sess-001/captures/cap-1.png");
        let mut corrupt = captures[0].1.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        fs::write(&png, &corrupt).unwrap();
        // Update the declaration so import passes; exercise the PNG decoder,
        // not just the import checksum guard.
        let session_path = volume
            .path()
            .join("FOLDSCAN/sessions/sess-001/session.json");
        let mut session: serde_json::Value =
            serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
        session["captures"][0]["sha256"] = serde_json::json!(sha256_hex(&corrupt));
        fs::write(session_path, serde_json::to_vec(&session).unwrap()).unwrap();
        let result =
            execute_pdf_export_plan_path(volume.path(), destination.path(), "sess-001", &ids);
        assert!(matches!(result, ExportExecutionResult::Err { .. }));
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);

        for fixture in ["rgb_4x4.png", "palette_4x4.png"] {
            // Unsupported color PNGs must fail before export.
            let rgb_bytes = fs::read(format!("../../domain/tests/fixtures/png/{fixture}")).unwrap();
            let session_path = volume
                .path()
                .join("FOLDSCAN/sessions/sess-001/session.json");
            fs::write(
                volume
                    .path()
                    .join("FOLDSCAN/sessions/sess-001/captures/cap-1.png"),
                &rgb_bytes,
            )
            .unwrap();
            let mut session: serde_json::Value =
                serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
            session["captures"][0]["bytes"] = serde_json::json!(rgb_bytes.len());
            session["captures"][0]["sha256"] = serde_json::json!(sha256_hex(&rgb_bytes));
            fs::write(session_path, serde_json::to_vec(&session).unwrap()).unwrap();
            let result =
                execute_pdf_export_plan_path(volume.path(), destination.path(), "sess-001", &ids);
            assert!(
                matches!(result, ExportExecutionResult::Err { error } if error.category == "invalid_request")
            );
            assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn pdf_export_rolls_back_if_an_original_disappears_after_planning() {
        let volume = TempDir::new().unwrap();
        let destination = TempDir::new().unwrap();
        let captures = create_png_fixture(volume.path());
        let imported = import_volume(volume.path()).unwrap();
        let order = vec!["cap-2".to_string(), "cap-1".to_string()];
        let selected = select_captures(&imported, "sess-001", &order).unwrap();
        let (plan, manifest, pdf) = canonical_pdf_plan("sess-001", &selected).unwrap();
        let document = plan
            .files
            .iter()
            .find(|f| f.content_kind == ContentKind::Document)
            .unwrap();
        let sources = HashMap::from([(document.relative_path.clone(), ExportSource::bytes(pdf))]);
        // cap-2 is written before the missing cap-1; failure must remove it too.
        fs::remove_file(&selected[1].host_path).unwrap();
        assert!(finish_export(
            destination.path(),
            "sess-001",
            &order,
            &selected,
            &plan,
            &manifest,
            sources
        )
        .is_err());
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
        assert_eq!(fs::read(&selected[0].host_path).unwrap(), captures[1].1);
        fs::write(&selected[1].host_path, &captures[0].1).unwrap();
        assert!(matches!(
            execute_pdf_export_plan_path(volume.path(), destination.path(), "sess-001", &order),
            ExportExecutionResult::Ok { .. }
        ));
    }

    #[test]
    fn pdf_plan_rechecks_checksum_and_refuses_a_changed_source() {
        let volume = TempDir::new().unwrap();
        let captures = create_png_fixture(volume.path());
        let imported = import_volume(volume.path()).unwrap();
        let selected = select_captures(&imported, "sess-001", &["cap-1".to_string()]).unwrap();
        let mut changed = captures[0].1.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        fs::write(&selected[0].host_path, &changed).unwrap();
        assert!(
            matches!(canonical_pdf_plan("sess-001", &selected), Err(err) if err.category == Category::ChecksumMismatch)
        );
        assert_eq!(fs::read(&selected[0].host_path).unwrap(), changed);
    }

    #[test]
    fn status_uses_status_schema_and_shell_version() {
        let s = status();
        assert_eq!(s.schema, STATUS_SCHEMA);
        assert_eq!(s.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn status_reports_the_domain_protocol_constants_verbatim() {
        let s = status();
        assert_eq!(
            s.domain_protocol,
            format!(
                "{}.{}",
                foldscan_domain::version::SUPPORTED_MAJOR,
                foldscan_domain::version::KNOWN_MINOR
            )
        );
        assert_eq!(s.domain_protocol, "0.1");
    }

    #[test]
    fn status_serializes_to_the_pinned_json_shape() {
        let value = serde_json::to_value(status()).expect("serialization is infallible");
        let obj = value.as_object().expect("status is an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["app_version", "domain_protocol", "schema"]);
    }

    #[test]
    fn import_volume_path_returns_structured_summary_on_valid_fixture() {
        let tmp = TempDir::new().unwrap();
        create_valid_fixture(tmp.path());

        let res = import_volume_path(tmp.path());
        match res {
            ImportResult::Ok { summary } => {
                assert_eq!(summary.schema, IMPORT_SUMMARY_SCHEMA);
                assert_eq!(summary.device_id, "fs-test-01");
                assert_eq!(summary.firmware_version, "0.1.0-dev");
                assert_eq!(summary.session_count, 1);
                assert_eq!(summary.total_captures, 1);
                assert_eq!(summary.total_bytes, FAKE_JPEG.len() as u64);
                assert_eq!(summary.sessions.len(), 1);
                assert_eq!(summary.sessions[0].session_id, "sess-001");
                assert_eq!(summary.sessions[0].capture_count, 1);
                assert_eq!(summary.sessions[0].total_bytes, FAKE_JPEG.len() as u64);
                assert_eq!(summary.sessions[0].captures.len(), 1);
                assert_eq!(summary.sessions[0].captures[0].capture_id, "cap-1");
                assert_eq!(
                    summary.sessions[0].captures[0].bytes,
                    FAKE_JPEG.len() as u64
                );
            }
            ImportResult::Err { error } => {
                panic!("expected success, got error: {:?}", error);
            }
        }
    }

    #[test]
    fn import_volume_path_returns_structured_failure_on_missing_dir() {
        let tmp = TempDir::new().unwrap();
        let res = import_volume_path(&tmp.path().join("does-not-exist"));
        match res {
            ImportResult::Ok { .. } => panic!("expected failure on missing directory"),
            ImportResult::Err { error } => {
                assert_eq!(error.category, "storage_unavailable");
                assert!(!error.message.is_empty());
            }
        }
    }

    #[test]
    fn session_summary_serializes_the_pinned_capture_list_shape() {
        let summary = ImportSummary {
            schema: IMPORT_SUMMARY_SCHEMA.to_string(),
            device_id: "fs-1".to_string(),
            firmware_version: "0.1.0".to_string(),
            session_count: 1,
            total_captures: 2,
            total_bytes: 30,
            sessions: vec![ImportedSessionSummary {
                session_id: "sess-1".to_string(),
                capture_count: 2,
                total_bytes: 30,
                captures: vec![
                    ImportedCaptureSummary {
                        capture_id: "cap-a".to_string(),
                        bytes: 10,
                    },
                    ImportedCaptureSummary {
                        capture_id: "cap-b".to_string(),
                        bytes: 20,
                    },
                ],
            }],
        };
        let value = serde_json::to_value(summary).expect("serialization is infallible");
        let session = &value["sessions"][0];
        assert_eq!(session["session_id"], "sess-1");
        assert_eq!(session["capture_count"], 2);
        assert_eq!(session["total_bytes"], 30);
        // The review plan consumes `captures` in array order; pin the shape so
        // the UI contract cannot drift silently.
        assert!(session["captures"].is_array());
        let mut keys: Vec<&str> = session["captures"][0]
            .as_object()
            .expect("capture is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["bytes", "capture_id"]);
        assert_eq!(session["captures"][0]["capture_id"], "cap-a");
        assert_eq!(session["captures"][0]["bytes"], 10);
        assert_eq!(session["captures"][1]["capture_id"], "cap-b");
    }

    #[test]
    fn import_result_serializes_tagged_json() {
        let ok_result = ImportResult::Ok {
            summary: ImportSummary {
                schema: IMPORT_SUMMARY_SCHEMA.to_string(),
                device_id: "fs-1".to_string(),
                firmware_version: "0.1.0".to_string(),
                session_count: 0,
                total_captures: 0,
                total_bytes: 0,
                sessions: Vec::new(),
            },
        };
        let val = serde_json::to_value(ok_result).unwrap();
        assert_eq!(val["status"], "ok");
        assert_eq!(val["summary"]["device_id"], "fs-1");

        let err_result = ImportResult::Err {
            error: ImportFailure {
                category: "invalid_request".to_string(),
                message: "corrupted manifest".to_string(),
            },
        };
        let err_val = serde_json::to_value(err_result).unwrap();
        assert_eq!(err_val["status"], "err");
        assert_eq!(err_val["error"]["category"], "invalid_request");
    }

    #[test]
    fn preview_export_plan_returns_canonical_original_layout_and_digest() {
        let tmp = TempDir::new().unwrap();
        create_preview_fixture(tmp.path());

        let result = preview_export_plan_path(
            tmp.path(),
            "sess-001",
            &["cap-2".to_string(), "cap-1".to_string()],
        );

        match result {
            ExportPreviewResult::Ok { preview } => {
                assert_eq!(preview.schema, EXPORT_PREVIEW_SCHEMA);
                assert_eq!(preview.manifest_schema, "foldscan.export/0.1");
                assert_eq!(preview.session_id, "sess-001");
                assert_eq!(preview.capture_ids, vec!["cap-2", "cap-1"]);
                assert_eq!(preview.files.len(), 3);
                assert_eq!(
                    preview.files[0].relative_path,
                    "originals/sess-001/cap-2.jpg"
                );
                assert_eq!(preview.files[0].capture_id.as_deref(), Some("cap-2"));
                assert_eq!(preview.files[0].content_kind, "original");
                assert_eq!(
                    preview.files[1].relative_path,
                    "originals/sess-001/cap-1.jpg"
                );
                assert_eq!(preview.files[1].capture_id.as_deref(), Some("cap-1"));
                assert_eq!(preview.files[1].content_kind, "original");
                assert_eq!(preview.files[2].relative_path, "export.json");
                assert!(preview.files[2].capture_id.is_none());
                assert_eq!(preview.files[2].content_kind, "manifest");
                assert_eq!(preview.manifest_digest.len(), 64);
                assert!(preview
                    .manifest_digest
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit()));
            }
            ExportPreviewResult::Err { error } => {
                panic!("expected preview success, got error: {:?}", error)
            }
        }
    }

    #[test]
    fn preview_export_plan_rejects_empty_or_invalid_selection() {
        let tmp = TempDir::new().unwrap();
        create_preview_fixture(tmp.path());

        let empty = preview_export_plan_path(tmp.path(), "sess-001", &[]);
        match empty {
            ExportPreviewResult::Ok { .. } => panic!("empty selection must fail"),
            ExportPreviewResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("at least one capture"));
            }
        }

        let duplicate = preview_export_plan_path(
            tmp.path(),
            "sess-001",
            &["cap-1".to_string(), "cap-1".to_string()],
        );
        match duplicate {
            ExportPreviewResult::Ok { .. } => panic!("duplicate selection must fail"),
            ExportPreviewResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("duplicate capture_id"));
            }
        }

        let unknown_capture =
            preview_export_plan_path(tmp.path(), "sess-001", &["cap-missing".to_string()]);
        match unknown_capture {
            ExportPreviewResult::Ok { .. } => panic!("unknown capture must fail"),
            ExportPreviewResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("is not present"));
            }
        }

        let unknown_session =
            preview_export_plan_path(tmp.path(), "sess-missing", &["cap-1".to_string()]);
        match unknown_session {
            ExportPreviewResult::Ok { .. } => panic!("unknown session must fail"),
            ExportPreviewResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("session"));
            }
        }
    }

    #[test]
    fn preview_result_serializes_tagged_json() {
        let ok = ExportPreviewResult::Ok {
            preview: ExportPreview {
                schema: EXPORT_PREVIEW_SCHEMA.to_string(),
                manifest_schema: "foldscan.export/0.1".to_string(),
                session_id: "sess-001".to_string(),
                capture_ids: vec!["cap-1".to_string()],
                files: vec![ExportPreviewFile {
                    relative_path: "originals/sess-001/cap-1.jpg".to_string(),
                    capture_id: Some("cap-1".to_string()),
                    content_kind: "original".to_string(),
                }],
                manifest_digest: "a".repeat(64),
            },
        };
        let ok_json = serde_json::to_value(ok).unwrap();
        assert_eq!(ok_json["status"], "ok");
        assert_eq!(ok_json["preview"]["schema"], EXPORT_PREVIEW_SCHEMA);

        let err = ExportPreviewResult::Err {
            error: ImportFailure {
                category: "invalid_request".to_string(),
                message: "bad request".to_string(),
            },
        };
        let err_json = serde_json::to_value(err).unwrap();
        assert_eq!(err_json["status"], "err");
        assert_eq!(err_json["error"]["category"], "invalid_request");
    }

    #[test]
    fn execute_export_plan_writes_originals_then_manifest() {
        let tmp = TempDir::new().unwrap();
        create_preview_fixture(tmp.path());
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();

        // The execution digest must equal the read-only preview digest for
        // the same selection — one reviewed state, one canonical export.
        let preview = preview_export_plan_path(
            tmp.path(),
            "sess-001",
            &["cap-2".to_string(), "cap-1".to_string()],
        );
        let preview_digest = match preview {
            ExportPreviewResult::Ok { preview } => preview.manifest_digest,
            ExportPreviewResult::Err { error } => panic!("preview failed: {:?}", error),
        };

        let result = execute_export_plan_path(
            tmp.path(),
            &dest,
            "sess-001",
            &["cap-2".to_string(), "cap-1".to_string()],
        );
        let summary = match result {
            ExportExecutionResult::Ok { execution } => execution,
            ExportExecutionResult::Err { error } => panic!("export failed: {:?}", error),
        };
        assert_eq!(summary.schema, EXPORT_EXECUTION_SCHEMA);
        assert_eq!(summary.session_id, "sess-001");
        assert_eq!(summary.capture_ids, vec!["cap-2", "cap-1"]);
        assert_eq!(summary.files_written, 3); // 2 originals + manifest
        assert_eq!(summary.manifest_digest, preview_digest);
        assert!(foldscan_domain::checksum::is_lowercase_hex_sha256(
            &summary.manifest_sha256
        ));

        let root = Path::new(&summary.root);
        assert!(root.starts_with(&dest));
        let dir_name = root.file_name().unwrap().to_string_lossy().to_string();
        assert!(dir_name.starts_with("foldscan-export-sess-001-"));
        assert_eq!(dir_name.len(), "foldscan-export-sess-001-".len() + 12);

        // Exact planned layout, originals byte-identical to the volume.
        let cap2 = root.join("originals/sess-001/cap-2.jpg");
        let cap1 = root.join("originals/sess-001/cap-1.jpg");
        let manifest_file = root.join("export.json");
        assert!(cap2.is_file() && cap1.is_file() && manifest_file.is_file());
        let second = b"\xFF\xD8\xFF\xE0second-page-image\xFF\xD9";
        assert_eq!(fs::read(&cap2).unwrap(), second);
        assert_eq!(fs::read(&cap1).unwrap(), FAKE_JPEG);
        let manifest_bytes = fs::read(&manifest_file).unwrap();
        assert_eq!(
            sha256_hex(&manifest_bytes),
            summary.manifest_sha256,
            "reported manifest hash must match finalized bytes"
        );

        // Manifest-last content check: the portable document lists pages in
        // the reviewed order and no staging leftovers exist.
        let doc: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(doc["schema"], "foldscan.export/0.1");
        assert_eq!(doc["pages"][0]["capture_id"], "cap-2");
        assert_eq!(doc["pages"][1]["capture_id"], "cap-1");
        assert!(doc["pages"][0]["processed_sha256"].is_null());
        let mut leftovers = Vec::new();
        for entry in walkdir(root) {
            if entry
                .extension()
                .map(|e| e == "foldscan-part")
                .unwrap_or(false)
            {
                leftovers.push(entry);
            }
        }
        assert!(
            leftovers.is_empty(),
            "finalized export has staged leftovers"
        );

        // The volume stays read-only through export.
        assert_eq!(
            fs::read(
                tmp.path()
                    .join("FOLDSCAN/sessions/sess-001/captures/cap-1.jpg")
            )
            .unwrap(),
            FAKE_JPEG
        );
    }

    fn walkdir(root: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    #[test]
    fn execute_export_plan_refuses_to_overwrite_a_prior_export() {
        let tmp = TempDir::new().unwrap();
        create_preview_fixture(tmp.path());
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();

        let first = execute_export_plan_path(
            tmp.path(),
            &dest,
            "sess-001",
            &["cap-1".to_string(), "cap-2".to_string()],
        );
        let root = match first {
            ExportExecutionResult::Ok { execution } => Path::new(&execution.root).to_path_buf(),
            ExportExecutionResult::Err { error } => panic!("first export failed: {:?}", error),
        };
        assert!(root.is_dir());

        // Re-running the identical reviewed export resolves to the same
        // deterministic root and must be refused, not merged or replaced.
        let second = execute_export_plan_path(
            tmp.path(),
            &dest,
            "sess-001",
            &["cap-1".to_string(), "cap-2".to_string()],
        );
        match second {
            ExportExecutionResult::Ok { .. } => panic!("re-export must be refused"),
            ExportExecutionResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("already exists"));
            }
        }
        // The refused run left the first export untouched.
        assert!(root.join("export.json").is_file());
    }

    #[test]
    fn execute_export_plan_rejects_bad_requests_before_touching_disk() {
        let tmp = TempDir::new().unwrap();
        create_preview_fixture(tmp.path());
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();

        let empty_dest = execute_export_plan_path(
            tmp.path(),
            Path::new(""),
            "sess-001",
            &["cap-1".to_string()],
        );
        match empty_dest {
            ExportExecutionResult::Ok { .. } => panic!("empty destination must fail"),
            ExportExecutionResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("destination parent"));
            }
        }

        let empty_sel = execute_export_plan_path(tmp.path(), &dest, "sess-001", &[]);
        match empty_sel {
            ExportExecutionResult::Ok { .. } => panic!("empty selection must fail"),
            ExportExecutionResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("at least one capture"));
            }
        }

        let unknown =
            execute_export_plan_path(tmp.path(), &dest, "sess-001", &["cap-missing".to_string()]);
        match unknown {
            ExportExecutionResult::Ok { .. } => panic!("unknown capture must fail"),
            ExportExecutionResult::Err { error } => {
                assert_eq!(error.category, "invalid_request");
                assert!(error.message.contains("is not present"));
            }
        }

        // None of the refusals created an export directory.
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn execution_result_serializes_tagged_json() {
        let ok = ExportExecutionResult::Ok {
            execution: ExportExecutionSummary {
                schema: EXPORT_EXECUTION_SCHEMA.to_string(),
                session_id: "sess-001".to_string(),
                capture_ids: vec!["cap-1".to_string()],
                root: "/tmp/foldscan-export-sess-001-abc".to_string(),
                files_written: 2,
                manifest_sha256: "b".repeat(64),
                manifest_digest: "a".repeat(64),
            },
        };
        let ok_json = serde_json::to_value(ok).unwrap();
        assert_eq!(ok_json["status"], "ok");
        assert_eq!(ok_json["execution"]["schema"], EXPORT_EXECUTION_SCHEMA);
        let mut keys: Vec<&str> = ok_json["execution"]
            .as_object()
            .expect("execution is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "capture_ids",
                "files_written",
                "manifest_digest",
                "manifest_sha256",
                "root",
                "schema",
                "session_id"
            ]
        );

        let err = ExportExecutionResult::Err {
            error: ImportFailure {
                category: "storage_unavailable".to_string(),
                message: "volume gone".to_string(),
            },
        };
        let err_json = serde_json::to_value(err).unwrap();
        assert_eq!(err_json["status"], "err");
        assert_eq!(err_json["error"]["category"], "storage_unavailable");
    }

    #[test]
    fn ocr_review_loads_real_export_and_refuses_tampering_and_wrong_digest() {
        use foldscan_domain::ocr::{OcrResult, OcrStatus};
        use foldscan_domain::{plan_export, ExportPage, ExportSession};

        let tmp = TempDir::new().unwrap();
        let original = b"synthetic original";
        let doc = OcrResult {
            schema: "foldscan.ocr/0.1".into(),
            capture_id: "cap-1".into(),
            requested_languages: vec!["eng".into()],
            status: OcrStatus::Completed {
                frame_width: 40,
                frame_height: 40,
                blocks: vec![OcrBlock {
                    text: "private fixture".into(),
                    confidence: 753,
                    x: 2,
                    y: 3,
                    width: 20,
                    height: 10,
                }],
            },
        };
        let session = ExportSession {
            session_id: "sess-001".into(),
            pages: vec![ExportPage {
                capture_id: "cap-1".into(),
                media_type: "image/jpeg".into(),
                processed_media_type: None,
                original_sha256: sha256_hex(original),
                original_bytes: original.len() as u64,
                processed_sha256: None,
                processed_bytes: None,
                recipe_digest: None,
            }],
            document: None,
            ocr: vec![doc],
            ocr_text: true,
        };
        let plan = plan_export(&[session], &[]).unwrap();
        let manifest = ExportManifest::from_plan(&plan, "sess-001").unwrap();
        let digest = manifest.digest();
        let source = tmp.path().join("source.jpg");
        fs::write(&source, original).unwrap();
        let sources = HashMap::from([(
            "originals/sess-001/cap-1.jpg".to_string(),
            ExportSource::file(source),
        )]);
        let root = tmp.path().join("out");
        execute_export(&plan, &root, &sources, &manifest).unwrap();
        let ok = review_exported_ocr_path(&root, &digest);
        let OcrReviewResult::Ok { review } = ok else {
            panic!("expected OCR review");
        };
        assert_eq!(review.schema, OCR_REVIEW_SCHEMA);
        assert_eq!(review.documents.len(), 1);
        assert_eq!(review.documents[0].capture_id, "cap-1");
        assert_eq!(review.documents[0].blocks[0].confidence, 753);
        assert_eq!(review.documents[0].blocks[0].text, "private fixture");
        let bad = review_exported_ocr_path(&root, &"0".repeat(64));
        let OcrReviewResult::Err { error } = bad else {
            panic!("wrong digest accepted");
        };
        assert_eq!(error.category, "checksum_mismatch");
        assert!(!error.message.contains("private"));
        fs::write(root.join("ocr/sess-001/cap-1.json"), b"tampered").unwrap();
        let bad = review_exported_ocr_path(&root, &digest);
        let OcrReviewResult::Err { error } = bad else {
            panic!("tampering accepted");
        };
        assert_eq!(error.category, "invalid_request");
        assert!(!error.message.contains("private"));
        let bad = review_exported_ocr_path(&tmp.path().join("missing"), &digest);
        assert!(matches!(bad, OcrReviewResult::Err { .. }));
    }

    #[test]
    fn ocr_projection_refuses_oversized_text_without_partial_results() {
        let tmp = TempDir::new().unwrap();
        let result = review_exported_ocr_path(tmp.path(), "not-a-digest");
        let OcrReviewResult::Err { error } = result else {
            panic!("invalid digest accepted");
        };
        assert_eq!(error.category, "invalid_request");
        assert!(!error.message.contains(tmp.path().to_str().unwrap()));
        let doc = OcrResult {
            schema: "foldscan.ocr/0.1".into(),
            capture_id: "p1".into(),
            requested_languages: vec!["eng".into()],
            status: OcrStatus::Completed {
                frame_width: 40,
                frame_height: 40,
                blocks: vec![OcrBlock {
                    text: "x".repeat(MAX_REVIEW_TEXT_BYTES + 1),
                    confidence: 700,
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                }],
            },
        };
        for docs in [
            vec![doc.clone()],
            vec![doc.clone(); MAX_REVIEW_DOCUMENTS + 1],
        ] {
            let json = serde_json::to_value(project_ocr_review(docs)).unwrap();
            assert_eq!(json["status"], "err");
            assert!(json.get("review").is_none());
        }
        let OcrStatus::Completed { blocks, .. } = &doc.status else {
            unreachable!()
        };
        let mut many = doc.clone();
        many.status = OcrStatus::Completed {
            frame_width: 40,
            frame_height: 40,
            blocks: vec![blocks[0].clone(); MAX_REVIEW_BLOCKS_PER_DOCUMENT + 1],
        };
        assert!(matches!(
            project_ocr_review(vec![many]),
            OcrReviewResult::Err { .. }
        ));
    }
}
