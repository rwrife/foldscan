//! OCR result document core and provider-interface seam (issue #5).
//!
//! Issue #5 requires *optional offline OCR behind an interface* with explicit
//! language-pack/configuration behavior, uncertainty presentation, and the
//! guarantee that OCR failure never blocks image/PDF export. This module is
//! the domain-layer contract for that criterion: it models the *result* of an
//! OCR run and the seam an offline engine adapter will later implement. It
//! deliberately contains no engine, no network use, no language data, and no
//! dependencies beyond the crate's existing pinned set.
//!
//! Document model (`foldscan.ocr/0.1`), one document per capture:
//! - `capture_id` binds the result to one imported capture;
//! - `requested_languages` records the explicit language-pack configuration
//!   the run was asked for (validated ids, bounded count);
//! - `status` is a *closed terminal* set — `completed` (recognized blocks
//!   with per-mille numeric confidence and pixel bounding boxes), `failed`
//!   (closed failure-code vocabulary), or `skipped` (closed reason
//!   vocabulary). A capture with no OCR document at all means "OCR was not
//!   run"; the absence is represented by the host, not by a placeholder doc.
//!
//! Design rules mirrored from the recipe/manifest modules:
//! - Uncertainty is numeric (integer per-mille confidence, 0..=1000), never
//!   a color-only or float-only signal, so a UI can present it accessibly.
//! - Bounds are enforced on raw bytes *before* allocation/parsing, and again
//!   on the parsed document (block count, text size, language count, id
//!   lengths, coordinate spans with overflow-checked arithmetic).
//! - Recognized text rejects *all* control characters (block structure is
//!   the only layout signal) so untrusted OCR text cannot smuggle escape
//!   sequences into downstream renderers.
//! - [`OcrResult::digest()`] uses the same canonical-JSON rule as recipe and
//!   export-manifest digests, so a host can pin a reviewed OCR document.
//! - The "OCR failure must not block image/PDF export" acceptance criterion
//!   holds structurally at this layer: OCR results are sidecar metadata and
//!   not inputs to the export planner, so the export plan and manifest digest
//!   are identical whether OCR completed, failed, or never ran. The
//!   `tests/ocr_document.rs` integration tests pin that equivalence.
//!
//! Evidence category: validation code over synthetic fixtures only. No OCR
//! engine exists yet; nothing here proves recognition accuracy, and no
//! physical device is involved.

use serde::{Deserialize, Serialize};

use crate::checksum::sha256_hex;
use crate::error::DomainError;
use crate::limits::MAX_ID_LEN;
use crate::version::{parse_tag_version, ProtocolVersion};

/// Schema tag prefix for OCR result documents (`foldscan.ocr/<major>.<minor>`).
pub const OCR_SCHEMA_PREFIX: &str = "foldscan.ocr/";

/// Maximum raw JSON bytes for one OCR result document (bound before parse).
pub const MAX_OCR_DOCUMENT_BYTES: usize = 1024 * 1024;

/// Maximum recognized text characters in one document (sum over blocks).
pub const MAX_OCR_TEXT_CHARS: usize = 200_000;

/// Maximum recognized blocks in one document.
pub const MAX_OCR_BLOCKS: usize = 20_000;

/// Maximum language packs requested in one document.
pub const MAX_OCR_LANGUAGES: usize = 16;

/// Maximum length of one language-pack id (BCP 47 is far below this; the
/// bound is a hostile-input guard, not a spec claim).
pub const MAX_LANGUAGE_ID_LEN: usize = 35;

/// Confidence is an integer in per-mille: 1000 means fully confident.
/// Keeping it integral gives the UI an exact, screen-reader-friendly value.
pub const CONFIDENCE_PER_MILLE_MAX: u16 = 1000;

/// Why an OCR pass that was requested did not run to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrFailureCode {
    /// The engine could not decode the image bytes for this capture.
    ImageDecode,
    /// The engine reported an internal failure on this capture.
    Engine,
    /// The engine exceeded its host-side time budget.
    Timeout,
    /// A requested language pack was unavailable to the engine.
    LanguageUnavailable,
    /// The user cancelled the OCR pass.
    Cancelled,
}

/// Why OCR did not run for a capture that the host could have processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrSkipReason {
    /// The user has OCR disabled globally (the default posture).
    Disabled,
    /// No language configuration was selected for this run.
    NoLanguages,
    /// The capture's media type is not eligible for OCR.
    UnsupportedMediaType,
    /// A larger pass budget excluded this capture deterministically.
    OutOfBudget,
}

/// One recognized text block with its location and uncertainty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrBlock {
    /// Editable recognized text (bounded, no control characters).
    pub text: String,
    /// Per-mille confidence, 0..=1000.
    pub confidence: u16,
    /// Pixel bounding box inside the processed frame the run consumed.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The terminal status of one OCR pass over one capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OcrStatus {
    /// Recognition finished; blocks carry the result (possibly zero blocks
    /// for a blank page — "completed with no text" is not a failure).
    Completed {
        /// Frame dimensions the coordinates are expressed in, so a later
        /// resize cannot silently invalidate boxes.
        frame_width: u32,
        frame_height: u32,
        blocks: Vec<OcrBlock>,
    },
    /// Recognition was attempted and failed with a stable code.
    Failed { code: OcrFailureCode },
    /// Recognition was not attempted, with a stable reason.
    Skipped { reason: OcrSkipReason },
}

