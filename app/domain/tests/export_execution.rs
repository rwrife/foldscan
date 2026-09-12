//! End-to-end fixture for the filesystem export executor: import a synthetic
//! volume, review pages, bind a recipe, plan the layout, then materialize the
//! export on disk and verify finalize/rollback/no-overwrite behavior — with
//! originals file-backed and derivatives arriving either as host files or as
//! in-memory bytes straight from the processing pipeline.
//!
//! Evidence category: software fixture test on synthetic data in a temporary
//! directory. No physical device, no image encoding, no PDF writing.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use foldscan_domain::checksum::{is_lowercase_hex_sha256, sha256_hex};
use foldscan_domain::error::Category;
use foldscan_domain::executor::{execute_export, ExportExecution, ExportSource, PART_SUFFIX};
use foldscan_domain::{
    export_sessions_from_import, import_volume, plan_export, remove_from, reorder,
    DeterministicProcessor, ExportManifest, ExportPage, GrayFrame, OpKind, ProcessingRecipe,
    Processor, RecipeOp,
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

/// Import, review (reorder + remove), run a real processing pass on a
/// synthetic frame, and bind its output to the first page. Returns a plan
/// plus materialization sources: originals are file-backed host files, the
/// derivative is in-memory bytes straight from `DeterministicProcessor`
/// (the shape a host codec will hand the exporter).
fn reviewed_plan(
    host: &Path,
) -> (
    foldscan_domain::ExportPlan,
    ExportManifest,
    HashMap<String, ExportSource>,
    Vec<u8>,
) {
    let vol = host.join("volume");
    std::fs::create_dir_all(&vol).unwrap();
    build_volume(&vol, &["p1", "p2", "p3"]);

    let imported = import_volume(&vol).expect("volume import");
    let mut sessions = export_sessions_from_import(&imported.sessions);
    let s = &mut sessions[0];
    assert!(reorder(s, "p3", 0));
    assert!(remove_from(s, "p2").is_some());

    let recipe = ProcessingRecipe {
        schema: "foldscan.recipe/0.1".to_string(),
        name: "deskew".to_string(),
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

    // Real processing run: the derivative payload is the processor's frame
    // bytes (a host codec would encode these later; here the raw frame stands
    // in for codec output, which does not change the executor contract).
    let source_frame =
        GrayFrame::from_pixels(4, 4, (0..16u8).map(|i| i.saturating_mul(16)).collect())
            .expect("source frame");
    let processed = DeterministicProcessor
        .process(&source_frame, &recipe)
        .expect("process");
    let derivative_bytes = processed.frame.pixels.clone();

    let first: &mut ExportPage = &mut s.pages[0];
    first.processed_media_type = Some("image/png".to_string());
    first.processed_sha256 = Some(sha256_hex(&derivative_bytes));
    first.processed_bytes = Some(derivative_bytes.len() as u64);
    // recipe_digest binds the recipe document the plan carries (the frame
    // digest lives inside processed_sha256's payload instead).
    first.recipe_digest = Some(recipe.digest());

    let plan = plan_export(&sessions, std::slice::from_ref(&recipe)).expect("plan");
    let manifest = ExportManifest::from_plan(&plan, &sessions[0].session_id).expect("manifest");

    // Originals are host files (distinct names per planned path so an
    // original and its derivative never share a host file). The derivative
    // arrives as in-memory bytes.
    let mut sources = HashMap::new();
    for f in &plan.files {
        match f.content_kind {
            foldscan_domain::ContentKind::Original => {
                let src = host.join(format!(
                    "src-orig-{}",
                    f.capture_id.as_deref().unwrap_or("x")
                ));
                std::fs::write(&src, FAKE_JPEG).unwrap();
                sources.insert(f.relative_path.clone(), ExportSource::file(src));
            }
            foldscan_domain::ContentKind::Derivative => {
                sources.insert(
                    f.relative_path.clone(),
                    ExportSource::bytes(derivative_bytes.clone()),
                );
            }
            _ => {}
        }
    }
    (plan, manifest, sources, derivative_bytes)
}

fn walk_files(base: &Path, dir: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk_files(base, &p, out);
        } else {
            out.push(
                p.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

#[test]
fn executes_plan_to_finalized_export_tree() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, derivative_bytes) = reviewed_plan(host.path());
    let root = host.path().join("out/export");

    let exec: ExportExecution =
        execute_export(&plan, &root, &sources, &manifest).expect("export executes");

    assert_eq!(exec.files_written, plan.files.len());
    assert_eq!(exec.root, root);

    // Every planned path exists on disk, nothing extra was written.
    let mut on_disk = Vec::new();
    walk_files(&root, &root, &mut on_disk);
    let planned: HashSet<String> = plan.files.iter().map(|f| f.relative_path.clone()).collect();
    let on_disk_set: HashSet<String> = on_disk.into_iter().collect();
    assert_eq!(planned, on_disk_set, "final tree matches the plan exactly");

    // Content bytes match the plan-declared checksums.
    for f in &plan.files {
        let bytes = std::fs::read(root.join(&f.relative_path)).unwrap();
        match f.content_kind {
            foldscan_domain::ContentKind::Original => {
                assert_eq!(sha256_hex(&bytes), sha256_hex(FAKE_JPEG));
            }
            foldscan_domain::ContentKind::Derivative => {
                assert_eq!(sha256_hex(&bytes), sha256_hex(&derivative_bytes));
                assert_eq!(bytes, derivative_bytes, "derivative bytes are exact");
            }
            _ => {}
        }
    }

    // The finalized export.json re-parses to the same manifest document.
    let manifest_bytes = std::fs::read(root.join("export.json")).unwrap();
    assert_eq!(exec.manifest_sha256, sha256_hex(&manifest_bytes));
    let back = ExportManifest::from_json_bytes(&manifest_bytes).expect("re-parses");
    assert_eq!(back.digest(), exec.manifest_digest);
    assert_eq!(back, manifest);

    // Recipe document finalized and bound by digest in its filename.
    let digest = plan.sessions[0].pages[0].recipe_digest.clone().unwrap();
    assert!(is_lowercase_hex_sha256(&digest));
    assert!(root.join(format!("recipes/{}.json", digest)).is_file());
}

#[test]
fn refuses_to_overwrite_existing_root() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, _) = reviewed_plan(host.path());
    let root = host.path().join("out2/export");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("keep.txt"), b"do not touch").unwrap();

    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    // Pre-existing content untouched.
    assert_eq!(
        std::fs::read(root.join("keep.txt")).unwrap(),
        b"do not touch".to_vec()
    );
}

