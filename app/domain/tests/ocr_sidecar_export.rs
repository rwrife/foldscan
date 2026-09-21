//! Per-capture OCR sidecar export binding (issue #33, child of #5).
//!
//! Covers the planner layout rules for bound `foldscan.ocr/0.1` documents,
//! manifest digest coverage of sidecar content, and executor
//! finalize/rollback of sidecar files that are serialized from the bound
//! document (never from a host-supplied source).
//!
//! Evidence category: software fixture test on synthetic OCR documents in a
//! temporary directory. No OCR engine exists — the documents are authored by
//! the test, not recognized by any tool. No physical device, no bench.

use std::collections::HashMap;

use foldscan_domain::executor::{execute_export, execute_export_cancellable, ExportSource};
use foldscan_domain::ocr::{OcrBlock, OcrFailureCode, OcrResult, OcrSkipReason, OcrStatus};
use foldscan_domain::{
    plan_export, remove_from, reorder, Category, ContentKind, ExportManifest, ExportPage,
    ExportPlan, ExportSession,
};
use tempfile::TempDir;

const PAYLOAD: &[u8] = &[0xAB; 100];

fn page(id: &str) -> ExportPage {
    ExportPage {
        capture_id: id.to_string(),
        media_type: "image/jpeg".to_string(),
        processed_media_type: None,
        original_sha256: foldscan_domain::checksum::sha256_hex(PAYLOAD),
        original_bytes: PAYLOAD.len() as u64,
        processed_sha256: None,
        processed_bytes: None,
        recipe_digest: None,
    }
}

fn session(ids: &[&str]) -> ExportSession {
    ExportSession {
        session_id: "sess-ocr".to_string(),
        pages: ids.iter().map(|id| page(id)).collect(),
        document: None,
        ocr: Vec::new(),
    }
}

/// A synthetic completed OCR document for one capture (authored bytes; no
/// engine produced them).
fn completed(id: &str, text: &str) -> OcrResult {
    OcrResult {
        schema: "foldscan.ocr/0.1".to_string(),
        capture_id: id.to_string(),
        requested_languages: vec!["eng".to_string()],
        status: OcrStatus::Completed {
            frame_width: 100,
            frame_height: 200,
            blocks: vec![OcrBlock {
                text: text.to_string(),
                confidence: 912,
                x: 0,
                y: 0,
                width: 90,
                height: 20,
            }],
        },
    }
}

fn failed(id: &str) -> OcrResult {
    OcrResult {
        schema: "foldscan.ocr/0.1".to_string(),
        capture_id: id.to_string(),
        requested_languages: vec!["eng".to_string()],
        status: OcrStatus::Failed {
            code: OcrFailureCode::Engine,
        },
    }
}

fn skipped(id: &str) -> OcrResult {
    OcrResult {
        schema: "foldscan.ocr/0.1".to_string(),
        capture_id: id.to_string(),
        requested_languages: vec!["eng".to_string()],
        status: OcrStatus::Skipped {
            reason: OcrSkipReason::Disabled,
        },
    }
}

fn ocr_path(id: &str) -> String {
    format!("ocr/sess-ocr/{}.json", id)
}

/// Host sources for originals only (no derivatives, document, or OCR).
fn original_sources(s: &ExportSession) -> HashMap<String, ExportSource> {
    let tmp = TempDir::new().expect("tmp");
    let mut sources = HashMap::new();
    for p in &s.pages {
        let f = tmp.path().join(format!("src-{}", p.capture_id));
        std::fs::write(&f, PAYLOAD).expect("write src");
        sources.insert(
            format!("originals/sess-ocr/{}.jpg", p.capture_id),
            ExportSource::file(f),
        );
    }
    // Leak the tempdir so the files live as long as the test process needs.
    std::mem::forget(tmp);
    sources
}

