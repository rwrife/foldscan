#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FoldScan companion desktop shell (issues #39 and #41).
//!
//! Scope: a buildable, linted, tested Tauri 2 shell that depends on the
//! `foldscan-domain` crate by path and exposes:
//! - [`companion_status`]: returns a versioned status document.
//! - [`import_volume_summary`]: runs bounded validation/import over a volume root
//!   and returns a structured, presentation-safe summary or structured failure.

use std::path::Path;

use foldscan_domain::import::{import_volume, ImportPlan};
use foldscan_domain::Category;
use serde::Serialize;

/// Schema tag for the status document returned by [`companion_status`].
pub const STATUS_SCHEMA: &str = "foldscan.companion.status/0.1";

/// Schema tag for the import summary returned on success.
pub const IMPORT_SUMMARY_SCHEMA: &str = "foldscan.companion.import_summary/0.1";

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

/// Compact per-session metrics for UI list presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportedSessionSummary {
    pub session_id: String,
    pub capture_count: usize,
    pub total_bytes: u64,
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
        total_captures = total_captures.saturating_add(count);
        total_bytes = total_bytes.saturating_add(bytes);
        sessions.push(ImportedSessionSummary {
            session_id: s.session_id.clone(),
            capture_count: count,
            total_bytes: bytes,
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            companion_status,
            import_volume_summary
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
}
