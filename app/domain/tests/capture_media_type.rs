//! Declared capture media type: validated at manifest-parse time and
//! carried faithfully through import → export-session synthesis → layout
//! planning (issue #51, part of #5).
//!
//! Each test builds a throwaway `FOLDSCAN/` tree in a temp dir. Fixtures
//! are synthetic byte blobs, not real page images. These are mocked/software
//! tests, not physical device integration evidence.

use foldscan_domain::checksum::sha256_hex;
use foldscan_domain::export::{
    export_sessions_from_import, plan_export, ContentKind, ExportManifest,
};
use foldscan_domain::import::import_volume;
use foldscan_domain::manifest::{DEFAULT_CAPTURE_MEDIA_TYPE, SUPPORTED_CAPTURE_MEDIA_TYPES};
use foldscan_domain::Category;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const FAKE_IMAGE: &[u8] = b"\xFF\xD8\xFF\xE0synthetic-page-image\xFF\xD9";

/// One capture entry spec for the fixture builder: file name, declared
/// media type (`None` = field omitted from session.json).
struct CaptureSpec {
    id: &'static str,
    file: &'static str,
    media_type: Option<&'static str>,
}

/// Write device.json plus one session (`sess-001`) holding `specs`, each
/// backed by identical FAKE_IMAGE bytes.
fn build_volume(root: &Path, specs: &[CaptureSpec]) {
    let foldscan = root.join("FOLDSCAN");
    let session = foldscan.join("sessions").join("sess-001");
    fs::create_dir_all(session.join("captures")).unwrap();

    fs::write(
        foldscan.join("device.json"),
        r#"{"protocol":{"major":0,"minor":1},"device_id":"fs-test-01","firmware_version":"0.1.0-dev","capabilities":["capture"]}"#,
    )
    .unwrap();

    let mut entries = Vec::with_capacity(specs.len());
    for spec in specs {
        fs::write(session.join("captures").join(spec.file), FAKE_IMAGE).unwrap();
        let media_field = match spec.media_type {
            Some(m) => format!(r#""media_type":"{}","#, m),
            None => String::new(),
        };
        entries.push(format!(
            r#"{{"capture_id":"{}","relative_path":"captures/{}",{}"bytes":{},"sha256":"{}","width_px":1600,"height_px":1200,"orientation":1,"captured_at":null}}"#,
            spec.id,
            spec.file,
            media_field,
            FAKE_IMAGE.len(),
            sha256_hex(FAKE_IMAGE)
        ));
    }
    fs::write(
        session.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","created_at":null,"clock_state":"unsynced","captures":[{}]}}"#,
            entries.join(",")
        ),
    )
    .unwrap();
}

#[test]
fn png_declared_capture_exports_with_png_extension() {
    let tmp = TempDir::new().unwrap();
    build_volume(
        tmp.path(),
        &[CaptureSpec {
            id: "cap-1",
            file: "cap-1.png",
            media_type: Some("image/png"),
        }],
    );

    let plan = import_volume(tmp.path()).expect("png-declared volume must import");
    assert_eq!(plan.sessions[0].captures[0].media_type, "image/png");

    let sessions = export_sessions_from_import(&plan.sessions);
    assert_eq!(sessions[0].pages[0].media_type, "image/png");

    let layout = plan_export(&sessions, &[]).expect("png session plans");
    let originals: Vec<&str> = layout
        .files
        .iter()
        .filter(|f| f.content_kind == ContentKind::Original)
        .map(|f| f.relative_path.as_str())
        .collect();
    // The regression this slice closes: PNG bytes must never be planned
    // under a `.jpg` name again.
    assert_eq!(originals, ["originals/sess-001/cap-1.png"]);
}

#[test]
fn mixed_declared_types_layout_each_extension_correctly() {
    let tmp = TempDir::new().unwrap();
    build_volume(
        tmp.path(),
        &[
            CaptureSpec {
                id: "cap-a",
                file: "cap-a.jpg",
                media_type: Some("image/jpeg"),
            },
            CaptureSpec {
                id: "cap-b",
                file: "cap-b.png",
                media_type: Some("image/png"),
            },
            CaptureSpec {
                id: "cap-c",
                file: "cap-c.jpg",
                media_type: None,
            },
        ],
    );

    let plan = import_volume(tmp.path()).expect("mixed volume must import");
    let captures: Vec<(&str, &str)> = plan.sessions[0]
        .captures
        .iter()
        .map(|c| (c.capture_id.as_str(), c.media_type.as_str()))
        .collect();
    assert_eq!(
        captures,
        vec![
            ("cap-a", "image/jpeg"),
            ("cap-b", "image/png"),
            ("cap-c", DEFAULT_CAPTURE_MEDIA_TYPE),
        ]
    );

    let sessions = export_sessions_from_import(&plan.sessions);
    let layout = plan_export(&sessions, &[]).expect("mixed session plans");
    let originals: Vec<&str> = layout
        .files
        .iter()
        .filter(|f| f.content_kind == ContentKind::Original)
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        originals,
        [
            "originals/sess-001/cap-a.jpg",
            "originals/sess-001/cap-b.png",
            "originals/sess-001/cap-c.jpg",
        ]
    );
}

