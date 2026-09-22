//! Export layout planning, originals/derivatives model, and the versioned
//! portable session export manifest.
//!
//! Scope of this slice (issue #5, export-side domain layer):
//! - an [`ExportPage`] model that keeps *originals* and *processed
//!   derivatives* as distinct entries — processing is always additive and
//!   reversible, originals are never mutated or deleted;
//! - "remove from export without deleting originals" via [`remove_from`];
//! - a deterministic [`ExportPlan`] directory-tree generator that writes
//!   *nothing*: it produces the intended relative layout (originals,
//!   processed, recipes, `export.json`) and rejects collisions/overwrites
//!   in the plan before any executor touches disk;
//! - a versioned, checksum-bound [`ExportManifest`] (`foldscan.export/0.1`)
//!   whose integrity fields cover page order and content, written last per
//!   the protocol's temporary-write/verify/finalize rule;
//! - an optional session-level document binding ([`SessionDocument`] /
//!   [`ExportManifestDocument`]): one assembled artifact per session
//!   (today only a PDF from [`crate::pdf`]) whose ordered capture bindings
//!   must equal the export page order, laid out at
//!   `documents/<session>/session.pdf` and covered by the manifest digest.
//! - an optional per-capture OCR sidecar binding: *completed*
//!   [`crate::ocr::OcrResult`] documents laid out at
//!   `ocr/<session>/<capture>.json` and bound into the manifest by content
//!   digest. `failed`/`skipped` results are rejected at the bind boundary so
//!   OCR failure structurally cannot alter or block an image/PDF export.
//!
//! Evidence category: planning + validation over data structures only. No
//! image encoding, PDF writing, or filesystem mutation happens here, and no
//! physical device is involved.

use serde::{Deserialize, Serialize};

use crate::checksum::{is_lowercase_hex_sha256, sha256_hex};
use crate::error::DomainError;
use crate::import::ImportedSession;
use crate::limits::*;
use crate::recipe::ProcessingRecipe;
use crate::version::{parse_tag_version, ProtocolVersion};

/// Schema tag prefix for export manifests (`foldscan.export/<major>.<minor>`).
pub const EXPORT_SCHEMA_PREFIX: &str = "foldscan.export/";

/// Maximum raw JSON bytes for one export manifest document.
pub const MAX_EXPORT_MANIFEST_BYTES: usize = 2 * 1024 * 1024;

/// Directory segment for preserved originals inside an export.
pub const ORIGINALS_DIR: &str = "originals";

/// Directory segment for processed derivatives inside an export.
pub const PROCESSED_DIR: &str = "processed";

/// Directory segment for recipe JSON documents inside an export.
pub const RECIPES_DIR: &str = "recipes";

/// Directory segment for session-level documents inside an export.
pub const DOCUMENTS_DIR: &str = "documents";

/// Directory segment for per-capture OCR sidecar documents inside an export.
pub const OCR_DIR: &str = "ocr";

/// File name of the session-level assembled document inside `documents/`.
pub const SESSION_DOCUMENT_FILE: &str = "session.pdf";

/// File name of the portable manifest at the export root.
pub const EXPORT_MANIFEST_FILE: &str = "export.json";

/// One page in an export: its preserved original and an optional processed
/// derivative. The original is the source of truth; the derivative is
/// reproducible from the original plus `recipe_digest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPage {
    pub capture_id: String,
    /// Original media type as declared by the device (e.g. `image/jpeg`).
    pub media_type: String,
    /// Declared export media type of the derivative (e.g. `image/png`).
    /// Omitted means PNG only when the other derivative fields are complete;
    /// original-only pages must omit all derivative fields.
    pub processed_media_type: Option<String>,
    /// Lowercase-hex SHA-256 of the original bytes (verified at import).
    pub original_sha256: String,
    pub original_bytes: u64,
    /// Lowercase-hex SHA-256 of the derivative bytes, when produced.
    pub processed_sha256: Option<String>,
    pub processed_bytes: Option<u64>,
    /// [`ProcessingRecipe::digest()`] of the recipe applied, when processed.
    pub recipe_digest: Option<String>,
}

/// Pages are ordered; the manifest preserves that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSession {
    pub session_id: String,
    pub pages: Vec<ExportPage>,
    /// Optional session-level assembled document (e.g. the PDF built from
    /// the ordered page set). `None` means the session exports pages only,
    /// exactly as before this field existed.
    pub document: Option<SessionDocument>,
    /// Optional per-capture OCR sidecar documents to export alongside the
    /// pages. Only [`crate::ocr::OcrStatus::Completed`] documents may be
    /// bound — `failed`/`skipped` outcomes are host-local review state and
    /// structurally cannot enter an export (issue #5: OCR failure must not
    /// block image/PDF export). An empty list means no OCR content, exactly
    /// as before this field existed. Each capture may appear at most once,
    /// and every entry must bind to a capture present in `pages`.
    pub ocr: Vec<crate::ocr::OcrResult>,
    /// Opt-in rendering of each bound completed OCR document as an
    /// additional plain-text sidecar (`ocr/<session>/<capture>.txt`,
    /// issue #35). `false` means JSON sidecars only, byte-identical to
    /// pre-text exports. Text is always *derived* by the executor from the
    /// bound document — hosts never supply text bytes — so this flag can
    /// only add renditions of text the session already binds, never new
    /// content. Requires every OCR binding to remain completed; it changes
    /// nothing when `ocr` is empty.
    pub ocr_text: bool,
}

