//! Canonical relative-path safety.
//!
//! Device-supplied paths are untrusted. A path is accepted only when it is a
//! relative, slash-separated UTF-8 path whose canonicalized form stays inside
//! the session root: no absolute paths, no drive prefixes, no `..`, no empty
//! or `.` components, and no backslash path separators.

use std::path::{Component, Path, PathBuf};

use crate::error::DomainError;
use crate::limits::MAX_PATH_LEN;

/// Validate a device-declared relative path and return it as a `PathBuf`
/// joined safely under `root`.
///
/// Rejection rules (protocol § Session manifest):
/// - must be non-empty and within `MAX_PATH_LEN` bytes;
/// - must not contain backslashes (a Windows drive/sep smuggling vector);
/// - must not be absolute and must not carry a drive prefix;
/// - every component must be a plain `Normal` name: no `..`, no `.`, no empty;
/// - no component may be the reserved `..` at any depth, and the joined
///   canonical form must remain under `root` (verified after `..`-free join,
///   so symlink escapes are checked by the caller at IO time).
pub fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, DomainError> {
    if relative.is_empty() {
        return Err(DomainError::invalid_request("relative path is empty"));
    }
    if relative.len() > MAX_PATH_LEN {
        return Err(DomainError::invalid_request(format!(
            "relative path exceeds {} bytes",
            MAX_PATH_LEN
        )));
    }
    if relative.contains('\\') {
        return Err(DomainError::invalid_request(
            "relative path contains a backslash separator",
        ));
    }
    if relative.starts_with('/') {
        return Err(DomainError::invalid_request("relative path is absolute"));
    }
    // `Path::components()` silently normalizes away interior `.` segments and
    // trailing slashes, so inspect the raw slash-separated segments first.
    for seg in relative.split('/') {
        if seg.is_empty() || seg == "." {
            return Err(DomainError::invalid_request(
                "relative path contains an empty or '.' segment",
            ));
        }
    }
    // Windows drive-prefix check ("C:/...") applies regardless of host OS.
    let bytes = relative.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(DomainError::invalid_request(
            "relative path has a drive prefix",
        ));
    }

    let rel = Path::new(relative);
    for c in rel.components() {
        match c {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.is_empty() {
                    return Err(DomainError::invalid_request(
                        "relative path has an empty component",
                    ));
                }
            }
            _ => {
                // RootDir, ParentDir, CurDir, Prefix all rejected.
                return Err(DomainError::invalid_request(
                    "relative path contains a non-normal component (absolute, '.', or '..')",
                ));
            }
        }
    }

    let joined = root.join(rel);
    // Belt-and-braces containment check on the lexical form.
    if !joined.starts_with(root) {
        return Err(DomainError::invalid_request(
            "relative path escapes the session root",
        ));
    }
    Ok(joined)
}
