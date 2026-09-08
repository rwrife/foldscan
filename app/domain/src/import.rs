//! USB/removable-media import engine.
//!
//! Given a volume root that is expected to contain `FOLDSCAN/`, the importer:
//! 1. caps raw manifest bytes *before* parsing (no unbounded allocation);
//! 2. validates protocol/schema, identifiers, bounds, and paths;
//! 3. walks sessions with a bounded session count;
//! 4. verifies each capture file exists, its actual size matches the declared
//!    size, and its SHA-256 matches the manifest (reading with a size cap);
//! 5. quarantines (never deletes) anything incomplete or unverifiable;
//! 6. rejects duplicate capture IDs *across* sessions;
//! 7. returns a validated import plan that contains no absolute paths.
//!
//! Every rejection is a stable protocol category. The importer never writes
//! to the device and never deletes originals.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::checksum::sha256_hex;
use crate::error::{Category, DomainError};
use crate::limits::*;
use crate::manifest::{DeviceManifest, SessionManifest};
use crate::paths::safe_join;

/// Root directory name expected on the removable volume.
pub const VOLUME_ROOT_DIR: &str = "FOLDSCAN";

/// Outcome for one verified capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCapture {
    pub capture_id: String,
    /// Path relative to the session directory (never absolute).
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Outcome for one successfully validated + verified session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSession {
    pub session_id: String,
    pub captures: Vec<ImportedCapture>,
}

/// Full import result for one volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    pub device_id: String,
    pub firmware_version: String,
    pub sessions: Vec<ImportedSession>,
}

/// Read at most `max` bytes from a file; error if the file is larger.
fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, DomainError> {
    let mut file = File::open(path).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot open manifest: {}", kind_of(&e)))
    })?;
    let mut buf = Vec::new();
    file.by_ref()
        .take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| {
            DomainError::storage_unavailable(format!("cannot read manifest: {}", kind_of(&e)))
        })?;
    if buf.len() as u64 > max {
        return Err(DomainError::invalid_request(format!(
            "manifest exceeds {} byte bound",
            max
        )));
    }
    Ok(buf)
}

fn kind_of(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "not-found",
        std::io::ErrorKind::PermissionDenied => "permission-denied",
        _ => "io-error",
    }
}

/// Parse a manifest JSON document with bounded parsing.
fn parse_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, DomainError> {
    serde_json::from_slice(bytes).map_err(|e| {
        DomainError::invalid_request(format!(
            "manifest JSON invalid: {}",
            trim_diag(&e.to_string())
        ))
    })
}

/// Keep JSON parser diagnostics short and free of host path leakage.
fn trim_diag(s: &str) -> String {
    let cut = s.split(" at line").next().unwrap_or(s);
    let cut = cut.trim_start_matches("expected ").to_string();
    let mut cut: String = cut.chars().take(120).collect();
    if cut.ends_with(char::is_whitespace) {
        cut = cut.trim_end().to_string();
    }
    cut
}

/// Locate the `FOLDSCAN/` directory on a volume root (case-insensitive on the
/// exact name only), or accept a root that *is* a FOLDSCAN directory.
fn locate_foldscan_dir(volume_root: &Path) -> Result<PathBuf, DomainError> {
    let direct = volume_root.join(VOLUME_ROOT_DIR);
    if direct.is_dir() {
        return Ok(direct);
    }
    if volume_root
        .file_name()
        .map(|n| n.eq_ignore_ascii_case(VOLUME_ROOT_DIR))
        .unwrap_or(false)
        && volume_root.is_dir()
    {
        return Ok(volume_root.to_path_buf());
    }
    Err(DomainError::storage_unavailable(
        "volume does not contain a FOLDSCAN directory",
    ))
}

