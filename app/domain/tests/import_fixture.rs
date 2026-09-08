//! End-to-end import tests over synthetic removable-media fixtures.
//!
//! Each test builds a throwaway `FOLDSCAN/` tree in a temp dir, then runs the
//! importer. Fixtures are synthetic JPEG-stuffed bytes, not real documents.
//! These are mocked/software tests, not physical device integration evidence.

use foldscan_domain::checksum::sha256_hex;
use foldscan_domain::import::import_volume;
use foldscan_domain::Category;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const FAKE_JPEG: &[u8] = b"\xFF\xD8\xFF\xE0synthetic-page-image\xFF\xD9";

/// Build a minimal valid volume: device.json + one session with one capture.
fn build_valid_volume(root: &Path) -> String {
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
    sha
}

#[test]
fn valid_volume_imports_with_verified_checksums() {
    let tmp = TempDir::new().unwrap();
    let sha = build_valid_volume(tmp.path());
    let plan = import_volume(tmp.path()).expect("valid volume must import");
    assert_eq!(plan.device_id, "fs-test-01");
    assert_eq!(plan.sessions.len(), 1);
    assert_eq!(plan.sessions[0].session_id, "sess-001");
    let caps = &plan.sessions[0].captures;
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].sha256, sha);
    assert_eq!(caps[0].bytes, FAKE_JPEG.len() as u64);
    // Import plan must never carry absolute paths.
    assert!(!caps[0].relative_path.starts_with('/'));
}

#[test]
fn foldscan_dir_can_be_the_root_itself() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let inner = tmp.path().join("FOLDSCAN");
    let plan = import_volume(&inner).expect("root-that-is-FOLDSCAN must import");
    assert_eq!(plan.sessions.len(), 1);
}

#[test]
fn missing_foldscan_dir_is_storage_unavailable() {
    let tmp = TempDir::new().unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::StorageUnavailable);
}

#[test]
fn truncated_json_is_invalid_request() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let dev = tmp.path().join("FOLDSCAN").join("device.json");
    fs::write(&dev, b"{\"protocol\":{\"major\":0,\"min").unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn oversized_manifest_is_invalid_request() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let dev = tmp.path().join("FOLDSCAN").join("device.json");
    // Pad with a huge capabilities array beyond MAX_MANIFEST_BYTES (256 KiB).
    let huge = format!(
        r#"{{"protocol":{{"major":0,"minor":1}},"device_id":"d","firmware_version":"v","capabilities":["a"{}]}}"#,
        ",\"a\"".repeat(70_000)
    );
    assert!(huge.len() as u64 > foldscan_domain::limits::MAX_MANIFEST_BYTES);
    fs::write(&dev, huge).unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    // Error text must not leak the absolute host path.
    assert!(!err.message.contains(&tmp.path().display().to_string()));
}

#[test]
fn unknown_major_version_fails_without_deleting_files() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let dev = tmp.path().join("FOLDSCAN").join("device.json");
    fs::write(
        &dev,
        r#"{"protocol":{"major":7,"minor":0},"device_id":"d","firmware_version":"v","capabilities":[]}"#,
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::UnsupportedVersion);
    // Fail-safe: nothing on the "device" was removed.
    assert!(tmp.path().join("FOLDSCAN").join("sessions").exists());
}

