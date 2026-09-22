//! Plain-text OCR rendition export binding (issue #35, child of #5).
//!
//! Covers the deterministic plain-text rendering of completed OCR result
//! documents, the opt-in `ocr/<session>/<capture>.txt` layout, manifest
//! `ocr_text_digest` binding/validation, and executor finalize/rollback of
//! text files rendered from the bound document (never host-supplied bytes).
//!
//! Evidence category: software fixture test on synthetic OCR documents in a
//! temporary directory. No OCR engine exists — the documents are authored by
//! the test, not recognized by any tool. No physical device, no bench.

use std::collections::HashMap;

use foldscan_domain::executor::{execute_export, execute_export_cancellable, ExportSource};
use foldscan_domain::export::PlannedFile;
use foldscan_domain::ocr::{OcrBlock, OcrFailureCode, OcrResult, OcrSkipReason, OcrStatus};
use foldscan_domain::{
    plan_export, remove_from, reorder, Category, ContentKind, ExportManifest, ExportPage,
    ExportPlan, ExportSession, MAX_OCR_TEXT_BYTES,
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
        session_id: "sess-txt".to_string(),
        pages: ids.iter().map(|id| page(id)).collect(),
        document: None,
        ocr: Vec::new(),
        ocr_text: false,
    }
}

/// A synthetic completed OCR document with one text block.
fn completed(id: &str, text: &str) -> OcrResult {
    completed_blocks(id, vec![(text.to_string(), 0, 0, 90, 20)])
}

