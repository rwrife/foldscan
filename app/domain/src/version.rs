//! Protocol/schema version negotiation.
//!
//! Rules from `docs/protocol.md`:
//! - Unknown *major* versions fail safely without deletion.
//! - Newer *minor* versions are accepted only when required fields and
//!   semantics remain compatible; unknown extra fields are tolerated.

use crate::error::DomainError;
use serde::Deserialize;

/// The highest protocol major version this core understands.
pub const SUPPORTED_MAJOR: u32 = 0;

/// The protocol minor version this core was written against.
pub const KNOWN_MINOR: u32 = 1;

/// `"foldscan.session/<major>.<minor>"` schema tag used by session manifests.
pub const SESSION_SCHEMA_PREFIX: &str = "foldscan.session/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ProtocolVersion {
    pub major: u32,
    pub minor: u32,
}

impl ProtocolVersion {
    /// Accept only the supported major; a newer minor is compatible because
    /// unknown fields are tolerated and required fields keep their meaning.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.major != SUPPORTED_MAJOR {
            return Err(DomainError::unsupported_version(format!(
                "protocol major {} is not supported (this build supports major {})",
                self.major, SUPPORTED_MAJOR
            )));
        }
        Ok(())
    }
}

/// Parse and validate a session `schema` tag such as `foldscan.session/0.1`.
pub fn parse_session_schema(schema: &str) -> Result<ProtocolVersion, DomainError> {
    parse_tag_version(SESSION_SCHEMA_PREFIX, schema, "session schema")
}

/// Parse and validate a `<prefix><major>.<minor>` schema tag such as
/// `foldscan.recipe/0.1` or `foldscan.export/0.1`.
///
/// Unknown *major* versions map to `UnsupportedVersion` (fail safely);
/// malformed tags map to `InvalidRequest` (structural violation).
pub fn parse_tag_version(
    prefix: &str,
    schema: &str,
    label: &str,
) -> Result<ProtocolVersion, DomainError> {
    let rest = schema
        .strip_prefix(prefix)
        .ok_or_else(|| DomainError::invalid_request(format!("{} tag has unknown prefix", label)))?;
    let (major_s, minor_s) = rest.split_once('.').ok_or_else(|| {
        DomainError::invalid_request(format!("{} tag is missing a minor component", label))
    })?;
    let major = major_s
        .parse::<u32>()
        .map_err(|_| DomainError::invalid_request(format!("{} major is not a number", label)))?;
    let minor = minor_s
        .parse::<u32>()
        .map_err(|_| DomainError::invalid_request(format!("{} minor is not a number", label)))?;
    let v = ProtocolVersion { major, minor };
    v.validate()?;
    Ok(v)
}
