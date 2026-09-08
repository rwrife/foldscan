//! Unit tests for relative-path safety.

use foldscan_domain::paths::safe_join;
use std::path::Path;

fn root() -> &'static Path {
    Path::new("/session-root")
}

#[test]
fn accepts_plain_relative_paths() {
    let ok = safe_join(root(), "captures/abc.jpg").unwrap();
    assert_eq!(ok, Path::new("/session-root/captures/abc.jpg"));
}

#[test]
fn rejects_parent_traversal() {
    for bad in [
        "../etc/passwd",
        "captures/../../etc/passwd",
        "..",
        "captures/..",
    ] {
        let err = safe_join(root(), bad).expect_err("must reject traversal");
        assert_eq!(
            err.category,
            foldscan_domain::Category::InvalidRequest,
            "wrong category for {:?}",
            bad
        );
    }
}

#[test]
fn rejects_absolute_and_drive_prefixes() {
    for bad in [
        "/etc/passwd",
        "C:/Windows/system32",
        "c:/evil",
        "\\server\\share",
    ] {
        assert!(safe_join(root(), bad).is_err(), "must reject {:?}", bad);
    }
}

#[test]
fn rejects_empty_and_current_dir() {
    assert!(safe_join(root(), "").is_err());
    assert!(safe_join(root(), "./captures/a.jpg").is_err());
    assert!(safe_join(root(), "captures/./a.jpg").is_err());
}

#[test]
fn rejects_backslash_anywhere() {
    assert!(safe_join(root(), "captures\\a.jpg").is_err());
}

#[test]
fn rejects_oversized_paths() {
    let long = format!("captures/{}.jpg", "a".repeat(2000));
    assert!(safe_join(root(), &long).is_err());
}