/// One capture's participation in a session-level document: which capture
/// contributed a page and the frame dimensions that page was assembled at.
/// Recording dimensions here is what makes the issue #5 criterion "verify
/// exported page order/dimensions" checkable from the portable manifest
/// alone, without re-parsing the document bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentPage {
    pub capture_id: String,
    /// Declared frame width of this page inside the assembled document.
    pub width_px: u32,
    /// Declared frame height of this page inside the assembled document.
    pub height_px: u32,
}

/// A session-level document derivative: one assembled artifact (today only
/// a PDF, produced by [`crate::pdf::export_pdf`] over the ordered page set)
/// carried alongside the per-page originals/derivatives.
///
/// The binding list must equal the session's export page order exactly —
/// the document *is* the page order, so a manifest whose document claims a
/// different order is rejected rather than silently exported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDocument {
    /// Closed media-type vocabulary; only `application/pdf` is accepted.
    pub media_type: String,
    /// Lowercase-hex SHA-256 of the assembled document bytes.
    pub sha256: String,
    /// Declared byte count, bounded by the PDF writer's document ceiling.
    pub bytes: u64,
    /// Ordered per-capture page bindings, equal to the session page order.
    pub pages: Vec<DocumentPage>,
}

impl SessionDocument {
    /// Validate a document record against the ordered capture ids it must
    /// bind to (media vocabulary, checksum shape, byte/dimension bounds,
    /// non-empty binding list, exact order equality). Bounds-checked
    /// arithmetic only; nothing here reads document bytes.
    pub(crate) fn validate_against<'a>(
        &self,
        page_ids: impl Iterator<Item = &'a str>,
    ) -> Result<(), DomainError> {
        validate_document_fields(
            &self.media_type,
            &self.sha256,
            self.bytes,
            &self.pages,
            page_ids,
        )
    }
}

/// Shared field validation for both the session-side and manifest-side
/// views of a session document (same rules, different struct shapes).
fn validate_document_fields<'a>(
    media_type: &str,
    sha256: &str,
    bytes: u64,
    pages: &[DocumentPage],
    page_ids: impl Iterator<Item = &'a str>,
) -> Result<(), DomainError> {
    if media_type != "application/pdf" {
        return Err(DomainError::invalid_request(format!(
            "session document media type {media_type} is not supported (only application/pdf)"
        )));
    }
    validate_checksum("session document sha256", sha256)?;
    if bytes > crate::pdf::MAX_DOCUMENT_BYTES {
        return Err(DomainError::invalid_request(format!(
            "session document declares {bytes} bytes, over the {} limit",
            crate::pdf::MAX_DOCUMENT_BYTES
        )));
    }
    if pages.is_empty() {
        return Err(DomainError::invalid_request(
            "session document must bind at least one page",
        ));
    }
    let declared: Vec<&str> = pages.iter().map(|p| p.capture_id.as_str()).collect();
    let expected: Vec<&str> = page_ids.collect();
    if declared != expected {
        return Err(DomainError::invalid_request(
            "session document page bindings must equal the session export page order",
        ));
    }
    for p in pages {
        if p.width_px == 0 || p.height_px == 0 {
            return Err(DomainError::invalid_request(format!(
                "document page {} declares a zero dimension",
                p.capture_id
            )));
        }
        if (p.width_px as u64) > MAX_DIMENSION_PX || (p.height_px as u64) > MAX_DIMENSION_PX {
            return Err(DomainError::invalid_request(format!(
                "document page {} declares a pixel dimension over {MAX_DIMENSION_PX}",
                p.capture_id
            )));
        }
    }
    Ok(())
}

/// Validate a session's OCR sidecar binding list and index it by capture.
/// Rules (all checked against the *whole* list, so partial-binding states
/// fail deterministically):
/// - every document re-validates as a well-formed `foldscan.ocr/0.1` doc;
/// - only `completed` documents are bindable — `failed`/`skipped` are host
///   review state, and admitting them would let OCR outcomes leak into the
///   export layout/manifest (issue #5 requires OCR failure to never block
///   or alter image/PDF export; the only export-relevant OCR fact is text
///   that actually exists);
/// - every sidecar binds to a capture that is actually in the export;
/// - at most one sidecar per capture.
///
/// Layout order itself is derived from session page order by the planner,
/// so the host's list order can never change the plan.
fn validate_ocr_bindings(
    session: &ExportSession,
) -> Result<std::collections::HashMap<&str, &crate::ocr::OcrResult>, DomainError> {
    use std::collections::HashMap;

    let page_ids: std::collections::HashSet<&str> = session
        .pages
        .iter()
        .map(|p| p.capture_id.as_str())
        .collect();
    let mut index: HashMap<&str, &crate::ocr::OcrResult> = HashMap::new();
    for doc in &session.ocr {
        doc.validate().map_err(|e| {
            DomainError::invalid_request(format!(
                "bound ocr document for {} is invalid: {}",
                doc.capture_id, e.message
            ))
        })?;
        if !matches!(doc.status, crate::ocr::OcrStatus::Completed { .. }) {
            return Err(DomainError::invalid_request(format!(
                "ocr sidecar for {} is not a completed result; only completed documents export",
                doc.capture_id
            )));
        }
        if !page_ids.contains(doc.capture_id.as_str()) {
            return Err(DomainError::invalid_request(format!(
                "ocr sidecar binds capture {} which is not in the export",
                doc.capture_id
            )));
        }
        if index.insert(doc.capture_id.as_str(), doc).is_some() {
            return Err(DomainError::invalid_request(format!(
                "duplicate ocr sidecar for capture {}",
                doc.capture_id
            )));
        }
    }
    Ok(index)
}

