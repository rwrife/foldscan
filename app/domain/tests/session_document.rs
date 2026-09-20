//! Session-level document binding: planner layout, manifest coverage, and
//! executor finalize/rollback for one assembled PDF per export session.
//!
//! Evidence category: software fixture test on synthetic data (real
//! `pdf::export_pdf` output over synthetic `GrayFrame`s) in a temporary
//! directory. No physical device, no PDF parser round-trip beyond what
//! `src/pdf.rs` already self-verifies.

use std::collections::HashMap;

use foldscan_domain::checksum::sha256_hex;
use foldscan_domain::error::Category;
use foldscan_domain::executor::{execute_export, execute_export_cancellable, ExportSource};
use foldscan_domain::{
    plan_export, reorder, ContentKind, DocumentPage, ExportManifest, ExportPage, ExportPlan,
    ExportSession, GrayFrame, SessionDocument,
};
use tempfile::TempDir;

fn page(id: &str) -> ExportPage {
    ExportPage {
        capture_id: id.to_string(),
        media_type: "image/jpeg".to_string(),
        processed_media_type: None,
        original_sha256: sha256_hex(id.as_bytes()),
        original_bytes: 100,
        processed_sha256: None,
        processed_bytes: None,
        recipe_digest: None,
    }
}

fn session(ids: &[&str]) -> ExportSession {
    ExportSession {
        session_id: "sess-doc".to_string(),
        pages: ids.iter().map(|id| page(id)).collect(),
        document: None,
    }
}

fn doc_for(s: &ExportSession, bytes: u64) -> SessionDocument {
    SessionDocument {
        media_type: "application/pdf".to_string(),
        sha256: sha256_hex(b"session-pdf-bytes"),
        bytes,
        pages: s
            .pages
            .iter()
            .map(|p| DocumentPage {
                capture_id: p.capture_id.clone(),
                width_px: 8,
                height_px: 6,
            })
            .collect(),
    }
}

/// Real PDF bytes over synthetic frames matching the declared page order.
fn real_pdf(ids: &[(&str, u32, u32)]) -> Vec<u8> {
    let frames: Vec<GrayFrame> = ids
        .iter()
        .map(|(_, w, h)| GrayFrame::new(*w, *h, 200).expect("frame"))
        .collect();
    foldscan_domain::export_pdf(&frames).expect("pdf")
}

const DOC_PATH: &str = "documents/sess-doc/session.pdf";

// ---------- planner ----------

#[test]
fn planner_adds_document_layout_after_pages_before_recipes() {
    let mut s = session(&["a", "b"]);
    let pdf = real_pdf(&[("a", 4, 3), ("b", 4, 3)]);
    s.document = Some(doc_for(&s, pdf.len() as u64));
    let plan = plan_export(&[s], &[]).expect("plan");
    let paths: Vec<&str> = plan
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "originals/sess-doc/a.jpg",
            "originals/sess-doc/b.jpg",
            DOC_PATH,
            "export.json",
        ]
    );
    let doc_file = plan
        .files
        .iter()
        .find(|f| f.content_kind == ContentKind::Document)
        .expect("document planned");
    assert!(doc_file.capture_id.is_none());
}

#[test]
fn undocumented_sessions_plan_exactly_as_before() {
    let s = session(&["a", "b"]);
    let plan = plan_export(std::slice::from_ref(&s), &[]).expect("plan");
    assert!(!plan
        .files
        .iter()
        .any(|f| f.content_kind == ContentKind::Document));
    let m = ExportManifest::from_plan(&plan, "sess-doc").expect("manifest");
    assert!(m.document.is_none());
    // Absent document serializes as null/omitted and digests like before.
    let json = serde_json::to_string(&m).unwrap();
    assert!(!json.contains("documents/"));
}

