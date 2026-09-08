//! Device and session manifest types with strict pre-validation.
//!
//! Deserialization is intentionally *lenient about unknown fields* (forward
//! minor compatibility) but every value is then checked against protocol
//! bounds *before* any file IO or bulk allocation happens.

use std::collections::HashSet;

use serde::Deserialize;

use crate::checksum::is_lowercase_hex_sha256;
use crate::error::DomainError;
use crate::limits::*;
use crate::version::{parse_session_schema, ProtocolVersion};

fn default_null_time() -> Option<String> {
    None
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceManifest {
    pub protocol: ProtocolVersion,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub firmware_version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl DeviceManifest {
    /// Validate a parsed `device.json` against protocol bounds.
    pub fn validate(&self) -> Result<(), DomainError> {
        self.protocol.validate()?;
        validate_identifier("device_id", &self.device_id)?;
        validate_identifier("firmware_version", &self.firmware_version)?;
        if self.capabilities.len() > 64 {
            return Err(DomainError::invalid_request("capability list too long"));
        }
        for c in &self.capabilities {
            validate_identifier("capability", c)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionManifest {
    pub schema: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default = "default_null_time")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub clock_state: Option<String>,
    #[serde(default)]
    pub captures: Vec<CaptureEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureEntry {
    #[serde(default)]
    pub capture_id: String,
    #[serde(default)]
    pub relative_path: String,
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub width_px: u64,
    #[serde(default)]
    pub height_px: u64,
    #[serde(default)]
    pub orientation: u16,
    #[serde(default = "default_null_time")]
    pub captured_at: Option<String>,
    #[serde(default)]
    pub camera: serde_json::Value,
    #[serde(default)]
    pub illumination: serde_json::Value,
}

impl SessionManifest {
    /// Validate a parsed `session.json`. Rejects oversized capture lists,
    /// duplicate capture IDs, malformed checksums, out-of-range dimensions,
    /// unsafe relative paths, and declared totals beyond `MAX_SESSION_TOTAL_BYTES`.
    pub fn validate(&self) -> Result<ProtocolVersion, DomainError> {
        let version = parse_session_schema(&self.schema)?;
        validate_identifier("session_id", &self.session_id)?;

        if self.captures.len() > MAX_CAPTURES_PER_SESSION {
            return Err(DomainError::invalid_request(format!(
                "session lists {} captures, over the {} limit",
                self.captures.len(),
                MAX_CAPTURES_PER_SESSION
            )));
        }

        if let Some(state) = &self.clock_state {
            if !matches!(state.as_str(), "synced" | "unsynced" | "unknown") {
                return Err(DomainError::invalid_request(
                    "clock_state is not a known value",
                ));
            }
        }

        let mut seen_ids: HashSet<&str> = HashSet::new();
        let mut total_declared: u64 = 0;
        for entry in &self.captures {
            validate_identifier("capture_id", &entry.capture_id)?;
            if !seen_ids.insert(entry.capture_id.as_str()) {
                return Err(DomainError::invalid_request(format!(
                    "duplicate capture_id in manifest: {}",
                    entry.capture_id
                )));
            }
            if !is_lowercase_hex_sha256(&entry.sha256) {
                return Err(DomainError::invalid_request(format!(
                    "capture {} has a malformed sha256 field",
                    entry.capture_id
                )));
            }
            if entry.bytes > MAX_CAPTURE_BYTES {
                return Err(DomainError::invalid_request(format!(
                    "capture {} declares {} bytes, over the per-file limit",
                    entry.capture_id, entry.bytes
                )));
            }
            if entry.width_px > MAX_DIMENSION_PX || entry.height_px > MAX_DIMENSION_PX {
                return Err(DomainError::invalid_request(format!(
                    "capture {} declares dimensions {}x{} beyond the {}px bound",
                    entry.capture_id, entry.width_px, entry.height_px, MAX_DIMENSION_PX
                )));
            }
            if entry.orientation > 8 {
                return Err(DomainError::invalid_request(format!(
                    "capture {} has EXIF orientation {} outside 1..=8",
                    entry.capture_id, entry.orientation
                )));
            }
            // Path is syntax-checked here; containment is enforced at IO time.
            crate::paths::safe_join(std::path::Path::new("/"), &entry.relative_path)?;
            total_declared = total_declared
                .checked_add(entry.bytes)
                .ok_or_else(|| DomainError::invalid_request("capture byte totals overflow"))?;
        }
        if total_declared > MAX_SESSION_TOTAL_BYTES {
            return Err(DomainError::invalid_request(format!(
                "session declares {} total bytes, over the {} limit",
                total_declared, MAX_SESSION_TOTAL_BYTES
            )));
        }
        Ok(version)
    }
}

fn validate_identifier(field: &str, value: &str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::invalid_request(format!("{} is empty", field)));
    }
    if value.len() > MAX_ID_LEN {
        return Err(DomainError::invalid_request(format!(
            "{} exceeds {} bytes",
            field, MAX_ID_LEN
        )));
    }
    if value.chars().any(|c| c == '\0' || c == '/' || c == '\\') {
        return Err(DomainError::invalid_request(format!(
            "{} contains forbidden characters",
            field
        )));
    }
    Ok(())
}