/// Build export sessions (and the recipe documents they reference) from a
/// validated import. Every imported capture becomes an original-backed page;
/// no processing has happened yet, so no derivatives exist.
pub fn export_sessions_from_import(sessions: &[ImportedSession]) -> Vec<ExportSession> {
    sessions
        .iter()
        .map(|s| ExportSession {
            session_id: s.session_id.clone(),
            pages: s
                .captures
                .iter()
                .map(|c| ExportPage {
                    capture_id: c.capture_id.clone(),
                    media_type: "image/jpeg".to_string(),
                    processed_media_type: None,
                    original_sha256: c.sha256.clone(),
                    original_bytes: c.bytes,
                    processed_sha256: None,
                    processed_bytes: None,
                    recipe_digest: None,
                })
                .collect(),
            document: None,
            ocr: Vec::new(),
            ocr_text: false,
        })
        .collect()
}

/// Remove a page *from the export* without deleting the original capture.
/// Returns the removed page so the caller can show the consequence. The
/// original file is never touched (protocol: originals are immutable through
/// ordinary app operations; deletion never silently crosses the boundary).
pub fn remove_from(session: &mut ExportSession, capture_id: &str) -> Option<ExportPage> {
    let idx = session
        .pages
        .iter()
        .position(|p| p.capture_id == capture_id)?;
    Some(session.pages.remove(idx))
}

/// Move a page within the session's export order. Out-of-range requests are
/// clamped; removal-then-reinsert keeps originals intact.
pub fn reorder(session: &mut ExportSession, capture_id: &str, to_index: usize) -> bool {
    let Some(from) = session
        .pages
        .iter()
        .position(|p| p.capture_id == capture_id)
    else {
        return false;
    };
    let page = session.pages.remove(from);
    let to = to_index.min(session.pages.len());
    session.pages.insert(to, page);
    true
}

/// One entry in a planned export directory tree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlannedFile {
    /// Path relative to the export root, slash-separated.
    pub relative_path: String,
    /// Provenance: which capture/recipe this file belongs to.
    pub capture_id: Option<String>,
    pub content_kind: ContentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContentKind {
    Original,
    Derivative,
    Document,
    Ocr,
    OcrText,
    Recipe,
    Manifest,
}

/// A complete, collision-checked directory-tree plan for one export.
/// Producing a plan writes nothing; an executor may materialize it later
/// using temporary-write/verify/finalize semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportPlan {
    pub sessions: Vec<ExportSession>,
    pub recipes: Vec<ProcessingRecipe>,
    pub files: Vec<PlannedFile>,
}