#[test]
fn planner_rejects_order_mismatched_bindings() {
    let s = session(&["a", "b", "c"]);
    let mut doc = doc_for(&s, 10);
    doc.pages.swap(0, 1);
    let mut bad = s.clone();
    bad.document = Some(doc);
    let err = plan_export(&[bad], &[]).expect_err("swap must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn planner_rejects_binding_list_that_drops_or_adds_captures() {
    let s = session(&["a", "b"]);
    let mut short = doc_for(&s, 10);
    short.pages.pop();
    let mut sess = s.clone();
    sess.document = Some(short);
    assert!(plan_export(&[sess], &[]).is_err());

    let mut long = doc_for(&s, 10);
    long.pages.push(DocumentPage {
        capture_id: "ghost".into(),
        width_px: 1,
        height_px: 1,
    });
    let mut sess = s.clone();
    sess.document = Some(long);
    assert!(plan_export(&[sess], &[]).is_err());
}

#[test]
fn planner_rejects_empty_binding_list() {
    let s = session(&["a"]);
    let mut doc = doc_for(&s, 10);
    doc.pages.clear();
    let mut sess = s.clone();
    sess.document = Some(doc);
    assert!(plan_export(&[sess], &[]).is_err());
}

#[test]
fn planner_rejects_unsupported_media_type_and_bad_shapes() {
    let s = session(&["a"]);
    for mutate in [
        (|d: &mut SessionDocument| d.media_type = "application/zip".into()) as fn(&mut _),
        |d: &mut SessionDocument| d.sha256 = "ZZ".into(),
        |d: &mut SessionDocument| d.bytes = u64::MAX,
        |d: &mut SessionDocument| d.pages[0].width_px = 0,
        |d: &mut SessionDocument| d.pages[0].height_px = 40_000,
    ] {
        let mut doc = doc_for(&s, 10);
        mutate(&mut doc);
        let mut sess = s.clone();
        sess.document = Some(doc);
        let err = plan_export(&[sess], &[]).expect_err("invalid document must be rejected");
        assert_eq!(err.category, Category::InvalidRequest);
    }
}

// ---------- manifest ----------

#[test]
fn manifest_digest_covers_document_order_and_content() {
    let mut s = session(&["a", "b"]);
    s.document = Some(doc_for(&s, 1234));
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let base = ExportManifest::from_plan(&plan, "sess-doc").expect("manifest");
    assert!(base.document.is_some());
    assert_eq!(base.document.as_ref().unwrap().path, DOC_PATH);

    // A changed byte count changes the digest.
    let mut s2 = s.clone();
    s2.document.as_mut().unwrap().bytes = 4321;
    let plan2 = plan_export(&[s2], &[]).expect("plan");
    let m2 = ExportManifest::from_plan(&plan2, "sess-doc").expect("manifest");
    assert_ne!(base.digest(), m2.digest());

    // A reordered session (document rebound to the new order) digests
    // differently, proving order sensitivity end to end through planning.
    let mut s3 = session(&["a", "b"]);
    assert!(reorder(&mut s3, "b", 0));
    s3.document = Some(doc_for(&s3, 1234));
    let plan3 = plan_export(&[s3], &[]).expect("plan");
    let m3 = ExportManifest::from_plan(&plan3, "sess-doc").expect("manifest");
    assert_ne!(base.digest(), m3.digest());
    assert_eq!(m3.document.as_ref().unwrap().pages[0].capture_id, "b");
}

#[test]
fn manifest_round_trips_document_and_rejects_tampered_order() {
    let mut s = session(&["a", "b"]);
    s.document = Some(doc_for(&s, 1234));
    let plan = plan_export(&[s], &[]).expect("plan");
    let m = ExportManifest::from_plan(&plan, "sess-doc").expect("manifest");
    let bytes = serde_json::to_vec(&m).expect("serialize");
    let back = ExportManifest::from_json_bytes(&bytes).expect("round trip");
    assert_eq!(m, back);
    assert_eq!(m.digest(), back.digest());

    // Tamper: swap the document binding order while keeping page order.
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let pages = value
        .get_mut("document")
        .and_then(|d| d.get_mut("pages"))
        .expect("document pages present");
    let arr = pages.as_array_mut().unwrap();
    arr.swap(0, 1);
    let tampered = serde_json::to_vec(&value).unwrap();
    let err = ExportManifest::from_json_bytes(&tampered)
        .expect_err("binding order must equal manifest page order");
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn manifest_from_json_defaults_missing_document_to_none() {
    // Older manifests (no document key) must keep parsing and digesting.
    let s = session(&["a"]);
    let plan = plan_export(&[s], &[]).expect("plan");
    let m = ExportManifest::from_plan(&plan, "sess-doc").expect("manifest");
    let mut value: serde_json::Value = serde_json::to_value(&m).unwrap();
    value.as_object_mut().unwrap().remove("document");
    let bytes = serde_json::to_vec(&value).unwrap();
    let back = ExportManifest::from_json_bytes(&bytes).expect("legacy parse");
    assert!(back.document.is_none());
    assert_eq!(back.digest(), m.digest());
}

// ---------- executor ----------

/// A session whose page digests match `vec![0xAB; 100]` originals, plus a
/// document binding over real `export_pdf` bytes.
fn documented_fixture() -> (ExportPlan, ExportManifest, Vec<u8>) {
    const PAYLOAD: &[u8] = &[0xAB; 100];
    let mut s = ExportSession {
        session_id: "sess-doc".to_string(),
        pages: ["a", "b"]
            .iter()
            .map(|id| ExportPage {
                capture_id: id.to_string(),
                media_type: "image/jpeg".to_string(),
                processed_media_type: None,
                original_sha256: sha256_hex(PAYLOAD),
                original_bytes: PAYLOAD.len() as u64,
                processed_sha256: None,
                processed_bytes: None,
                recipe_digest: None,
            })
            .collect(),
        document: None,
    };
    let pdf = real_pdf(&[("a", 4, 3), ("b", 4, 3)]);
    s.document = Some(SessionDocument {
        media_type: "application/pdf".to_string(),
        sha256: sha256_hex(&pdf),
        bytes: pdf.len() as u64,
        pages: vec![
            DocumentPage {
                capture_id: "a".into(),
                width_px: 4,
                height_px: 3,
            },
            DocumentPage {
                capture_id: "b".into(),
                width_px: 4,
                height_px: 3,
            },
        ],
    });
    let plan = plan_export(&[s], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-doc").expect("manifest");
    (plan, manifest, pdf)
}

fn originals_sources(plan: &ExportPlan, host: &std::path::Path) -> HashMap<String, ExportSource> {
    let mut map = HashMap::new();
    let dir = host.join("src-originals");
    std::fs::create_dir_all(&dir).unwrap();
    for f in plan
        .files
        .iter()
        .filter(|f| f.content_kind == ContentKind::Original)
    {
        let src = dir.join(f.capture_id.as_deref().unwrap());
        std::fs::write(&src, vec![0xAB; 100]).unwrap();
        map.insert(f.relative_path.clone(), ExportSource::file(&src));
    }
    map
}

#[test]
fn executor_writes_document_with_in_memory_bytes_and_counts_it() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, pdf) = documented_fixture();
    let mut sources = originals_sources(&plan, host.path());
    sources.insert(DOC_PATH.to_string(), ExportSource::bytes(pdf.clone()));

    let dest = host.path().join("out/export");
    let exec = execute_export(&plan, &dest, &sources, &manifest).expect("export");
    // originals(2) + document(1) + manifest(1)
    assert_eq!(exec.files_written, 4);
    let written = std::fs::read(dest.join(DOC_PATH)).expect("document on disk");
    assert_eq!(written, pdf);
    // The finalized PDF keeps its own structure: the export copy re-hashes
    // to the exact checksum the manifest recorded.
    assert_eq!(
        sha256_hex(&written),
        manifest.document.as_ref().unwrap().sha256
    );
    // Page order proof: manifest binding order matches session page order,
    // and the exported document bytes embed exactly those page dimensions.
    let ids: Vec<&str> = manifest
        .document
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .map(|p| p.capture_id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
}

#[test]
fn executor_rejects_missing_document_source_before_touching_disk() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, _pdf) = documented_fixture();
    let sources = originals_sources(&plan, host.path()); // no document source
    let dest = host.path().join("out2/export");
    let err = execute_export(&plan, &dest, &sources, &manifest)
        .expect_err("missing document source must fail preflight");
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(!host.path().join("out2").exists(), "no parent dirs created");
}