#[test]
fn traversal_path_in_manifest_is_rejected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let session = tmp
        .path()
        .join("FOLDSCAN")
        .join("sessions")
        .join("sess-001");
    let sha = sha256_hex(FAKE_JPEG);
    fs::write(
        session.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","captures":[{{"capture_id":"cap-x","relative_path":"../../etc/passwd","bytes":{},"sha256":"{}","width_px":10,"height_px":10}}]}}"#,
            FAKE_JPEG.len(), sha
        ),
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn duplicate_capture_ids_within_session_rejected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let session = tmp
        .path()
        .join("FOLDSCAN")
        .join("sessions")
        .join("sess-001");
    let sha = sha256_hex(FAKE_JPEG);
    fs::write(session.join("captures").join("cap-2.jpg"), FAKE_JPEG).unwrap();
    let entry = |id: &str, file: &str| {
        format!(
            r#"{{"capture_id":"{}","relative_path":"captures/{}","bytes":{},"sha256":"{}","width_px":10,"height_px":10}}"#,
            id,
            file,
            FAKE_JPEG.len(),
            sha
        )
    };
    fs::write(
        session.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","captures":[{},{}]}}"#,
            entry("cap-1", "cap-1.jpg"),
            entry("cap-1", "cap-2.jpg")
        ),
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("duplicate"));
}

#[test]
fn duplicate_capture_ids_across_sessions_rejected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let root = tmp.path().join("FOLDSCAN");
    let s2 = root.join("sessions").join("sess-002");
    fs::create_dir_all(s2.join("captures")).unwrap();
    fs::write(s2.join("captures").join("cap-1.jpg"), FAKE_JPEG).unwrap();
    let sha = sha256_hex(FAKE_JPEG);
    fs::write(
        s2.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-002","captures":[{{"capture_id":"cap-1","relative_path":"captures/cap-1.jpg","bytes":{},"sha256":"{}","width_px":10,"height_px":10}}]}}"#,
            FAKE_JPEG.len(), sha
        ),
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("more than one session"));
}

#[test]
fn checksum_mismatch_is_detected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    // Flip file content while keeping declared size identical.
    let mut alt = FAKE_JPEG.to_vec();
    let last = alt.len() - 1;
    alt[last] = 0xFE;
    assert_eq!(alt.len(), FAKE_JPEG.len());
    fs::write(
        tmp.path()
            .join("FOLDSCAN")
            .join("sessions")
            .join("sess-001")
            .join("captures")
            .join("cap-1.jpg"),
        &alt,
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
}

#[test]
fn declared_size_mismatch_is_detected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let session = tmp
        .path()
        .join("FOLDSCAN")
        .join("sessions")
        .join("sess-001");
    fs::write(session.join("captures").join("cap-1.jpg"), b"tiny").unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
}

#[test]
fn missing_declared_capture_is_storage_unavailable() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    fs::remove_file(
        tmp.path()
            .join("FOLDSCAN")
            .join("sessions")
            .join("sess-001")
            .join("captures")
            .join("cap-1.jpg"),
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::StorageUnavailable);
}

#[test]
fn excessive_dimensions_rejected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let session = tmp
        .path()
        .join("FOLDSCAN")
        .join("sessions")
        .join("sess-001");
    let sha = sha256_hex(FAKE_JPEG);
    fs::write(
        session.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","captures":[{{"capture_id":"cap-1","relative_path":"captures/cap-1.jpg","bytes":{},"sha256":"{}","width_px":4294967295,"height_px":10}}]}}"#,
            FAKE_JPEG.len(), sha
        ),
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn malformed_sha256_rejected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let session = tmp
        .path()
        .join("FOLDSCAN")
        .join("sessions")
        .join("sess-001");
    fs::write(
        session.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","captures":[{{"capture_id":"cap-1","relative_path":"captures/cap-1.jpg","bytes":{},"sha256":"DEADBEEF","width_px":10,"height_px":10}}]}}"#,
            FAKE_JPEG.len()
        ),
    )
    .unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn unknown_clock_state_rejected() {
    let tmp = TempDir::new().unwrap();
    build_valid_volume(tmp.path());
    let session = tmp
        .path()
        .join("FOLDSCAN")
        .join("sessions")
        .join("sess-001");
    let body = fs::read_to_string(session.join("session.json")).unwrap();
    let bad = body.replace(
        "\"clock_state\":\"unsynced\"",
        "\"clock_state\":\"telepathic\"",
    );
    fs::write(session.join("session.json"), bad).unwrap();
    let err = import_volume(tmp.path()).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
}