#[test]
fn checksum_mismatch_rolls_back_whole_tree() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, _) = reviewed_plan(host.path());
    // Corrupt one original source after the plan recorded its checksum.
    let orig = plan
        .files
        .iter()
        .find(|f| f.content_kind == foldscan_domain::ContentKind::Original)
        .unwrap();
    std::fs::write(
        sources
            .get(&orig.relative_path)
            .and_then(|s| s.as_file_path())
            .unwrap(),
        b"tampered-bytes-longer",
    )
    .unwrap();

    let root = host.path().join("out3/export");
    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
    assert!(!root.exists(), "rollback removed the partial export tree");
}

#[test]
fn declared_size_mismatch_is_rejected_before_copy() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, _) = reviewed_plan(host.path());
    let orig = plan
        .files
        .iter()
        .find(|f| f.content_kind == foldscan_domain::ContentKind::Original)
        .unwrap();
    // A short file fails the declared-size gate before any bytes are copied.
    std::fs::write(
        sources
            .get(&orig.relative_path)
            .and_then(|s| s.as_file_path())
            .unwrap(),
        &FAKE_JPEG[..FAKE_JPEG.len() - 3],
    )
    .unwrap();

    let root = host.path().join("out4/export");
    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
    assert!(err.message.contains("bytes on disk but the plan declares"));
    assert!(!root.exists());
}

#[test]
fn memory_source_size_mismatch_is_rejected_before_write() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, mut sources, derivative_bytes) = reviewed_plan(host.path());
    // The plan declares the real derivative size; hand the executor a short
    // buffer instead. The gate must reject it before any staging begins.
    let deriv = plan
        .files
        .iter()
        .find(|f| f.content_kind == foldscan_domain::ContentKind::Derivative)
        .unwrap()
        .relative_path
        .clone();
    let mut short = derivative_bytes.clone();
    short.truncate(short.len() - 1);
    sources.insert(deriv, ExportSource::bytes(short));

    let root = host.path().join("out-mem-size/export");
    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
    assert!(err
        .message
        .contains("bytes in memory but the plan declares"));
    assert!(!root.exists());
}

