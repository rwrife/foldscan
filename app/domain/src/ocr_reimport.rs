//! Read-only re-import of OCR sidecars from a previously reviewed local export.
//!
//! Requires a caller-held export-manifest digest (from the export preview or
//! execution receipt). A digest stored *inside* the export directory would
//! not authenticate its own contents. No writes, network, or OCR engine.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};

use crate::checksum::sha256_hex;
use crate::error::DomainError;
use crate::export::{ExportManifest, MAX_EXPORT_MANIFEST_BYTES};
use crate::ocr::{OcrResult, MAX_OCR_DOCUMENT_BYTES, MAX_OCR_TEXT_BYTES};
use crate::paths::safe_join;

fn read_bound(root: &Path, relative: &str, max: usize) -> Result<Vec<u8>, DomainError> {
    let path = safe_join(root, relative)?;
    let mut part = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            return Err(DomainError::invalid_request("invalid export file path"));
        };
        part.push(name);
        let meta = std::fs::symlink_metadata(&part).map_err(|_| {
            DomainError::storage_unavailable("export file or directory unavailable")
        })?;
        if meta.file_type().is_symlink() {
            return Err(DomainError::invalid_request("symlink in export file path"));
        }
    }
    let mut file = File::open(path)
        .map_err(|_| DomainError::storage_unavailable("export file unavailable"))?;
    let meta = file
        .metadata()
        .map_err(|_| DomainError::storage_unavailable("export file unavailable"))?;
    if !meta.is_file() {
        return Err(DomainError::invalid_request("export file is not regular"));
    }
    if meta.len() > max as u64 {
        return Err(DomainError::invalid_request(
            "export file exceeds byte bound",
        ));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DomainError::storage_unavailable("cannot read export file"))?;
    if bytes.len() > max {
        return Err(DomainError::invalid_request(
            "export file exceeds byte bound",
        ));
    }
    Ok(bytes)
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Load the completed OCR documents bound to an export, in manifest page
/// order. Pages without OCR are skipped. A caller must supply the 64-character
/// lowercase SHA-256 of the *reviewed* export manifest; an untrusted directory
/// cannot authenticate itself. Optional `.txt` renditions must equal the
/// canonical rendering of the matching JSON document byte for byte.
///
/// This reads no image/PDF files and does not prove their integrity; it does
/// not persist edits. On concurrently mutating/untrusted filesystems there is
/// a symlink-check/open race; callers must use a stable local export snapshot.
/// The digest is an integrity pin, not a signature or identity proof.
/// Errors do not include host paths or recognized text.
pub fn load_exported_ocr(
    root: &Path,
    expected_manifest_digest: &str,
) -> Result<Vec<OcrResult>, DomainError> {
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|_| DomainError::storage_unavailable("export directory unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(DomainError::invalid_request(
            "export root must be a directory, not a symlink",
        ));
    }
    if expected_manifest_digest.len() != 64
        || !expected_manifest_digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(DomainError::invalid_request(
            "reviewed manifest digest must be lowercase SHA-256",
        ));
    }
    let bytes = read_bound(root, "export.json", MAX_EXPORT_MANIFEST_BYTES)?;
    let manifest = ExportManifest::from_json_bytes(&bytes)
        .map_err(|e| DomainError::new(e.category, "export manifest invalid"))?;
    if manifest.digest() != expected_manifest_digest {
        return Err(DomainError::checksum_mismatch(
            "reviewed export manifest digest mismatch",
        ));
    }
    let mut docs = Vec::new();
    for page in &manifest.pages {
        let Some(expected) = &page.ocr_digest else {
            continue;
        };
        if !safe_id(&page.capture_id) {
            return Err(DomainError::invalid_request("unsafe OCR capture ID"));
        }
        let path = format!("ocr/{}/{}.json", manifest.session_id, page.capture_id);
        let bytes = read_bound(root, &path, MAX_OCR_DOCUMENT_BYTES)?;
        let doc = OcrResult::from_json_bytes(&bytes)
            .map_err(|e| DomainError::new(e.category, "OCR document invalid"))?;
        if !matches!(doc.status, crate::ocr::OcrStatus::Completed { .. }) {
            return Err(DomainError::invalid_request(
                "exported OCR document is not completed",
            ));
        }
        if doc.capture_id != page.capture_id || doc.digest() != *expected {
            return Err(DomainError::checksum_mismatch(
                "OCR document binding mismatch",
            ));
        }
        if let Some(text_digest) = &page.ocr_text_digest {
            let path = format!("ocr/{}/{}.txt", manifest.session_id, page.capture_id);
            let text = read_bound(root, &path, MAX_OCR_TEXT_BYTES)?;
            let canonical = doc
                .render_plain_text()
                .map_err(|_| DomainError::invalid_request("OCR document has no text rendition"))?;
            if sha256_hex(&text) != *text_digest || text != canonical.as_bytes() {
                return Err(DomainError::checksum_mismatch(
                    "OCR text rendition mismatch",
                ));
            }
        }
        docs.push(doc);
    }
    Ok(docs)
}