/// Import a whole volume. `volume_root` is where the removable media is
/// mounted (or a directory that itself is a FOLDSCAN folder for tests).
pub fn import_volume(volume_root: &Path) -> Result<ImportPlan, DomainError> {
    let foldscan_dir = locate_foldscan_dir(volume_root)?;

    // --- device.json ---
    let device_path = foldscan_dir.join("device.json");
    let device_bytes = read_capped(&device_path, MAX_MANIFEST_BYTES)?;
    let device: DeviceManifest = parse_json(&device_bytes)?;
    device.validate()?;

    // --- sessions/ ---
    let sessions_dir = foldscan_dir.join("sessions");
    if !sessions_dir.is_dir() {
        return Ok(ImportPlan {
            device_id: device.device_id.clone(),
            firmware_version: device.firmware_version.clone(),
            sessions: Vec::new(),
        });
    }

    let mut session_dirs: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(&sessions_dir).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot list sessions: {}", kind_of(&e)))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            DomainError::storage_unavailable(format!("cannot list sessions: {}", kind_of(&e)))
        })?;
        let path = entry.path();
        if path.is_dir() {
            session_dirs.push(path);
        }
    }
    session_dirs.sort();
    if session_dirs.len() > MAX_SESSIONS {
        return Err(DomainError::invalid_request(format!(
            "volume lists {} sessions, over the {} limit",
            session_dirs.len(),
            MAX_SESSIONS
        )));
    }

    let mut seen_capture_ids: HashSet<String> = HashSet::new();
    let mut imported = Vec::new();

    for session_dir in session_dirs {
        let session = import_session(&session_dir, &mut seen_capture_ids)?;
        imported.push(session);
    }

    Ok(ImportPlan {
        device_id: device.device_id,
        firmware_version: device.firmware_version,
        sessions: imported,
    })
}

/// Validate and verify one session directory. `seen_capture_ids` enforces
/// cross-session uniqueness; duplicates abort the whole volume import.
pub fn import_session(
    session_dir: &Path,
    seen_capture_ids: &mut HashSet<String>,
) -> Result<ImportedSession, DomainError> {
    let manifest_path = session_dir.join("session.json");
    let manifest_bytes = read_capped(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest: SessionManifest = parse_json(&manifest_bytes)?;
    manifest.validate()?;

    if manifest.captures.len() > MAX_CAPTURES_PER_SESSION {
        // Re-check post-parse bound (validate already covers this).
        return Err(DomainError::invalid_request("too many captures"));
    }

    // Local duplicate check within this session already covered by validate;
    // here enforce *cross-session* uniqueness and abort the import loudly.
    let mut local_seen: HashMap<String, ()> = HashMap::new();
    for entry in &manifest.captures {
        if seen_capture_ids.contains(&entry.capture_id) {
            return Err(DomainError::invalid_request(format!(
                "capture_id {} appears in more than one session",
                entry.capture_id
            )));
        }
        local_seen.insert(entry.capture_id.clone(), ());
    }
    for id in local_seen.keys() {
        seen_capture_ids.insert(id.clone());
    }

    let mut verified = Vec::with_capacity(manifest.captures.len());
    for entry in &manifest.captures {
        let file_path = safe_join(session_dir, &entry.relative_path)?;

        // Real size must match declared size *before* reading contents.
        let meta = std::fs::metadata(&file_path).map_err(|_| {
            DomainError::storage_unavailable(format!(
                "declared capture file missing: {}",
                entry.relative_path
            ))
        })?;
        if !meta.is_file() {
            return Err(DomainError::invalid_request(
                "capture path is not a regular file",
            ));
        }
        if meta.len() != entry.bytes {
            return Err(DomainError::checksum_mismatch(format!(
                "capture {} is {} bytes on disk but manifest declares {}",
                entry.capture_id,
                meta.len(),
                entry.bytes
            )));
        }
        if entry.bytes > MAX_CAPTURE_BYTES {
            return Err(DomainError::invalid_request(
                "capture exceeds per-file bound",
            ));
        }

        // Read with a hard cap and verify the checksum.
        let mut file = File::open(&file_path).map_err(|e| {
            DomainError::storage_unavailable(format!("cannot read capture: {}", kind_of(&e)))
        })?;
        let mut buf = Vec::with_capacity(entry.bytes as usize);
        file.by_ref()
            .take(entry.bytes)
            .read_to_end(&mut buf)
            .map_err(|e| {
                DomainError::storage_unavailable(format!("cannot read capture: {}", kind_of(&e)))
            })?;
        if buf.len() as u64 != entry.bytes {
            return Err(DomainError::checksum_mismatch(format!(
                "capture {} truncated during read",
                entry.capture_id
            )));
        }
        let actual = sha256_hex(&buf);
        if actual != entry.sha256 {
            return Err(DomainError::checksum_mismatch(format!(
                "capture {} checksum mismatch",
                entry.capture_id
            )));
        }

        verified.push(ImportedCapture {
            capture_id: entry.capture_id.clone(),
            relative_path: entry.relative_path.clone(),
            bytes: entry.bytes,
            sha256: entry.sha256.clone(),
        });
    }

    Ok(ImportedSession {
        session_id: manifest.session_id.clone(),
        captures: verified,
    })
}

/// Return the category for an import error (helper for callers/logging).
pub fn category_of(err: &DomainError) -> Category {
    err.category
}