/// Build the layout plan. Every referenced recipe (by digest) must be
/// present in `recipes`; duplicate capture IDs or duplicate planned paths
/// are rejected before any disk mutation could occur.
pub fn plan_export(
    sessions: &[ExportSession],
    recipes: &[ProcessingRecipe],
) -> Result<ExportPlan, DomainError> {
    use std::collections::HashSet;

    let mut files: Vec<PlannedFile> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut seen_capture_ids: HashSet<&str> = HashSet::new();

    let recipe_index: std::collections::HashMap<String, usize> = recipes
        .iter()
        .enumerate()
        .map(|(i, r)| (r.digest(), i))
        .collect();

    for session in sessions {
        if session.session_id.is_empty() {
            return Err(DomainError::invalid_request("export session_id is empty"));
        }
        let sdir = sanitize_segment(&session.session_id)?;
        for page in &session.pages {
            if !seen_capture_ids.insert(page.capture_id.as_str()) {
                return Err(DomainError::invalid_request(format!(
                    "duplicate capture_id in export: {}",
                    page.capture_id
                )));
            }
            validate_checksum("original sha256", &page.original_sha256)?;

            let orig = format!(
                "{}/{}/{}.{}",
                ORIGINALS_DIR,
                sdir,
                page.capture_id,
                ext_for(&page.media_type)?
            );
            insert_path(
                &mut files,
                &mut seen_paths,
                orig,
                Some(&page.capture_id),
                ContentKind::Original,
            )?;

            if let Some(proc_sum) = &page.processed_sha256 {
                validate_checksum("processed sha256", proc_sum)?;
                let digest = page.recipe_digest.as_ref().ok_or_else(|| {
                    DomainError::invalid_request(format!(
                        "page {} has a derivative but no recipe digest",
                        page.capture_id
                    ))
                })?;
                if !recipe_index.contains_key(digest.as_str()) {
                    return Err(DomainError::invalid_request(format!(
                        "page {} references unknown recipe digest {}",
                        page.capture_id, digest
                    )));
                }
                let bytes = page.processed_bytes.ok_or_else(|| {
                    DomainError::invalid_request(
                        "derivative metadata requires a declared byte count",
                    )
                })?;
                validate_processed_bytes(bytes)?;
                let pmtype = page.processed_media_type.as_deref().unwrap_or("image/png");
                let proc = format!(
                    "{}/{}/{}.{}",
                    PROCESSED_DIR,
                    sdir,
                    page.capture_id,
                    ext_for(pmtype)?
                );
                insert_path(
                    &mut files,
                    &mut seen_paths,
                    proc,
                    Some(&page.capture_id),
                    ContentKind::Derivative,
                )?;
            } else if page.processed_media_type.is_some()
                || page.processed_bytes.is_some()
                || page.recipe_digest.is_some()
            {
                return Err(DomainError::invalid_request(
                    "original-only page must not carry derivative metadata",
                ));
            }
        }

        // Session-level assembled document, when one is bound. Its page
        // bindings must equal this session's export page order; the layout
        // path is derived (never caller-supplied), so it cannot smuggle a
        // different destination.
        if let Some(doc) = &session.document {
            doc.validate_against(session.pages.iter().map(|p| p.capture_id.as_str()))?;
            let doc_path = format!("{}/{}/{}", DOCUMENTS_DIR, sdir, SESSION_DOCUMENT_FILE);
            insert_path(
                &mut files,
                &mut seen_paths,
                doc_path,
                None,
                ContentKind::Document,
            )?;
        }

        // Per-capture OCR sidecars, when bound. Validation happens against
        // the whole binding list first (closed terminal status, known
        // captures, one sidecar per capture), then layout follows *session
        // page order* — not binding-list order — so the plan is identical
        // however the host happens to order its OCR list. With `ocr_text`
        // opted in, the rendered `.txt` rendition follows its `.json`
        // sidecar (still page order overall), so a consumer reading the
        // plan sees each capture's text pair together.
        let ocr_index = validate_ocr_bindings(session)?;
        for page in &session.pages {
            if ocr_index.contains_key(page.capture_id.as_str()) {
                let ocr_path = format!("{}/{}/{}.json", OCR_DIR, sdir, page.capture_id);
                insert_path(
                    &mut files,
                    &mut seen_paths,
                    ocr_path,
                    Some(&page.capture_id),
                    ContentKind::Ocr,
                )?;
                if session.ocr_text {
                    let txt_path = format!("{}/{}/{}.txt", OCR_DIR, sdir, page.capture_id);
                    insert_path(
                        &mut files,
                        &mut seen_paths,
                        txt_path,
                        Some(&page.capture_id),
                        ContentKind::OcrText,
                    )?;
                }
            }
        }
    }

    // One JSON document per referenced recipe digest.
    let mut referenced: Vec<&String> = Vec::new();
    for session in sessions {
        for page in &session.pages {
            if let Some(d) = &page.recipe_digest {
                if !referenced.contains(&d) {
                    referenced.push(d);
                }
            }
        }
    }
    for digest in referenced {
        let path = format!("{}/{}.json", RECIPES_DIR, digest);
        insert_path(&mut files, &mut seen_paths, path, None, ContentKind::Recipe)?;
    }

    // Manifest last: the protocol finalizes manifests after content.
    insert_path(
        &mut files,
        &mut seen_paths,
        EXPORT_MANIFEST_FILE.to_string(),
        None,
        ContentKind::Manifest,
    )?;

    Ok(ExportPlan {
        sessions: sessions.to_vec(),
        recipes: recipes.to_vec(),
        files,
    })
}

fn insert_path(
    files: &mut Vec<PlannedFile>,
    seen: &mut std::collections::HashSet<String>,
    path: String,
    capture_id: Option<&str>,
    kind: ContentKind,
) -> Result<(), DomainError> {
    crate::paths::safe_join(std::path::Path::new("/"), &path)?;
    if !seen.insert(path.clone()) {
        return Err(DomainError::invalid_request(format!(
            "export layout collision on path: {}",
            path
        )));
    }
    files.push(PlannedFile {
        relative_path: path,
        capture_id: capture_id.map(str::to_string),
        content_kind: kind,
    });
    Ok(())
}

