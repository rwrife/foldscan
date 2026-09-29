#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FoldScan companion desktop shell (issues #39, #41, #43, and #47).
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

use std::collections::HashSet;
use std::path::Path;

use foldscan_domain::export::{
    export_sessions_from_import, plan_export, ContentKind, ExportManifest,
};
use foldscan_domain::import::{import_volume, ImportPlan, ImportedSession};
use foldscan_domain::{Category, DomainError};
use serde::Serialize;

/// Schema tag for the status document returned by [`companion_status`].
pub const STATUS_SCHEMA: &str = "foldscan.companion.status/0.1";

/// Schema tag for the import summary returned on success.
pub const IMPORT_SUMMARY_SCHEMA: &str = "foldscan.companion.import_summary/0.1";

/// Schema tag for the read-only export preview returned on success.
pub const EXPORT_PREVIEW_SCHEMA: &str = "foldscan.companion.export_preview/0.1";

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

fn build_export_preview(
    imported: &ImportPlan,
    session_id: &str,
    capture_ids: &[String],
) -> Result<ExportPreview, DomainError> {
    if session_id.trim().is_empty() {
        return Err(invalid_request("session_id is empty"));
    }
    if capture_ids.is_empty() {
        return Err(invalid_request(
            "at least one capture must remain in the reviewed export selection",
        ));
    }

    let session = select_session(&imported.sessions, session_id)?;
    let capture_index: std::collections::HashMap<&str, &foldscan_domain::import::ImportedCapture> =
        session
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

    let mut export_sessions = export_sessions_from_import(&[ImportedSession {
        session_id: session.session_id.clone(),
        captures: selected,
    }]);
    let session = export_sessions
        .first_mut()
        .ok_or_else(|| DomainError::internal("export session synthesis produced no sessions"))?;
    session.document = None;
    session.ocr.clear();
    session.ocr_text = false;

    let plan = plan_export(&export_sessions, &[])?;
    let manifest = ExportManifest::from_plan(&plan, session_id)?;

    let files = plan
        .files
        .iter()
        .map(|file| {
            let content_kind = match file.content_kind {
                ContentKind::Original => "original",
                ContentKind::Derivative => "derivative",
                ContentKind::Document => "document",
                ContentKind::Ocr => "ocr",
                ContentKind::OcrText => "ocr_text",
                ContentKind::Recipe => "recipe",
                ContentKind::Manifest => "manifest",
            }
            .to_string();
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            companion_status,
            import_volume_summary,
            preview_export_plan
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
}