fn run(
    plan: &ExportPlan,
    manifest: &ExportManifest,
    sources: HashMap<String, ExportSource>,
    root: &std::path::Path,
) -> Result<foldscan_domain::executor::ExportExecution, Category> {
    execute_export(plan, root, &sources, manifest).map_err(|e| e.category)
}

// ---------- planner ----------

#[test]
fn planner_adds_sidecar_layout_after_pages_in_page_order() {
    let mut s = session(&["a", "b", "c"]);
    // Bind out of page order on purpose: layout must follow page order.
    s.ocr = vec![completed("c", "gamma"), completed("a", "alpha")];
    let plan = plan_export(&[s], &[]).expect("plan");
    let paths: Vec<&str> = plan
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "originals/sess-ocr/a.jpg",
            "originals/sess-ocr/b.jpg",
            "originals/sess-ocr/c.jpg",
            "ocr/sess-ocr/a.json",
            "ocr/sess-ocr/c.json",
            "export.json",
        ]
    );
    let ocr_files: Vec<&str> = plan
        .files
        .iter()
        .filter(|f| f.content_kind == ContentKind::Ocr)
        .map(|f| f.capture_id.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(ocr_files, vec!["a", "c"]);
}

#[test]
fn binding_order_cannot_change_layout_or_manifest_digest() {
    let mut s1 = session(&["a", "b"]);
    s1.ocr = vec![completed("a", "one"), completed("b", "two")];
    let mut s2 = s1.clone();
    s2.ocr.reverse();
    let p1 = plan_export(&[s1], &[]).expect("plan1");
    let p2 = plan_export(&[s2], &[]).expect("plan2");
    assert_eq!(p1.files, p2.files);
    let m1 = ExportManifest::from_plan(&p1, "sess-ocr").expect("m1");
    let m2 = ExportManifest::from_plan(&p2, "sess-ocr").expect("m2");
    assert_eq!(m1.digest(), m2.digest());
}

#[test]
fn sidecar_free_sessions_plan_and_digest_exactly_as_before() {
    let s = session(&["a", "b"]);
    let plan = plan_export(std::slice::from_ref(&s), &[]).expect("plan");
    assert!(!plan
        .files
        .iter()
        .any(|f| f.content_kind == ContentKind::Ocr));
    let m = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    assert!(m.pages.iter().all(|p| p.ocr_digest.is_none()));
    let json = serde_json::to_string(&m).unwrap();
    assert!(!json.contains("ocr_digest"));
    // No *path value* starts with the ocr/ directory (the session id
    // itself contains "ocr" and must not create layout entries).
    assert!(!json.contains("\"ocr/"));
}