/// Identifier-safe directory segment: reuses identifier bounds and rejects
/// separators; nothing user-visible is written verbatim without this check.
fn sanitize_segment(id: &str) -> Result<String, DomainError> {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return Err(DomainError::invalid_request(
            "session id length out of bounds",
        ));
    }
    if id
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_' | '.'))
    {
        return Err(DomainError::invalid_request(
            "session id contains characters unsafe for export paths",
        ));
    }
    if id == "." || id == ".." || id.starts_with('.') {
        return Err(DomainError::invalid_request(
            "session id is not a safe export segment",
        ));
    }
    Ok(id.to_string())
}

fn ext_for(media_type: &str) -> Result<&'static str, DomainError> {
    match media_type {
        "image/jpeg" => Ok("jpg"),
        "image/png" => Ok("png"),
        "application/pdf" => Ok("pdf"),
        "text/plain" => Ok("txt"),
        other => Err(DomainError::invalid_request(format!(
            "unsupported export media type: {}",
            other
        ))),
    }
}

fn validate_processed_bytes(bytes: u64) -> Result<(), DomainError> {
    if bytes > MAX_CAPTURE_BYTES {
        return Err(DomainError::invalid_request(
            "derivative byte count exceeds the per-file limit",
        ));
    }
    Ok(())
}

fn validate_checksum(label: &str, sum: &str) -> Result<(), DomainError> {
    if !is_lowercase_hex_sha256(sum) {
        return Err(DomainError::invalid_request(format!(
            "{} is not a lowercase-hex SHA-256",
            label
        )));
    }
    Ok(())
}

/// The versioned portable export manifest (schema `foldscan.export/0.1`).
/// This is what the release/export packaging writes *last*, after all
/// content files are finalized and re-verified.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportManifest {
    pub schema: String,
    pub session_id: String,
    pub pages: Vec<ExportManifestPage>,
    /// Session-level assembled document (path from the plan), when bound.
    /// Absent/null means pages-only export, byte-identical to pre-document
    /// manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<ExportManifestDocument>,
}

/// Manifest view of a [`SessionDocument`]: the record plus the plan-derived
/// layout path. The path is never caller-supplied — `from_plan` copies it
/// from the collision-checked layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportManifestDocument {
    pub path: String,
    /// Closed media-type vocabulary; only `application/pdf` is accepted.
    pub media_type: String,
    pub sha256: String,
    pub bytes: u64,
    /// Ordered per-capture page bindings, equal to the manifest page order.
    pub pages: Vec<DocumentPage>,
}