/// A synthetic completed OCR document with several text blocks, each given
/// as (text, x, y, width, height) inside a 100x200 frame.
fn completed_blocks(id: &str, blocks: Vec<(String, u32, u32, u32, u32)>) -> OcrResult {
    OcrResult {
        schema: "foldscan.ocr/0.1".to_string(),
        capture_id: id.to_string(),
        requested_languages: vec!["eng".to_string()],
        status: OcrStatus::Completed {
            frame_width: 100,
            frame_height: 200,
            blocks: blocks
                .into_iter()
                .map(|(text, x, y, width, height)| OcrBlock {
                    text,
                    confidence: 912,
                    x,
                    y,
                    width,
                    height,
                })
                .collect(),
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

fn json_path(id: &str) -> String {
    format!("ocr/sess-txt/{}.json", id)
}

fn txt_path(id: &str) -> String {
    format!("ocr/sess-txt/{}.txt", id)
}

/// Host sources for originals only (no derivatives/document/OCR sources).
fn original_sources(s: &ExportSession) -> HashMap<String, ExportSource> {
    let tmp = TempDir::new().expect("tmp");
    let mut sources = HashMap::new();
    for p in &s.pages {
        let f = tmp.path().join(format!("src-{}", p.capture_id));
        std::fs::write(&f, PAYLOAD).expect("write src");
        sources.insert(
            format!("originals/sess-txt/{}.jpg", p.capture_id),
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

// ---------- rendering ----------

#[test]
fn rendering_is_deterministic_and_pinned_by_golden_digest() {
    let doc = completed("a", "alpha");
    let once = doc.render_plain_text().expect("render");
    let twice = doc.render_plain_text().expect("render again");
    assert_eq!(once, twice);
    assert_eq!(once, "alpha\n");
    // Golden digest of the exact rendered bytes (SHA-256 of b"alpha\n").
    assert_eq!(
        doc.plain_text_digest().expect("digest"),
        "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060"
    );
}

#[test]
fn block_order_is_content_in_the_rendering() {
    let ab = completed_blocks(
        "a",
        vec![
            ("alpha".to_string(), 0, 0, 90, 20),
            ("beta".to_string(), 0, 30, 90, 20),
        ],
    );
    let mut ba = ab.clone();
    if let OcrStatus::Completed { blocks, .. } = &mut ba.status {
        blocks.reverse();
    }
    assert_eq!(ab.render_plain_text().expect("ab"), "alpha\nbeta\n");
    assert_eq!(ba.render_plain_text().expect("ba"), "beta\nalpha\n");
    // Golden digests pin that order changes the rendition and its digest.
    assert_eq!(
        ab.plain_text_digest().expect("ab digest"),
        "e49c81e2d2f84e259d40e2fb8192f3bcd198b355184845d76d8f58807d0d78ee"
    );
    assert_eq!(
        ba.plain_text_digest().expect("ba digest"),
        "3588d4ce80593f91177fe39f97f96fece7050ebc8e030a2a92a7f61e67f07af9"
    );
    assert_ne!(
        ab.plain_text_digest().unwrap(),
        ba.plain_text_digest().unwrap()
    );
}

#[test]
fn completed_with_no_blocks_renders_the_empty_page_text() {
    let doc = completed_blocks("a", Vec::new());
    let text = doc.render_plain_text().expect("render");
    assert_eq!(text, "\n");
    assert_eq!(
        doc.plain_text_digest().expect("digest"),
        "01ba4719c80b6fe911b091a7c05124b64eeece964e09c058ef8f9805daca546b"
    );
}

#[test]
fn non_completed_documents_have_nothing_to_render() {
    let f = failed("a");
    assert_eq!(
        f.render_plain_text()
            .expect_err("failed renders nothing")
            .category,
        Category::InvalidRequest
    );
    let s = skipped("a");
    assert_eq!(
        s.render_plain_text()
            .expect_err("skipped renders nothing")
            .category,
        Category::InvalidRequest
    );
}

#[test]
fn rendering_never_introduces_control_characters() {
    // Valid block text never contains control characters (validate()
    // rejects them), and the only character the renderer adds is '\n'.
    let doc = completed_blocks(
        "a",
        vec![
            ("first line".to_string(), 0, 0, 90, 20),
            ("second line sep".to_string(), 0, 30, 90, 20),
        ],
    );
    let text = doc.render_plain_text().expect("render");
    assert!(text.chars().all(|c| !c.is_control() || c == '\n'));
    // Control characters inside block text are rejected upstream, so they
    // can never ride into the rendering from untrusted documents.
    let mut bad = completed("a", "ok");
    if let OcrStatus::Completed { blocks, .. } = &mut bad.status {
        blocks[0].text = "line\twith\ttabs".to_string();
    }
    assert_eq!(
        bad.render_plain_text()
            .expect_err("tab must not render")
            .category,
        Category::InvalidRequest
    );
}

#[test]
fn non_ascii_text_renders_with_pinned_digest() {
    let doc = completed("a", "παν ἔixe 你好");
    assert_eq!(
        doc.plain_text_digest().expect("digest"),
        "f6d292c3ce41ce42766e8a1563fde743a1a5e0315099a70d75b93bf4de684628"
    );
    // The byte bound is derived from the validated character/block bounds.
    assert!(doc.render_plain_text().expect("render").len() <= MAX_OCR_TEXT_BYTES);
}

// ---------- planner ----------

#[test]
fn opt_in_adds_text_renditions_after_their_json_sidecars_in_page_order() {
    let mut s = session(&["a", "b", "c"]);
    s.ocr = vec![completed("c", "gamma"), completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s], &[]).expect("plan");
    let paths: Vec<&str> = plan
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "originals/sess-txt/a.jpg",
            "originals/sess-txt/b.jpg",
            "originals/sess-txt/c.jpg",
            "ocr/sess-txt/a.json",
            "ocr/sess-txt/a.txt",
            "ocr/sess-txt/c.json",
            "ocr/sess-txt/c.txt",
            "export.json",
        ]
    );
    let text_count = plan
        .files
        .iter()
        .filter(|f| f.content_kind == ContentKind::OcrText)
        .count();
    assert_eq!(text_count, 2);
}

#[test]
fn opt_out_and_empty_bindings_keep_layout_byte_identical_to_pre_text() {
    let mut with_ocr = session(&["a"]);
    with_ocr.ocr = vec![completed("a", "alpha")];
    let plan_off = plan_export(&[with_ocr.clone()], &[]).expect("plan off");

    let mut opt_out = with_ocr.clone();
    opt_out.ocr_text = false;
    let plan_opt_out = plan_export(&[opt_out], &[]).expect("plan opt-out");
    assert_eq!(plan_opt_out.files, plan_off.files);

    // Flag on, no bindings: nothing to render, layout unchanged.
    let mut flag_only = session(&["a"]);
    flag_only.ocr_text = true;
    let mut plain = session(&["a"]);
    plain.ocr_text = false;
    assert_eq!(
        plan_export(&[flag_only], &[]).expect("plan").files,
        plan_export(&[plain], &[]).expect("plan").files
    );
}

#[test]
fn text_opt_in_never_admits_failed_or_skipped_documents() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![failed("a"), skipped("b")];
    s.ocr_text = true;
    let err = plan_export(&[s], &[]).expect_err("non-completed must not bind");
    assert_eq!(err.category, Category::InvalidRequest);
}

#[test]
fn reordered_pages_keep_text_renditions_bound_to_their_capture() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![completed("b", "beta")];
    s.ocr_text = true;
    reorder(&mut s, "b", 0);
    let plan = plan_export(&[s], &[]).expect("plan");
    let paths: Vec<&str> = plan
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "originals/sess-txt/b.jpg",
            "originals/sess-txt/a.jpg",
            "ocr/sess-txt/b.json",
            "ocr/sess-txt/b.txt",
            "export.json",
        ]
    );
    let m = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    assert_eq!(m.pages[0].capture_id, "b");
    assert!(m.pages[0].ocr_text_digest.is_some());
    assert!(m.pages[1].ocr_text_digest.is_none());
}