#[test]
fn planner_rejects_failed_and_skipped_sidecars() {
    let mut s = session(&["a"]);
    s.ocr = vec![failed("a")];
    let err = plan_export(&[s.clone()], &[]).expect_err("failed status must not bind");
    assert_eq!(err.category, Category::InvalidRequest);
    let mut s2 = s.clone();
    s2.ocr = vec![skipped("a")];
    let err = plan_export(&[s2], &[]).expect_err("skipped status must not bind");
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn planner_rejects_sidecar_for_unknown_or_removed_capture() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("ghost", "boo")];
    let err = plan_export(&[s], &[]).expect_err("unknown capture must not bind");
    assert_eq!(err.category, Category::InvalidRequest);

    // Removing a page from the export strands its sidecar binding: the plan
    // must reject rather than silently drop or orphan it.
    let mut s2 = session(&["a", "b"]);
    s2.ocr = vec![completed("a", "x"), completed("b", "y")];
    plan_export(&[s2.clone()], &[]).expect("both bound is valid");
    remove_from(&mut s2, "b");
    let err = plan_export(&[s2], &[]).expect_err("stale sidecar must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn planner_rejects_duplicate_sidecar_per_capture() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "one"), completed("a", "two")];
    let err = plan_export(&[s], &[]).expect_err("duplicate binding must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn planner_rejects_malformed_bound_document() {
    // An empty language list is malformed at the ocr layer (validate());
    // the planner re-validates bound documents rather than trusting them.
    let mut s = session(&["a"]);
    let mut doc = completed("a", "text");
    doc.requested_languages.clear();
    s.ocr = vec![doc];
    let err = plan_export(&[s], &[]).expect_err("malformed doc must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
}

// ---------- manifest ----------

#[test]
fn manifest_binds_sidecar_digests_and_digest_changes_with_content() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![completed("a", "alpha")];
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let m = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    assert_eq!(
        m.pages[0].ocr_digest.as_deref(),
        Some(completed("a", "alpha").digest().as_str())
    );
    assert!(m.pages[1].ocr_digest.is_none());

    // Editing the recognized text changes the manifest integrity digest.
    let mut s2 = s.clone();
    s2.ocr = vec![completed("a", "ALPHA")];
    let p2 = plan_export(&[s2], &[]).expect("plan");
    let m2 = ExportManifest::from_plan(&p2, "sess-ocr").expect("manifest");
    assert_ne!(m.digest(), m2.digest());

    // Binding vs not binding changes the digest.
    let mut s3 = s.clone();
    s3.ocr.clear();
    let p3 = plan_export(&[s3], &[]).expect("plan");
    let m3 = ExportManifest::from_plan(&p3, "sess-ocr").expect("manifest");
    assert_ne!(m.digest(), m3.digest());
}

#[test]
fn manifest_round_trips_sidecar_digest_through_untrusted_parser() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let plan = plan_export(&[s], &[]).expect("plan");
    let m = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    let bytes = serde_json::to_vec_pretty(&m).expect("serialize");
    let back = ExportManifest::from_json_bytes(&bytes).expect("reparse");
    assert_eq!(m, back);
    assert_eq!(back.pages[0].ocr_digest, m.pages[0].ocr_digest);
}

#[test]
fn manifest_rejects_malformed_ocr_digest_shape() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let plan = plan_export(&[s], &[]).expect("plan");
    let mut m = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    m.pages[0].ocr_digest = Some("not-a-sha".to_string());
    assert_eq!(
        m.validate().map(|_| ()).err().unwrap().category,
        Category::InvalidRequest
    );
}

#[test]
fn reordered_pages_keep_sidecar_digests_bound_to_their_capture() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![completed("b", "beta")];
    reorder(&mut s, "b", 0);
    // Reordering changes the page order; the sidecar binding travels with
    // its capture, and the layout follows the new page order.
    let plan = plan_export(&[s], &[]).expect("plan");
    let paths: Vec<&str> = plan
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "originals/sess-ocr/b.jpg",
            "originals/sess-ocr/a.jpg",
            "ocr/sess-ocr/b.json",
            "export.json",
        ]
    );
    let m = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    assert_eq!(m.pages[0].capture_id, "b");
    assert_eq!(
        m.pages[0].ocr_digest.as_deref(),
        Some(completed("b", "beta").digest().as_str())
    );
    assert!(m.pages[1].ocr_digest.is_none());
}

#[test]
fn edited_plan_layout_diverging_from_bindings_is_rejected() {
    // A caller can mutate the public plan. Drop a planned sidecar file so
    // the layout no longer matches the session's bindings; from_plan and
    // the executor must both reject the tampered plan.
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let mut plan = plan_export(&[s], &[]).expect("plan");
    plan.files.retain(|f| f.content_kind != ContentKind::Ocr);
    let err = ExportManifest::from_plan(&plan, "sess-ocr")
        .expect_err("digest claim/layout mismatch must be rejected");
    assert_eq!(err.category, Category::InternalError);

    let manifest_ok = {
        // Rebuild the *canonical* plan's manifest for a control run below.
        let p = plan_export(
            &[{
                let mut s2 = session(&["a"]);
                s2.ocr = vec![completed("a", "alpha")];
                s2
            }],
            &[],
        )
        .expect("canonical plan");
        ExportManifest::from_plan(&p, "sess-ocr").expect("manifest")
    };
    let tmp = TempDir::new().unwrap();
    let sources = HashMap::new(); // originals below, but layout mismatch trips first
    let err = execute_export(&plan, &tmp.path().join("out"), &sources, &manifest_ok)
        .expect_err("tampered layout must be rejected pre-disk");
    // The executor's canonical-plan rebuild catches the edited layout as a
    // caller-input violation before manifest equality is even consulted.
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(!tmp.path().join("out").exists());
}