impl ExportManifestDocument {
    pub(crate) fn validate_against<'a>(
        &self,
        page_ids: impl Iterator<Item = &'a str>,
    ) -> Result<(), DomainError> {
        validate_document_fields(
            &self.media_type,
            &self.sha256,
            self.bytes,
            &self.pages,
            page_ids,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportManifestPage {
    pub capture_id: String,
    pub original_path: String,
    pub original_sha256: String,
    pub original_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_digest: Option<String>,
    /// Canonical-JSON content digest ([`crate::ocr::OcrResult::digest`]) of
    /// the OCR sidecar bound to this capture. Present iff the export lays
    /// out an `ocr/` document for the capture; absent means no OCR content,
    /// byte-identical to pre-sidecar manifests. Binding the digest (not a
    /// path — the path is derived) makes the manifest integrity digest cover
    /// exactly *which* captures carry OCR text and *what* that text is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_digest: Option<String>,
    /// SHA-256 of the plain-text rendering bytes
    /// ([`crate::ocr::OcrResult::plain_text_digest`]) for this capture's
    /// bound OCR document. Present iff the session opted into `ocr_text`
    /// and lays out an `ocr/<session>/<capture>.txt` for it; absent means
    /// no text rendition, byte-identical to pre-text manifests. The text is
    /// derived from the same bound document `ocr_digest` pins, so the two
    /// digests can never disagree about page text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_text_digest: Option<String>,
}

impl ExportManifest {
    /// Materialize the manifest document for one session from a validated
    /// plan (paths come from the plan, so the manifest cannot claim files
    /// the plan did not lay out).
    pub fn from_plan(plan: &ExportPlan, session_id: &str) -> Result<Self, DomainError> {
        let session = plan
            .sessions
            .iter()
            .find(|s| s.session_id == session_id)
            .ok_or_else(|| DomainError::invalid_request("session not present in export plan"))?;

        let mut pages = Vec::with_capacity(session.pages.len());
        // OCR sidecar index for this session's bound documents. A plan whose
        // Ocr-kind layout disagrees with the bindings (extra or missing
        // sidecar files for an edited plan) is rejected here, so a manifest
        // can never claim OCR content the layout does not carry, or omit a
        // digest for a sidecar the layout does carry.
        let ocr_index = validate_ocr_bindings(session)?;
        let planned_ocr: std::collections::HashSet<&str> = plan
            .files
            .iter()
            .filter(|f| f.content_kind == ContentKind::Ocr)
            .map(|f| f.capture_id.as_deref().unwrap_or_default())
            .collect();
        // Plain-text renditions planned for this session (issue #35). Same
        // cross-check as the JSON sidecars: layout, bindings, and the
        // session's opt-in flag must agree exactly, per capture.
        let planned_ocr_text: std::collections::HashSet<&str> = plan
            .files
            .iter()
            .filter(|f| f.content_kind == ContentKind::OcrText)
            .map(|f| f.capture_id.as_deref().unwrap_or_default())
            .collect();
        for page in &session.pages {
            let binds_ocr = ocr_index.contains_key(page.capture_id.as_str());
            let plans_ocr = planned_ocr.contains(page.capture_id.as_str());
            if binds_ocr != plans_ocr {
                return Err(DomainError::internal(
                    "ocr sidecar layout does not match the session's ocr bindings",
                ));
            }
            let wants_text = binds_ocr && session.ocr_text;
            let plans_text = planned_ocr_text.contains(page.capture_id.as_str());
            if wants_text != plans_text {
                return Err(DomainError::internal(
                    "ocr text layout does not match the session's ocr text opt-in",
                ));
            }
            let ocr_digest = if binds_ocr {
                Some(
                    ocr_index
                        .get(page.capture_id.as_str())
                        .expect("presence checked above")
                        .digest(),
                )
            } else {
                None
            };
            let ocr_text_digest = if wants_text {
                Some(
                    ocr_index
                        .get(page.capture_id.as_str())
                        .expect("presence checked above")
                        .plain_text_digest()?,
                )
            } else {
                None
            };
            let original_path = plan
                .files
                .iter()
                .find(|f| {
                    f.content_kind == ContentKind::Original
                        && f.capture_id.as_deref() == Some(page.capture_id.as_str())
                })
                .ok_or_else(|| {
                    DomainError::internal("original missing from plan for a session page")
                })?
                .relative_path
                .clone();
            let processed = page.processed_sha256.as_ref().map(|_| {
                plan.files
                    .iter()
                    .find(|f| {
                        f.content_kind == ContentKind::Derivative
                            && f.capture_id.as_deref() == Some(page.capture_id.as_str())
                    })
                    .map(|f| f.relative_path.clone())
            });
            let processed_path = match processed {
                Some(Some(p)) => Some(p),
                Some(None) => {
                    return Err(DomainError::internal(
                        "derivative missing from plan for a processed page",
                    ))
                }
                None => None,
            };
            pages.push(ExportManifestPage {
                capture_id: page.capture_id.clone(),
                original_path,
                original_sha256: page.original_sha256.clone(),
                original_bytes: page.original_bytes,
                processed_path,
                processed_sha256: page.processed_sha256.clone(),
                processed_bytes: page.processed_bytes,
                recipe_digest: page.recipe_digest.clone(),
                ocr_digest,
                ocr_text_digest,
            });
        }

        let document = match &session.document {
            None => None,
            Some(doc) => {
                // Match the exact per-session layout path rather than the
                // first Document file in the plan, so a multi-session plan
                // (or an edited one) can never bind one session's document
                // path into another session's manifest.
                let expected_doc_path =
                    format!("{}/{}/{}", DOCUMENTS_DIR, session_id, SESSION_DOCUMENT_FILE);
                let path = plan
                    .files
                    .iter()
                    .find(|f| {
                        f.content_kind == ContentKind::Document
                            && f.relative_path == expected_doc_path
                    })
                    .ok_or_else(|| {
                        DomainError::internal(
                            "document layout missing from plan for a documented session",
                        )
                    })?
                    .relative_path
                    .clone();
                Some(ExportManifestDocument {
                    path,
                    media_type: doc.media_type.clone(),
                    sha256: doc.sha256.clone(),
                    bytes: doc.bytes,
                    pages: doc.pages.clone(),
                })
            }
        };

        let manifest = ExportManifest {
            schema: format!("{}0.1", EXPORT_SCHEMA_PREFIX),
            session_id: session_id.to_string(),
            pages,
            document,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate a parsed export manifest (bounds, checksum shapes, paths).
    /// Processed path/checksum/byte count/recipe digest are all-or-none.
    /// This validates metadata, not encoded media or recipe file contents.
    pub fn validate(&self) -> Result<ProtocolVersion, DomainError> {
        let version = parse_tag_version(EXPORT_SCHEMA_PREFIX, &self.schema, "export schema")?;
        sanitize_segment(&self.session_id)?;
        if self.pages.len() > MAX_CAPTURES_PER_SESSION {
            return Err(DomainError::invalid_request(format!(
                "export lists {} pages, over the {} limit",
                self.pages.len(),
                MAX_CAPTURES_PER_SESSION
            )));
        }
        use std::collections::HashSet;
        let mut ids = HashSet::new();
        for page in &self.pages {
            if page.capture_id.is_empty() || page.capture_id.len() > MAX_ID_LEN {
                return Err(DomainError::invalid_request(
                    "export page capture_id out of bounds",
                ));
            }
            if !ids.insert(page.capture_id.as_str()) {
                return Err(DomainError::invalid_request(format!(
                    "duplicate capture_id in export manifest: {}",
                    page.capture_id
                )));
            }
            validate_checksum("original sha256", &page.original_sha256)?;
            let derivative_fields = [
                page.processed_path.is_some(),
                page.processed_sha256.is_some(),
                page.processed_bytes.is_some(),
                page.recipe_digest.is_some(),
            ];
            if derivative_fields.iter().any(|present| *present)
                && !derivative_fields.iter().all(|present| *present)
            {
                return Err(DomainError::invalid_request(
                    "derivative path, checksum, byte count, and recipe digest must be present together",
                ));
            }
            if let Some(sum) = &page.processed_sha256 {
                validate_checksum("processed sha256", sum)?;
            }
            if let Some(digest) = &page.recipe_digest {
                validate_checksum("recipe digest", digest)?;
            }
            if let Some(digest) = &page.ocr_digest {
                validate_checksum("ocr digest", digest)?;
            }
            if let Some(digest) = &page.ocr_text_digest {
                validate_checksum("ocr text digest", digest)?;
                // A text rendition is a rendering of a bound document:
                // text without the document digest would let a manifest
                // claim a .txt for text the export never reviewed.
                if page.ocr_digest.is_none() {
                    return Err(DomainError::invalid_request(format!(
                        "export page {} carries an ocr text digest without an ocr document digest",
                        page.capture_id
                    )));
                }
            }
            if let Some(bytes) = page.processed_bytes {
                validate_processed_bytes(bytes)?;
            }
            if page.original_bytes > MAX_CAPTURE_BYTES {
                return Err(DomainError::invalid_request(format!(
                    "export page {} declares {} bytes, over the per-file limit",
                    page.capture_id, page.original_bytes
                )));
            }
            crate::paths::safe_join(std::path::Path::new("/"), &page.original_path)?;
            if let Some(p) = &page.processed_path {
                crate::paths::safe_join(std::path::Path::new("/"), p)?;
            }
        }
        if let Some(doc) = &self.document {
            doc.validate_against(self.pages.iter().map(|p| p.capture_id.as_str()))?;
            crate::paths::safe_join(std::path::Path::new("/"), &doc.path)?;
        }
        Ok(version)
    }

    /// Parse and validate an untrusted export manifest document.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, DomainError> {
        if bytes.len() > MAX_EXPORT_MANIFEST_BYTES {
            return Err(DomainError::invalid_request(format!(
                "export manifest exceeds {} byte bound",
                MAX_EXPORT_MANIFEST_BYTES
            )));
        }
        let doc: ExportManifest = serde_json::from_slice(bytes).map_err(|e| {
            let cut = e.to_string();
            let cut = cut.split(" at line").next().unwrap_or(&cut);
            DomainError::invalid_request(format!("export manifest JSON invalid: {}", cut))
        })?;
        doc.validate()?;
        Ok(doc)
    }

    /// Deterministic integrity digest over the manifest's semantic content
    /// (session, page order, checksums, recipe digests) using the same
    /// canonicalization as recipe digests.
    pub fn digest(&self) -> String {
        let value = serde_json::to_value(self).expect("manifest serialization is infallible");
        let canonical = canonical_json_for_digest(&value);
        sha256_hex(canonical.as_bytes())
    }
}

/// Canonicalize with sorted object keys so digests are map-iteration stable.
fn canonical_json_for_digest(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("\"{}\":{}", k, canonical_json_for_digest(&map[*k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json_for_digest).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{OpKind, RecipeOp};

    fn page(id: &str) -> ExportPage {
        ExportPage {
            capture_id: id.to_string(),
            media_type: "image/jpeg".to_string(),
            processed_media_type: None,
            original_sha256: sha256_hex(id.as_bytes()),
            original_bytes: 100,
            processed_sha256: None,
            processed_bytes: None,
            recipe_digest: None,
        }
    }

    fn session() -> ExportSession {
        ExportSession {
            session_id: "sess-0001".to_string(),
            pages: vec![page("cap-a"), page("cap-b"), page("cap-c")],
            document: None,
            ocr: Vec::new(),
            ocr_text: false,
        }
    }

    fn rotate_recipe() -> ProcessingRecipe {
        ProcessingRecipe {
            schema: "foldscan.recipe/0.1".to_string(),
            name: "quarter".to_string(),
            ops: vec![RecipeOp {
                kind: OpKind::Rotate,
                params: serde_json::json!({"degrees": 90}),
            }],
        }
    }

    #[test]
    fn plan_lays_out_originals_and_manifest() {
        let s = session();
        let plan = plan_export(&[s], &[]).expect("plan");
        let paths: Vec<&str> = plan
            .files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"originals/sess-0001/cap-a.jpg"));
        assert!(paths.contains(&"originals/sess-0001/cap-c.jpg"));
        assert_eq!(
            plan.files.last().unwrap().content_kind,
            ContentKind::Manifest,
            "manifest must be planned last"
        );
    }

    #[test]
    fn processed_page_pulls_in_recipe_document() {
        let mut s = session();
        let r = rotate_recipe();
        let d = r.digest();
        s.pages[0].processed_sha256 = Some(sha256_hex(b"derived"));
        s.pages[0].processed_bytes = Some(120);
        s.pages[0].processed_media_type = Some("image/png".to_string());
        s.pages[0].recipe_digest = Some(d.clone());
        let plan = plan_export(&[s], &[r]).expect("plan");
        let paths: Vec<String> = plan.files.iter().map(|f| f.relative_path.clone()).collect();
        assert!(paths.contains(&"processed/sess-0001/cap-a.png".to_string()));
        assert!(paths.contains(&format!("recipes/{}.json", d)));
    }

    #[test]
    fn derivative_without_recipe_digest_is_rejected() {
        let mut s = session();
        s.pages[0].processed_sha256 = Some(sha256_hex(b"x"));
        let err = plan_export(&[s], &[]).unwrap_err();
        assert!(err.message.contains("no recipe digest"));
    }

    #[test]
    fn unknown_recipe_digest_is_rejected() {
        let mut s = session();
        s.pages[0].processed_sha256 = Some(sha256_hex(b"x"));
        s.pages[0].recipe_digest = Some("f".repeat(64));
        let err = plan_export(&[s], &[rotate_recipe()]).unwrap_err();
        assert!(err.message.contains("unknown recipe digest"));
    }

    #[test]
    fn duplicate_capture_ids_rejected() {
        let mut s = session();
        s.pages[2].capture_id = "cap-a".to_string();
        let err = plan_export(&[s], &[]).unwrap_err();
        assert!(err.message.contains("duplicate capture_id"));
    }

    #[test]
    fn remove_from_export_keeps_original_page_data() {
        let mut s = session();
        let removed = remove_from(&mut s, "cap-b").expect("present");
        assert_eq!(removed.capture_id, "cap-b");
        assert_eq!(s.pages.len(), 2);
        // Original checksum survived in the removed page (not deleted).
        assert!(is_lowercase_hex_sha256(&removed.original_sha256));
        assert!(remove_from(&mut s, "cap-b").is_none());
    }

    #[test]
    fn reorder_moves_pages_within_bounds() {
        let mut s = session();
        assert!(reorder(&mut s, "cap-c", 0));
        assert_eq!(s.pages[0].capture_id, "cap-c");
        assert!(reorder(&mut s, "cap-a", 999)); // clamped
        assert_eq!(s.pages.last().unwrap().capture_id, "cap-a");
        assert!(!reorder(&mut s, "missing", 0));
    }

    #[test]
    fn unsafe_session_id_rejected_in_layout() {
        let mut s = session();
        s.session_id = "../evil".to_string();
        let err = plan_export(&[s], &[]).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn manifest_round_trips_and_validates() {
        let s = session();
        let plan = plan_export(&[s], &[]).unwrap();
        let m = ExportManifest::from_plan(&plan, "sess-0001").unwrap();
        assert_eq!(m.pages.len(), 3);
        assert_eq!(m.pages[0].original_path, "originals/sess-0001/cap-a.jpg");
        let bytes = serde_json::to_vec(&m).unwrap();
        let back = ExportManifest::from_json_bytes(&bytes).unwrap();
        assert_eq!(m.digest(), back.digest());
    }

    #[test]
    fn manifest_rejects_bad_major_and_paths() {
        let s = session();
        let plan = plan_export(&[s], &[]).unwrap();
        let mut m = ExportManifest::from_plan(&plan, "sess-0001").unwrap();
        m.schema = "foldscan.export/7.0".to_string();
        let err = m.validate().unwrap_err();
        assert_eq!(
            err.category,
            crate::error::Category::UnsupportedVersion,
            "unknown major fails as version, keep files"
        );

        let mut m2 = ExportManifest::from_plan(&plan, "sess-0001").unwrap();
        m2.pages[0].original_path = "../escape.jpg".to_string();
        let err = m2.validate().unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn manifest_digest_order_sensitive() {
        let s = session();
        let plan = plan_export(std::slice::from_ref(&s), &[]).unwrap();
        let m1 = ExportManifest::from_plan(&plan, "sess-0001").unwrap();

        let mut s2 = s;
        s2.pages.reverse();
        let plan2 = plan_export(&[s2], &[]).unwrap();
        let m2 = ExportManifest::from_plan(&plan2, "sess-0001").unwrap();
        assert_ne!(m1.digest(), m2.digest(), "page order is part of integrity");
    }

    #[test]
    fn unsupported_media_type_rejected() {
        let mut s = session();
        s.pages[0].media_type = "image/tiff".to_string();
        let err = plan_export(&[s], &[]).unwrap_err();
        assert!(err.message.contains("unsupported export media type"));
    }

    #[test]
    fn from_import_creates_original_backed_pages() {
        let imported = vec![crate::import::ImportedSession {
            session_id: "s1".to_string(),
            captures: vec![crate::import::ImportedCapture {
                capture_id: "c1".to_string(),
                relative_path: "captures/c1.jpg".to_string(),
                bytes: 10,
                sha256: sha256_hex(b"0123456789"),
            }],
        }];
        let sessions = export_sessions_from_import(&imported);
        assert_eq!(
            sessions[0].pages[0].original_sha256,
            sha256_hex(b"0123456789")
        );
        assert!(sessions[0].pages[0].processed_sha256.is_none());
    }
}