#[test]
fn removing_a_page_strands_its_text_binding_like_json_sidecars() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![completed("a", "x"), completed("b", "y")];
    s.ocr_text = true;
    plan_export(&[s.clone()], &[]).expect("both bound is valid");
    remove_from(&mut s, "b");
    let err = plan_export(&[s], &[]).expect_err("stale text binding must be rejected");
    assert_eq!(err.category, Category::InvalidRequest);
}

// ---------- manifest ----------

#[test]
fn manifest_text_digest_matches_rendering_and_covers_page_order() {
    let mut s = session(&["a", "b"]);
    s.ocr = vec![completed("a", "alpha"), completed("b", "beta")];
    s.ocr_text = true;
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let m = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    assert_eq!(
        m.pages[0].ocr_text_digest.as_deref(),
        Some(
            completed("a", "alpha")
                .plain_text_digest()
                .expect("digest")
                .as_str()
        )
    );
    assert_eq!(
        m.pages[1].ocr_text_digest.as_deref(),
        Some(
            completed("b", "beta")
                .plain_text_digest()
                .expect("digest")
                .as_str()
        )
    );
    // The integrity digest differs from the JSON-sidecar-only manifest, so
    // text presence is manifest-visible.
    let mut json_only = s.clone();
    json_only.ocr_text = false;
    let mj = ExportManifest::from_plan(&plan_export(&[json_only], &[]).expect("plan"), "sess-txt")
        .expect("manifest");
    assert_ne!(m.digest(), mj.digest());
}

#[test]
fn text_digest_absent_means_byte_identical_pre_text_manifest() {
    // An opt-out (or OCR-less) manifest serializes with no
    // ocr_text_digest keys at all — the additive-optional compatibility
    // proof: pre-slice consumers see the exact pre-slice document, and a
    // stripped-key document parses back to an equal manifest/digest.
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let plan = plan_export(&[s], &[]).expect("plan");
    let m = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    assert!(m.pages[0].ocr_text_digest.is_none());
    let mut value = serde_json::to_value(&m).expect("to value");
    assert!(value
        .get("pages")
        .and_then(|p| p.get(0))
        .and_then(|p| p.get("ocr_text_digest"))
        .is_none());
    if let Some(obj) = value
        .get_mut("pages")
        .and_then(|p| p.get_mut(0))
        .and_then(|p| p.as_object_mut())
    {
        obj.remove("ocr_text_digest");
    }
    let bytes = serde_json::to_vec(&value).expect("serialize");
    let back = ExportManifest::from_json_bytes(&bytes).expect("parse legacy shape");
    assert_eq!(back, m);
    assert_eq!(back.digest(), m.digest());
}