#[test]
fn memory_source_checksum_mismatch_is_rejected_before_write() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, mut sources, derivative_bytes) = reviewed_plan(host.path());
    // Same length as declared, but flipped bytes: the declared-size gate
    // passes and the checksum gate must catch the tampering before staging.
    let deriv = plan
        .files
        .iter()
        .find(|f| f.content_kind == foldscan_domain::ContentKind::Derivative)
        .unwrap()
        .relative_path
        .clone();
    let mut tampered = derivative_bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF;
    sources.insert(deriv, ExportSource::bytes(tampered));

    let root = host.path().join("out-mem-sum/export");
    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
    assert!(err.message.contains("checksum does not match the plan"));
    assert!(!root.exists(), "no partial export may survive the gate");
}

#[test]
fn file_backed_derivative_still_supported() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, mut sources, derivative_bytes) = reviewed_plan(host.path());
    // Swap the in-memory derivative for a host file holding the same bytes:
    // the finalized tree must be identical to the bytes-backed path.
    let deriv = plan
        .files
        .iter()
        .find(|f| f.content_kind == foldscan_domain::ContentKind::Derivative)
        .unwrap()
        .relative_path
        .clone();
    let src = host.path().join("src-deriv-file");
    std::fs::write(&src, &derivative_bytes).unwrap();
    sources.insert(deriv.clone(), ExportSource::file(&src));

    let root = host.path().join("out-file-deriv/export");
    let exec = execute_export(&plan, &root, &sources, &manifest).expect("executes");
    assert_eq!(exec.files_written, plan.files.len());
    assert_eq!(
        std::fs::read(root.join(&deriv)).unwrap(),
        derivative_bytes,
        "file-backed derivative finalizes to the same bytes"
    );
}

#[test]
fn byte_backed_original_is_rejected_without_creating_root() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, mut sources, _) = reviewed_plan(host.path());
    // Originals must be re-read from the verified import — an in-memory
    // buffer standing in for one is rejected before the root exists.
    let orig = plan
        .files
        .iter()
        .find(|f| f.content_kind == foldscan_domain::ContentKind::Original)
        .unwrap()
        .relative_path
        .clone();
    sources.insert(orig, ExportSource::bytes(FAKE_JPEG.to_vec()));

    let root = host.path().join("out-orig-bytes/export");
    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("original source must be file-backed"));
    assert!(!root.exists());
}

#[test]
fn missing_source_is_rejected_without_creating_root() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, _) = reviewed_plan(host.path());
    let mut partial: HashMap<String, ExportSource> = sources
        .iter()
        .filter(|(path, _)| !path.starts_with("originals/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // Ensure at least one original is missing.
    assert!(partial.len() < sources.len());

    let root = host.path().join("out5/export");
    let err = execute_export(&plan, &root, &partial, &manifest).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("missing export source"));
    assert!(!root.exists(), "request-shape failure must not touch disk");
    partial.clear();
}

#[test]
fn unplanned_source_is_rejected() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, mut sources, _) = reviewed_plan(host.path());
    let smuggle = host.path().join("smuggle.jpg");
    std::fs::write(&smuggle, FAKE_JPEG).unwrap();
    sources.insert(
        "originals/sess-001/extra.jpg".to_string(),
        ExportSource::file(smuggle),
    );

    let root = host.path().join("out6/export");
    let err = execute_export(&plan, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("unplanned path"));
    assert!(!root.exists());
}

#[test]
fn session_mismatch_between_manifest_and_plan_is_rejected() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, _) = reviewed_plan(host.path());
    let mut shifted = plan.clone();
    shifted.sessions[0].session_id = "sess-999".to_string();

    let root = host.path().join("out7/export");
    let err = execute_export(&shifted, &root, &sources, &manifest).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(!root.exists());
}

#[test]
fn finalized_tree_contains_no_staged_part_files() {
    let host = TempDir::new().unwrap();
    let (plan, manifest, sources, _) = reviewed_plan(host.path());
    let root = host.path().join("out8/export");
    execute_export(&plan, &root, &sources, &manifest).expect("executes");

    let mut files = Vec::new();
    walk_files(&root, &root, &mut files);
    assert!(
        !files.iter().any(|f| f.ends_with(PART_SUFFIX)),
        "no staging leftovers: {:?}",
        files
    );
}
