#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! FoldScan companion desktop shell (scaffold, issue #37).
//!
//! Scope: a buildable, linted, tested Tauri 2 shell that depends on the
//! `foldscan-domain` crate by path and exposes exactly one command,
//! [`companion_status`], returning a small versioned status document. No UI
//! workflows exist yet; this crate only proves the toolchain wiring (Rust
//! shell against the Linux WebKitGTK backend, pinned dependencies, CI gates).

use serde::Serialize;

/// Schema tag for the status document returned by [`companion_status`].
pub const STATUS_SCHEMA: &str = "foldscan.companion.status/0.1";

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

/// The single command registered by the scaffold. It performs no I/O and
/// takes no arguments, so the capability surface stays `core:default`.
#[tauri::command]
fn companion_status() -> CompanionStatus {
    status()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![companion_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_status_schema_and_shell_version() {
        let s = status();
        assert_eq!(s.schema, STATUS_SCHEMA);
        assert_eq!(s.app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn status_reports_the_domain_protocol_constants_verbatim() {
        // Proves the shell is linked against the real domain crate and not a
        // hardcoded copy of its protocol numbers.
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
}