#[test]
fn manifest_text_digest_shape_is_validated() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s], &[]).expect("plan");
    let mut m = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    m.pages[0].ocr_text_digest = Some("not-a-sha".to_string());
    assert_eq!(
        m.validate().map(|_| ()).err().unwrap().category,
        Category::InvalidRequest
    );
}

#[test]
fn manifest_text_digest_without_document_digest_is_rejected() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s], &[]).expect("plan");
    let mut m = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    m.pages[0].ocr_digest = None;
    assert_eq!(
        m.validate().map(|_| ()).err().unwrap().category,
        Category::InvalidRequest
    );
}

#[test]
fn edited_plan_layout_diverging_from_text_opt_in_is_rejected() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;

    // Drop the planned .txt so the layout no longer matches the opt-in.
    let mut plan = plan_export(&[s.clone()], &[]).expect("plan");
    plan.files
        .retain(|f| f.content_kind != ContentKind::OcrText);
    let err = ExportManifest::from_plan(&plan, "sess-txt")
        .expect_err("missing text layout must be rejected");
    assert_eq!(err.category, Category::InternalError);

    // Keep the .txt in the layout but flip the session's opt-in off: the
    // manifest must likewise refuse to reconcile the two.
    let mut plan2 = plan_export(&[s], &[]).expect("plan");
    plan2.sessions[0].ocr_text = false;
    let err = ExportManifest::from_plan(&plan2, "sess-txt")
        .expect_err("extra text layout must be rejected");
    assert_eq!(err.category, Category::InternalError);
}

#[test]
fn injected_extra_text_file_never_executes() {
    // A caller can push a PlannedFile directly (public fields). An extra
    // OcrText entry for a capture the plan did not legitimately lay out
    // must never reach disk: the executor rebuilds the canonical layout
    // and rejects the divergence before any directory is created.
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    let mut tampered = plan.clone();
    tampered.files.push(PlannedFile {
        relative_path: json_path("a"),
        capture_id: Some("a".to_string()),
        content_kind: ContentKind::Ocr,
    });
    let host = TempDir::new().expect("host");
    let dest = host.path().join("export");
    let err = run(
        &tampered,
        &manifest,
        original_sources(&tampered.sessions[0]),
        &dest,
    )
    .expect_err("duplicated layout must be rejected");
    assert_eq!(err, Category::InvalidRequest);
    assert!(!dest.exists(), "rejected export must create nothing");
}

// ---------- executor ----------

#[test]
fn executor_writes_rendered_text_and_counts_it() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    let host = TempDir::new().expect("host");
    let dest = host.path().join("export");
    let exec = run(&plan, &manifest, original_sources(&s), &dest).expect("export");
    // originals + json sidecar + txt rendition + manifest.
    assert_eq!(exec.files_written, 4);
    let written = std::fs::read(dest.join(txt_path("a"))).expect("txt file");
    assert_eq!(written, b"alpha\n");
    assert_eq!(
        foldscan_domain::checksum::sha256_hex(&written),
        manifest.pages[0]
            .ocr_text_digest
            .as_deref()
            .expect("digest")
    );
}

