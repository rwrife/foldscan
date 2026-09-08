//! Hard input bounds applied *before* any large allocation.
//!
//! The protocol requires the app to cap manifest size, capture count, image
//! dimensions, and total imported bytes before allocation. These constants are
//! deliberately conservative planning bounds, not measured device capability.

/// Maximum size in bytes of a single device.json or session.json read.
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// Maximum captures listed in one session manifest.
pub const MAX_CAPTURES_PER_SESSION: usize = 2_000;

/// Maximum declared pixel dimension (width or height) for a capture.
pub const MAX_DIMENSION_PX: u64 = 32_768;

/// Maximum declared byte size of a single capture file.
pub const MAX_CAPTURE_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum declared total bytes across all captures in one session.
pub const MAX_SESSION_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Maximum length of identifier-like strings (device_id, session_id,
/// capture_id, firmware_version).
pub const MAX_ID_LEN: usize = 128;

/// Maximum length of a relative path string.
pub const MAX_PATH_LEN: usize = 1_024;

/// Maximum number of sessions enumerated under FOLDSCAN/sessions.
pub const MAX_SESSIONS: usize = 1_000;