#[test]
fn executor_rejects_document_checksum_mismatch_with_full_rollback() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, pdf) = documented_fixture();
    let mut sources = originals_sources(&plan, host.path());
    let mut corrupted = pdf.clone();
    corrupted.push(0xFF); // same content plus extra byte -> checksum+size mismatch
    sources.insert(DOC_PATH.to_string(), ExportSource::bytes(corrupted));
    let dest = host.path().join("out3/export");
    let err = execute_export(&plan, &dest, &sources, &manifest)
        .expect_err("mismatched document must fail");
    assert_eq!(err.category, Category::ChecksumMismatch);
    assert!(!dest.exists(), "rollback removed the whole export tree");
}

#[test]
fn executor_rejects_unplanned_document_source() {
    let host = TempDir::new().unwrap();
    // Pages-only session: plan has no document file.
    let s = session(&["a"]);
    let plan = plan_export(&[s], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-doc").expect("manifest");
    let mut sources = originals_sources(&plan, host.path());
    sources.insert(
        DOC_PATH.to_string(),
        ExportSource::bytes(b"smuggled".to_vec()),
    );
    let dest = host.path().join("out4/export");
    let err = execute_export(&plan, &dest, &sources, &manifest)
        .expect_err("unplanned document source must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(!host.path().join("out4").exists());
}

#[test]
fn cancellation_before_document_file_rolls_back() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let host = TempDir::new().unwrap();
    let (plan, manifest, pdf) = documented_fixture();
    let mut sources = originals_sources(&plan, host.path());
    sources.insert(DOC_PATH.to_string(), ExportSource::bytes(pdf));

    // Files: orig a, orig b, document, manifest. Cancel on the 3rd poll
    // (before the document is staged).
    let polls = AtomicUsize::new(0);
    let dest = host.path().join("out5/export");
    let err = execute_export_cancellable(&plan, &dest, &sources, &manifest, || {
        polls.fetch_add(1, Ordering::SeqCst) >= 3
    })
    .expect_err("cancellation must abort before the document");
    assert_eq!(err.category, Category::Cancelled);
    assert!(
        !dest.exists(),
        "rollback removed partially written originals"
    );
}

#[test]
fn import_sessions_have_no_document_by_default() {
    // export_sessions_from_import constructs document: None — pinned so the
    // additive field never silently changes import behavior.
    let s = session(&["a", "b"]);
    let plan = plan_export(&[s], &[]).unwrap();
    assert!(plan.sessions.iter().all(|x| x.document.is_none()));
}

#[test]
fn reordered_session_rejects_stale_document_binding_end_to_end() {
    // Reordering the session without rebuilding the document binding is the
    // classic stale-metadata bug; planning must reject it, not export a PDF
    // whose page order silently differs from the manifest's.
    let mut s = session(&["a", "b"]);
    s.document = Some(doc_for(&s, 1234));
    assert!(reorder(&mut s, "b", 0));
    let err = plan_export(&[s], &[]).expect_err("stale binding must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
}
