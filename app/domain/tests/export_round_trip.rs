//! End-to-end domain fixture: import a synthetic volume, review the pages
//! (rotate/reorder/remove-from-export), attach a processing recipe, plan the
//! export layout, and round-trip the portable export manifest.
//!
//! Evidence category: software fixture test on synthetic data. No physical
//! device, no image encoding, no PDF writing.

use std::collections::HashSet;
use std::path::Path;

use foldscan_domain::checksum::sha256_hex;
use foldscan_domain::{
    export_sessions_from_import, import_volume, plan_export, remove_from, reorder, ContentKind,
    ExportManifest, OpKind, ProcessingRecipe, RecipeOp,
};
use tempfile::TempDir;

const FAKE_JPEG: &[u8] = b"\xFF\xD8\xFF\xE0synthetic-page-image\xFF\xD9";

/// Build a valid volume with one session holding captures with the given ids.
fn build_volume(root: &Path, capture_ids: &[&str]) {
    let foldscan = root.join("FOLDSCAN");
    let session = foldscan.join("sessions").join("sess-001");
    std::fs::create_dir_all(session.join("captures")).unwrap();

    std::fs::write(
        foldscan.join("device.json"),
        r#"{"protocol":{"major":0,"minor":1},"device_id":"fs-test-01","firmware_version":"0.1.0-dev","capabilities":["capture"]}"#,
    )
    .unwrap();

    let entries: Vec<String> = capture_ids
        .iter()
        .map(|id| {
            std::fs::write(session.join("captures").join(format!("{}.jpg", id)), FAKE_JPEG).unwrap();
            format!(
                r#"{{"capture_id":"{}","relative_path":"captures/{}.jpg","media_type":"image/jpeg","bytes":{},"sha256":"{}","width_px":1600,"height_px":1200,"orientation":1,"captured_at":null}}"#,
                id,
                id,
                FAKE_JPEG.len(),
                sha256_hex(FAKE_JPEG)
            )
        })
        .collect();
    std::fs::write(
        session.join("session.json"),
        format!(
            r#"{{"schema":"foldscan.session/0.1","session_id":"sess-001","created_at":null,"clock_state":"unsynced","captures":[{}]}}"#,
            entries.join(",")
        ),
    )
    .unwrap();
}

#[test]
fn import_then_process_then_export_round_trip() {
    let dir = TempDir::new().unwrap();
    build_volume(dir.path(), &["p1", "p2", "p3"]);

    let plan = import_volume(dir.path()).expect("volume import");
    assert_eq!(plan.sessions.len(), 1);

    // Review stage: reorder and remove-from-export (originals untouched).
    let mut sessions = export_sessions_from_import(&plan.sessions);
    let s = &mut sessions[0];
    assert!(reorder(s, "p3", 0));
    let removed = remove_from(s, "p2").expect("p2 present");
    assert_eq!(s.pages.len(), 2);
    assert_eq!(s.pages[0].capture_id, "p3");

    // Process stage (planning only): bind a rotate+illumination recipe.
    let recipe = ProcessingRecipe {
        schema: "foldscan.recipe/0.1".to_string(),
        name: "deskew-quarter".to_string(),
        ops: vec![
            RecipeOp {
                kind: OpKind::Rotate,
                params: serde_json::json!({"degrees": 90}),
            },
            RecipeOp {
                kind: OpKind::Illumination,
                params: serde_json::json!({"mode": "flatten", "strength": 0.5}),
            },
        ],
    };
    recipe.validate().expect("recipe valid");
    let d = recipe.digest();
    let first = &mut s.pages[0];
    first.processed_media_type = Some("image/png".to_string());
    first.processed_sha256 = Some(sha256_hex(b"synthetic-derivative"));
    first.processed_bytes = Some(1234);
    first.recipe_digest = Some(d.clone());

    // Export planning: originals + derivative + recipe + manifest, no collisions.
    let export_plan = plan_export(&sessions, std::slice::from_ref(&recipe)).expect("export plan");
    let kinds: Vec<ContentKind> = export_plan.files.iter().map(|f| f.content_kind).collect();
    assert_eq!(kinds.last().unwrap(), &ContentKind::Manifest);
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == ContentKind::Original)
            .count(),
        2,
        "two surviving originals"
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == ContentKind::Derivative)
            .count(),
        1
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == ContentKind::Recipe).count(),
        1
    );
    // Removed page's original is NOT in the export (removed, not deleted).
    let paths: Vec<&str> = export_plan
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert!(!paths.iter().any(|p| p.contains("p2")));
    assert!(paths.iter().any(|p| p.contains("p3")));

    // Portable manifest round-trip and recipe binding.
    let session_id = sessions[0].session_id.clone();
    let manifest = ExportManifest::from_plan(&export_plan, &session_id).expect("manifest");
    assert_eq!(manifest.pages.len(), 2);
    assert_eq!(manifest.pages[0].capture_id, "p3");
    assert_eq!(manifest.pages[0].recipe_digest.as_deref(), Some(d.as_str()));

    let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    let back = ExportManifest::from_json_bytes(&bytes).expect("manifest parses back");
    assert_eq!(manifest.digest(), back.digest());

    // Uniqueness invariant across the surviving plan.
    let ids: HashSet<&str> = manifest
        .pages
        .iter()
        .map(|p| p.capture_id.as_str())
        .collect();
    assert_eq!(ids.len(), manifest.pages.len());

    // The removed page still carries its verified original checksum —
    // removal from export never destroyed the original's record.
    assert!(foldscan_domain::checksum::is_lowercase_hex_sha256(
        &removed.original_sha256
    ));
}