#[test]
fn omitted_media_type_defaults_to_jpeg() {
    let tmp = TempDir::new().unwrap();
    build_volume(
        tmp.path(),
        &[CaptureSpec {
            id: "cap-1",
            file: "cap-1.jpg",
            media_type: None,
        }],
    );

    let plan = import_volume(tmp.path()).expect("omitted media_type must import");
    assert_eq!(
        plan.sessions[0].captures[0].media_type,
        DEFAULT_CAPTURE_MEDIA_TYPE
    );
    assert_eq!(DEFAULT_CAPTURE_MEDIA_TYPE, "image/jpeg");
    // The default must be a member of the closed vocabulary it validates
    // against, or the default itself would be rejected.
    assert!(SUPPORTED_CAPTURE_MEDIA_TYPES.contains(&DEFAULT_CAPTURE_MEDIA_TYPE));
}

#[test]
fn unknown_declared_media_type_is_rejected_before_file_io() {
    for bad in ["image/tiff", "application/x-executable", "IMAGE/JPEG", ""] {
        let tmp = TempDir::new().unwrap();
        build_volume(
            tmp.path(),
            &[CaptureSpec {
                id: "cap-1",
                file: "cap-1.jpg",
                media_type: Some(bad),
            }],
        );
        let err = import_volume(tmp.path())
            .expect_err(&format!("{bad:?} must be rejected before capture IO"));
        assert_eq!(err.category, Category::InvalidRequest, "for {bad:?}");
        // Message names the offending capture and value (bounded, no paths).
        assert!(err.message.contains("cap-1"), "for {bad:?}: {err}");
        assert!(err.message.contains("media type"), "for {bad:?}: {err}");
        assert!(!err.message.contains(&tmp.path().display().to_string()));
    }
}

#[test]
fn one_bad_capture_fails_the_whole_session_import() {
    // Validation runs over every declared entry before any capture file is
    // opened, so a single unknown type aborts the session — the good JPEG
    // sibling never becomes a partial import.
    let tmp = TempDir::new().unwrap();
    build_volume(
        tmp.path(),
        &[
            CaptureSpec {
                id: "cap-good",
                file: "cap-good.jpg",
                media_type: Some("image/jpeg"),
            },
            CaptureSpec {
                id: "cap-bad",
                file: "cap-bad.jpg",
                media_type: Some("image/tiff"),
            },
        ],
    );
    let err = import_volume(tmp.path()).expect_err("mixed good/bad must fail");
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("cap-bad"));
}

#[test]
fn declared_type_change_changes_the_export_manifest_digest() {
    // The exported layout now depends on the declared media type, so an
    // attacker flipping `image/png` -> `image/jpeg` (or vice versa) in an
    // otherwise identical manifest produces a different manifest digest.
    let digest_for = |media: Option<&'static str>| {
        let tmp = TempDir::new().unwrap();
        build_volume(
            tmp.path(),
            &[CaptureSpec {
                id: "cap-1",
                file: "cap-1.jpg",
                media_type: media,
            }],
        );
        let plan = import_volume(tmp.path()).expect("imports");
        let sessions = export_sessions_from_import(&plan.sessions);
        let layout = plan_export(&sessions, &[]).expect("plans");
        let originals: Vec<String> = layout
            .files
            .iter()
            .filter(|f| f.content_kind == ContentKind::Original)
            .map(|f| f.relative_path.clone())
            .collect();
        assert_eq!(
            originals.len(),
            1,
            "one original planned for {:?}",
            media.unwrap_or("omitted")
        );
        ExportManifest::from_plan(&layout, "sess-001")
            .expect("manifest")
            .digest()
    };
    assert_ne!(
        digest_for(Some("image/jpeg")),
        digest_for(Some("image/png"))
    );
    // Omitted declares the same JPEG default, so it must digest identically.
    assert_eq!(
        digest_for(None),
        digest_for(Some("image/jpeg")),
        "the documented default must be byte-equivalent to declaring it"
    );
}