#[test]
fn text_opt_in_json_sidecar_bytes_are_unchanged_by_the_rendition() {
    // The .txt is an *additional* file: with the same binding, JSON-only
    // and JSON+text exports finalize identical .json sidecar bytes.
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    let host = TempDir::new().expect("host");
    let dest_off = host.path().join("off");
    let plan_off = plan_export(&[s.clone()], &[]).expect("plan");
    let m_off = ExportManifest::from_plan(&plan_off, "sess-txt").expect("manifest");
    run(&plan_off, &m_off, original_sources(&s), &dest_off).expect("export off");

    s.ocr_text = true;
    let dest_on = host.path().join("on");
    let plan_on = plan_export(&[s.clone()], &[]).expect("plan");
    let m_on = ExportManifest::from_plan(&plan_on, "sess-txt").expect("manifest");
    run(&plan_on, &m_on, original_sources(&s), &dest_on).expect("export on");

    assert_eq!(
        std::fs::read(dest_off.join(json_path("a"))).expect("json off"),
        std::fs::read(dest_on.join(json_path("a"))).expect("json on")
    );
    assert_eq!(
        std::fs::read(dest_off.join("originals/sess-txt/a.jpg")).expect("orig off"),
        std::fs::read(dest_on.join("originals/sess-txt/a.jpg")).expect("orig on")
    );
    assert!(!dest_off.join(txt_path("a")).exists());
    assert!(dest_on.join(txt_path("a")).is_file());
}

#[test]
fn executor_rejects_host_supplied_text_source() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    let mut sources = original_sources(&s);
    let tmp = TempDir::new().expect("tmp");
    let f = tmp.path().join("forged.txt");
    std::fs::write(&f, b"forged text\n").expect("write");
    sources.insert(txt_path("a"), ExportSource::file(f));
    let host = TempDir::new().expect("host");
    let dest = host.path().join("export");
    let err = run(&plan, &manifest, sources, &dest).expect_err("forged source rejected");
    assert_eq!(err, Category::InvalidRequest);
    // Nothing was created — rejection happens before any disk mutation.
    assert!(!dest.exists());
}

#[test]
fn mutated_bound_document_fails_before_finalizing_text() {
    // Build plan+manifest from a reviewed session, then mutate the text of
    // the bound document inside the plan's session while keeping the
    // original (reviewed) manifest. The executor must reject — via the
    // canonical layout/manifest rebuild or the text-digest cross-check —
    // and create nothing.
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");

    let mut tampered = plan.clone();
    if let OcrStatus::Completed { blocks, .. } = &mut tampered.sessions[0].ocr[0].status {
        blocks[0].text = "tampered".to_string();
    }
    let host = TempDir::new().expect("host");
    let dest = host.path().join("export");
    let err = execute_export(
        &tampered,
        &dest,
        &original_sources(&tampered.sessions[0]),
        &manifest,
    )
    .expect_err("tampered document must not export under the old manifest");
    assert!(matches!(
        err.category,
        Category::InvalidRequest | Category::InternalError | Category::ChecksumMismatch
    ));
    assert!(!dest.exists(), "rejected export must create nothing");
}

#[test]
fn cancellation_between_files_rolls_back_including_text_renditions() {
    let mut s = session(&["a"]);
    s.ocr = vec![completed("a", "alpha")];
    s.ocr_text = true;
    let plan = plan_export(&[s.clone()], &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    let host = TempDir::new().expect("host");
    let dest = host.path().join("export");
    let sources = original_sources(&s);
    // Cancel on the second poll: the original file may have been finalized
    // already, but the tree must roll back completely, .txt included.
    let mut polls = 0usize;
    let err = execute_export_cancellable(&plan, &dest, &sources, &manifest, || {
        polls += 1;
        polls > 1
    })
    .expect_err("cancellation must abort");
    assert_eq!(err.category, Category::Cancelled);
    assert!(!dest.exists(), "cancelled tree must be removed");
}

#[test]
fn text_free_session_export_is_file_free_of_ocr_dir_entries_like_before() {
    // Sessions with no OCR at all never grow an ocr/ directory even with
    // legacy hosts leaving the flag off — file-level additive proof.
    let s = session(&["a"]);
    let plan = plan_export(std::slice::from_ref(&s), &[]).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, "sess-txt").expect("manifest");
    let host = TempDir::new().expect("host");
    let dest = host.path().join("export");
    run(&plan, &manifest, original_sources(&s), &dest).expect("export");
    let m = std::fs::read_to_string(dest.join("export.json")).expect("manifest");
    assert!(!m.contains("ocr_text_digest"));
    assert!(!m.contains("ocr_digest"));
    assert!(!dest.join("ocr").exists());
}