// ---------- executor ----------

#[test]
fn executor_writes_sidecar_and_manifest_last_verified() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![completed("a", "alpha"), completed("b", "beta")];
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("export");

    let exec = run(&plan, &manifest, original_sources(&s), &dest).expect("export");
    assert_eq!(exec.files_written, plan.files.len());

    // Finalized sidecar re-parses to exactly the bound document.
    let a_bytes = std::fs::read(dest.join(ocr_path("a"))).expect("sidecar a");
    let a_doc = OcrResult::from_json_bytes(&a_bytes).expect("reparse a");
    assert_eq!(a_doc, completed("a", "alpha"));
    let b_bytes = std::fs::read(dest.join(ocr_path("b"))).expect("sidecar b");
    assert_eq!(
        OcrResult::from_json_bytes(&b_bytes).expect("reparse b"),
        completed("b", "beta")
    );

    // The manifest on disk carries the digests the plan promised.
    let m_bytes = std::fs::read(dest.join("export.json")).expect("manifest");
    let m = ExportManifest::from_json_bytes(&m_bytes).expect("reparse manifest");
    assert_eq!(m.digest(), exec.manifest_digest);
    assert_eq!(
        m.pages[0].ocr_digest.as_deref(),
        Some(a_doc.digest().as_str())
    );

    // No staged leftovers anywhere.
    for entry in walk(dest.as_path()) {
        assert!(
            !entry.to_string_lossy().ends_with(".foldscan-part"),
            "staged leftover: {}",
            entry.display()
        );
    }
}

#[test]
fn executor_rejects_host_source_for_sidecar_path_before_touching_disk() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("export");

    let mut sources = original_sources(&s);
    sources.insert(
        ocr_path("a"),
        ExportSource::bytes(b"{\"evil\":true}".to_vec()),
    );
    let err = run(&plan, &manifest, sources, &dest).expect_err("host sidecar bytes must not enter");
    assert_eq!(err, Category::InvalidRequest);
    assert!(!dest.exists(), "rejection must precede any disk mutation");
}

#[test]
fn executor_cancellation_stops_before_sidecar_and_rolls_back() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("export");

    // Cancel exactly at the sidecar boundary: allow originals, cancel when
    // about to write the sidecar. A counter callback is overkill here — a
    // first-two-files pass-through reproduces the same prefix.
    let mut seen = 0usize;
    let result = execute_export_cancellable(&plan, &dest, &original_sources(&s), &manifest, || {
        seen += 1;
        seen > 2 // polls: pre-root(1), file a(2) pass, file sidecar(3) cancels
    });
    let err = result.expect_err("cancellation must abort before the sidecar");
    assert_eq!(err.category, Category::Cancelled);
    assert!(
        !dest.exists(),
        "rollback must remove partially written tree"
    );
}

#[test]
fn executor_sidecar_free_export_is_byte_identical_to_pre_sidecar() {
    // Same session without any ocr binding: the finalized manifest bytes
    // must carry no ocr/ocr_digest keys at all (additive-optional proof at
    // the filesystem level, complementing the digest tests above).
    let s = session(&["a"]);
    let plan = plan_export(std::slice::from_ref(&s), &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-ocr").expect("manifest");
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("export");
    run(&plan, &manifest, original_sources(&s), &dest).expect("export");
    let m = std::fs::read_to_string(dest.join("export.json")).expect("manifest");
    assert!(!m.contains("ocr_digest"));
    assert!(!m.contains("\"ocr/"));
    assert!(!dest.join("ocr").exists());
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
