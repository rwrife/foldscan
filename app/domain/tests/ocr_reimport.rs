//! Read-only OCR export re-import, exercised against a real domain export.
//! Software fixtures only: no OCR engine or physical device.
use std::collections::HashMap;
use std::path::Path;

use foldscan_domain::checksum::sha256_hex;
use foldscan_domain::executor::{execute_export, ExportSource};
use foldscan_domain::ocr::{OcrBlock, OcrResult, OcrStatus};
use foldscan_domain::{
    load_exported_ocr, plan_export, Category, ExportManifest, ExportPage, ExportSession,
};
use tempfile::TempDir;

const ORIGINAL: &[u8] = b"synthetic original bytes";

fn fixture() -> (TempDir, String, OcrResult) {
    let temp = TempDir::new().unwrap();
    let doc = OcrResult {
        schema: "foldscan.ocr/0.1".into(),
        capture_id: "p1".into(),
        requested_languages: vec!["eng".into()],
        status: OcrStatus::Completed {
            frame_width: 40,
            frame_height: 40,
            blocks: vec![OcrBlock {
                text: "private test".into(),
                confidence: 900,
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            }],
        },
    };
    let page = |id: &str| ExportPage {
        capture_id: id.into(),
        media_type: "image/jpeg".into(),
        processed_media_type: None,
        original_sha256: sha256_hex(ORIGINAL),
        original_bytes: ORIGINAL.len() as u64,
        processed_sha256: None,
        processed_bytes: None,
        recipe_digest: None,
    };
    let session = ExportSession {
        session_id: "session-1".into(),
        pages: vec![page("p2"), page("p1")],
        document: None,
        ocr: vec![doc.clone()],
        ocr_text: true,
    };
    let plan = plan_export(&[session], &[]).unwrap();
    let manifest = ExportManifest::from_plan(&plan, "session-1").unwrap();
    let digest = manifest.digest();
    let src = temp.path().join("source.jpg");
    std::fs::write(&src, ORIGINAL).unwrap();
    let sources = ["p1", "p2"]
        .into_iter()
        .map(|id| {
            (
                format!("originals/session-1/{id}.jpg"),
                ExportSource::file(src.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    execute_export(&plan, &temp.path().join("out"), &sources, &manifest).unwrap();
    (temp, digest, doc)
}

fn root(tmp: &TempDir) -> std::path::PathBuf {
    tmp.path().join("out")
}

#[test]
fn round_trip_returns_only_completed_bound_documents_in_page_order() {
    let (tmp, digest, doc) = fixture();
    assert_eq!(load_exported_ocr(&root(&tmp), &digest).unwrap(), vec![doc]);
    assert_eq!(
        std::fs::read(root(&tmp).join("ocr/session-1/p1.txt")).unwrap(),
        b"private test\n"
    );
}

#[test]
fn two_bound_documents_follow_manifest_page_order_not_disk_order() {
    let (tmp, _, first) = fixture();
    let mut second = first.clone();
    second.capture_id = "p2".into();
    let json = root(&tmp).join("ocr/session-1/p2.json");
    std::fs::write(json, serde_json::to_vec(&second).unwrap()).unwrap();
    let path = root(&tmp).join("export.json");
    let mut manifest = ExportManifest::from_json_bytes(&std::fs::read(&path).unwrap()).unwrap();
    manifest.pages[0].ocr_digest = Some(second.digest());
    let digest = manifest.digest();
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest).unwrap(),
        vec![second, first]
    );
}

#[test]
fn requires_the_reviewed_manifest_digest() {
    let (tmp, _, _) = fixture();
    let err = load_exported_ocr(&root(&tmp), &"0".repeat(64)).unwrap_err();
    assert_eq!(err.category, Category::ChecksumMismatch);
}

#[test]
fn rejects_changed_json_even_if_valid_ocr() {
    let (tmp, digest, _) = fixture();
    let path = root(&tmp).join("ocr/session-1/p1.json");
    let json = std::fs::read_to_string(&path)
        .unwrap()
        .replace("private test", "private edit");
    std::fs::write(path, json).unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest)
            .unwrap_err()
            .category,
        Category::ChecksumMismatch
    );
}

#[test]
fn rejects_changed_text_even_if_json_is_intact() {
    let (tmp, digest, _) = fixture();
    std::fs::write(root(&tmp).join("ocr/session-1/p1.txt"), b"private edit\n").unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest)
            .unwrap_err()
            .category,
        Category::ChecksumMismatch
    );
}

#[test]
fn rejects_non_completed_ocr_even_when_manifest_digest_is_re_pinned() {
    let (tmp, _, _) = fixture();
    let json = root(&tmp).join("ocr/session-1/p1.json");
    let mut doc = OcrResult::from_json_bytes(&std::fs::read(&json).unwrap()).unwrap();
    doc.status = OcrStatus::Failed {
        code: foldscan_domain::OcrFailureCode::Engine,
    };
    std::fs::write(&json, serde_json::to_vec(&doc).unwrap()).unwrap();
    let path = root(&tmp).join("export.json");
    let mut manifest = ExportManifest::from_json_bytes(&std::fs::read(&path).unwrap()).unwrap();
    manifest.pages[1].ocr_digest = Some(doc.digest());
    manifest.pages[1].ocr_text_digest = None;
    let digest = manifest.digest();
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest)
            .unwrap_err()
            .category,
        Category::InvalidRequest
    );
}

#[test]
fn rejects_missing_and_oversized_sidecars_without_private_diagnostics() {
    let (tmp, digest, _) = fixture();
    let path = root(&tmp).join("ocr/session-1/p1.json");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest)
            .unwrap_err()
            .category,
        Category::StorageUnavailable
    );
    std::fs::write(
        &path,
        vec![b'x'; foldscan_domain::MAX_OCR_DOCUMENT_BYTES + 1],
    )
    .unwrap();
    let err = load_exported_ocr(&root(&tmp), &digest).unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(!err.message.contains(tmp.path().to_str().unwrap()));
    assert!(!err.message.contains("private test"));
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_sidecar_and_directory() {
    use std::os::unix::fs::symlink;
    let (tmp, digest, _) = fixture();
    let ocr = root(&tmp).join("ocr");
    let json = ocr.join("session-1/p1.json");
    let other = tmp.path().join("other.json");
    std::fs::rename(&json, &other).unwrap();
    symlink(&other, &json).unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest)
            .unwrap_err()
            .category,
        Category::InvalidRequest
    );
    std::fs::remove_file(json).unwrap();
    let moved = tmp.path().join("moved-ocr");
    std::fs::rename(&ocr, &moved).unwrap();
    symlink(&moved, &ocr).unwrap();
    assert_eq!(
        load_exported_ocr(&root(&tmp), &digest)
            .unwrap_err()
            .category,
        Category::InvalidRequest
    );
}

#[test]
fn malformed_manifest_cannot_choose_an_untrusted_ocr_path() {
    let (tmp, _, _) = fixture();
    let path = root(&tmp).join("export.json");
    let bytes = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"p1\"", "\"../p1\"");
    std::fs::write(&path, bytes).unwrap();
    let err = load_exported_ocr(&root(&tmp), &"0".repeat(64)).unwrap_err();
    assert!(matches!(
        err.category,
        Category::InvalidRequest | Category::ChecksumMismatch
    ));
}

#[test]
fn cannot_use_a_missing_export_directory() {
    let err =
        load_exported_ocr(Path::new("/nonexistent-foldscan-export"), &"0".repeat(64)).unwrap_err();
    assert_eq!(err.category, Category::StorageUnavailable);
}