/// A versioned, reviewable OCR result document for one capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrResult {
    /// Schema tag, e.g. `foldscan.ocr/0.1`.
    pub schema: String,
    /// The capture this document describes.
    pub capture_id: String,
    /// Explicit language-pack ids the run was configured with (non-empty,
    /// unique, bounded; empty means the document is malformed — "not
    /// configured" is `skipped`, and "not run" is *no document*).
    pub requested_languages: Vec<String>,
    pub status: OcrStatus,
}

impl OcrResult {
    /// Validate bounds and shape of a parsed OCR document.
    pub fn validate(&self) -> Result<ProtocolVersion, DomainError> {
        let version = parse_tag_version(OCR_SCHEMA_PREFIX, &self.schema, "ocr schema")?;

        if self.capture_id.is_empty() || self.capture_id.len() > MAX_ID_LEN {
            return Err(DomainError::invalid_request("ocr capture_id out of bounds"));
        }
        if self.requested_languages.is_empty() {
            return Err(DomainError::invalid_request(
                "ocr document must record its requested language configuration",
            ));
        }
        if self.requested_languages.len() > MAX_OCR_LANGUAGES {
            return Err(DomainError::invalid_request(format!(
                "ocr requests {} languages, over the {} limit",
                self.requested_languages.len(),
                MAX_OCR_LANGUAGES
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for lang in &self.requested_languages {
            validate_language_id(lang)?;
            if !seen.insert(lang.as_str()) {
                return Err(DomainError::invalid_request(format!(
                    "duplicate language pack in ocr document: {}",
                    lang
                )));
            }
        }

        match &self.status {
            OcrStatus::Completed {
                frame_width,
                frame_height,
                blocks,
            } => {
                if *frame_width == 0 || *frame_height == 0 {
                    return Err(DomainError::invalid_request(
                        "completed ocr must declare a non-zero frame",
                    ));
                }
                if blocks.len() > MAX_OCR_BLOCKS {
                    return Err(DomainError::invalid_request(format!(
                        "ocr has {} blocks, over the {} limit",
                        blocks.len(),
                        MAX_OCR_BLOCKS
                    )));
                }
                let (fw, fh) = (*frame_width as u64, *frame_height as u64);
                let mut text_chars = 0usize;
                for (i, b) in blocks.iter().enumerate() {
                    if b.confidence > CONFIDENCE_PER_MILLE_MAX {
                        return Err(DomainError::invalid_request(format!(
                            "ocr block {} confidence {} exceeds per-mille bound",
                            i, b.confidence
                        )));
                    }
                    if b.width == 0 || b.height == 0 {
                        return Err(DomainError::invalid_request(format!(
                            "ocr block {} has an empty bounding box",
                            i
                        )));
                    }
                    let right = b.x as u64 + b.width as u64;
                    let bottom = b.y as u64 + b.height as u64;
                    if right > fw || bottom > fh {
                        return Err(DomainError::invalid_request(format!(
                            "ocr block {} bounding box escapes the declared frame",
                            i
                        )));
                    }
                    if b.text.chars().any(|c| c.is_control()) {
                        return Err(DomainError::invalid_request(format!(
                            "ocr block {} text contains control characters",
                            i
                        )));
                    }
                    text_chars = text_chars.saturating_add(b.text.chars().count());
                }
                if text_chars > MAX_OCR_TEXT_CHARS {
                    return Err(DomainError::invalid_request(format!(
                        "ocr text exceeds {} characters in total",
                        MAX_OCR_TEXT_CHARS
                    )));
                }
            }
            OcrStatus::Failed { .. } | OcrStatus::Skipped { .. } => {}
        }
        Ok(version)
    }

    /// Parse and validate an untrusted OCR result document. Bounds are
    /// checked on raw bytes before `serde_json` allocates.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, DomainError> {
        if bytes.is_empty() {
            return Err(DomainError::invalid_request("ocr document is empty"));
        }
        if bytes.len() > MAX_OCR_DOCUMENT_BYTES {
            return Err(DomainError::invalid_request(format!(
                "ocr document exceeds {} byte bound",
                MAX_OCR_DOCUMENT_BYTES
            )));
        }
        let doc: OcrResult = serde_json::from_slice(bytes).map_err(|e| {
            let cut = e.to_string();
            let cut = cut.split(" at line").next().unwrap_or(&cut);
            DomainError::invalid_request(format!("ocr document JSON invalid: {}", cut))
        })?;
        doc.validate()?;
        Ok(doc)
    }

    /// Deterministic integrity digest over the document's semantic content,
    /// using the same canonicalization (sorted object keys) as the recipe
    /// and export-manifest digests.
    pub fn digest(&self) -> String {
        let value = serde_json::to_value(self).expect("ocr serialization is infallible");
        let canonical = canonical_json_for_digest(&value);
        sha256_hex(canonical.as_bytes())
    }
}

/// Validate one language-pack id: ASCII alphanumerics plus `-`, bounded,
/// non-empty, no leading/trailing/doubled separator. This is a hostile-input
/// shape guard; actual pack availability is the engine's call.
fn validate_language_id(lang: &str) -> Result<(), DomainError> {
    if lang.is_empty() || lang.len() > MAX_LANGUAGE_ID_LEN {
        return Err(DomainError::invalid_request(format!(
            "language id {:?} is out of bounds",
            lang
        )));
    }
    if !lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(DomainError::invalid_request(format!(
            "language id {:?} contains forbidden characters",
            lang
        )));
    }
    if lang.starts_with('-') || lang.ends_with('-') || lang.contains("--") {
        return Err(DomainError::invalid_request(format!(
            "language id {:?} is malformed",
            lang
        )));
    }
    Ok(())
}

/// Canonicalize with sorted object keys so digests are map-iteration stable
/// (same rule as the export manifest and recipe digests).
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

/// The narrow seam an offline OCR engine adapter will implement later.
///
/// Contract for implementors:
/// - `recognize` is called once per capture after processing; it must be
///   pure with respect to its inputs (same `image` + `languages` => a
///   digest-reproducible document is achievable, though engines may record
///   engine-version metadata outside this slice's document).
/// - It must return `Ok` with a terminal `failed`/`skipped` status rather
///   than `Err` for ordinary recognition problems, so the caller can keep
///   going: an OCR failure must never block image/PDF export.
/// - It must perform no network access (offline requirement) and must not
///   mutate the image buffer the caller may still use for export.
/// - `supported_languages` is discovery only; `recognize` re-validates.
pub trait OcrProvider {
    /// Language pack ids this provider can currently service offline.
    fn supported_languages(&self) -> Vec<String>;

    /// Run recognition over one capture's processed image.
    fn recognize(
        &self,
        capture_id: &str,
        image: &crate::image::GrayFrame,
        languages: &[String],
    ) -> Result<OcrResult, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str, confidence: u16) -> OcrBlock {
        OcrBlock {
            text: text.to_string(),
            confidence,
            x: 0,
            y: 0,
            width: 100,
            height: 20,
        }
    }

    fn completed(blocks: Vec<OcrBlock>) -> OcrResult {
        OcrResult {
            schema: "foldscan.ocr/0.1".to_string(),
            capture_id: "cap-a".to_string(),
            requested_languages: vec!["eng".to_string()],
            status: OcrStatus::Completed {
                frame_width: 1000,
                frame_height: 1400,
                blocks,
            },
        }
    }

    #[test]
    fn completed_document_validates_and_digest_is_stable() {
        let doc = completed(vec![block("hello", 950)]);
        assert!(doc.validate().is_ok());
        assert_eq!(doc.digest(), doc.clone().digest());
    }

    #[test]
    fn confidence_bound_is_inclusive_per_mille() {
        assert!(completed(vec![block("x", 1000)]).validate().is_ok());
        let err = completed(vec![block("x", 1001)]).validate().unwrap_err();
        assert!(err.message.contains("per-mille"));
    }

    #[test]
    fn box_must_stay_in_declared_frame() {
        let mut b = block("x", 500);
        b.x = 950; // 950 + 100 > 1000
        let err = completed(vec![b]).validate().unwrap_err();
        assert!(err.message.contains("escapes the declared frame"));
    }

    #[test]
    fn control_characters_rejected_in_text() {
        // Block structure carries layout; text fields must never smuggle
        // control characters (newline included) into downstream renderers.
        let err = completed(vec![block("bad\ntext", 500)])
            .validate()
            .unwrap_err();
        assert!(err.message.contains("control characters"));
        let err = completed(vec![block("bad\x01text", 500)])
            .validate()
            .unwrap_err();
        assert!(err.message.contains("control characters"));
        let err = completed(vec![block("bad\u{7f}text", 500)])
            .validate()
            .unwrap_err();
        assert!(err.message.contains("control characters"));
    }

    #[test]
    fn language_list_must_be_unique_and_well_formed() {
        let mut doc = completed(vec![]);
        doc.requested_languages = vec!["eng".into(), "eng".into()];
        assert!(doc.validate().unwrap_err().message.contains("duplicate"));
        doc.requested_languages = vec!["eng lat".into()];
        assert!(doc.validate().unwrap_err().message.contains("forbidden"));
        doc.requested_languages = vec![];
        assert!(doc.validate().unwrap_err().message.contains("language"));
    }

    #[test]
    fn unknown_major_fails_safe_minor_is_tolerated() {
        let mut doc = completed(vec![]);
        doc.schema = "foldscan.ocr/1.0".to_string();
        assert_eq!(
            doc.validate().unwrap_err().category,
            crate::error::Category::UnsupportedVersion
        );
        doc.schema = "foldscan.ocr/0.9".to_string();
        assert!(doc.validate().is_ok());
    }
}
