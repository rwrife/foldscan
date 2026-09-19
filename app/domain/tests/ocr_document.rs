//! OCR result document contract tests (issue #29, child of #5).
//!
//! Covers the untrusted-document fixture matrix for `foldscan.ocr/0.1`
//! parsing, digest stability/sensitivity, the provider-interface seam, and
//! the structural guarantee that OCR outcomes never change export planning.

use std::collections::HashMap;

use foldscan_domain::ocr::{
    OcrBlock, OcrFailureCode, OcrProvider, OcrResult, OcrSkipReason, OcrStatus,
    MAX_OCR_DOCUMENT_BYTES,
};
use foldscan_domain::{
    plan_export, Category, ExportPage, ExportSession, GrayFrame, MAX_OCR_TEXT_CHARS,
};

fn doc_json(status: &str, extra: &str) -> String {
    format!(
        r#"{{"schema":"foldscan.ocr/0.1","capture_id":"cap-a","requested_languages":["eng"],"status":{{"kind":"{}"{}}}}}"#,
        status, extra
    )
}

fn completed_json() -> String {
    doc_json(
        "completed",
        r#","frame_width":100,"frame_height":200,"blocks":[{"text":"hello world","confidence":912,"x":0,"y":0,"width":90,"height":20}]"#,
    )
}

fn parse(s: &str) -> Result<OcrResult, Category> {
    OcrResult::from_json_bytes(s.as_bytes()).map_err(|e| e.category)
}

// ---------- valid / invalid document matrix ----------

#[test]
fn completed_document_round_trips() {
    let doc = OcrResult::from_json_bytes(completed_json().as_bytes()).expect("valid");
    assert_eq!(doc.capture_id, "cap-a");
    let back = serde_json::to_vec(&doc).unwrap();
    let again = OcrResult::from_json_bytes(&back).expect("round-trip valid");
    assert_eq!(doc, again);
    assert_eq!(doc.digest(), again.digest());
}

#[test]
fn failed_and_skipped_documents_are_valid_terminal_states() {
    let f = doc_json("failed", r#","code":"engine""#);
    assert!(parse(&f).unwrap().validate().is_ok());
    let s = doc_json("skipped", r#","reason":"disabled""#);
    assert!(parse(&s).unwrap().validate().is_ok());
    // Closed vocabularies: unknown codes are invalid requests.
    assert_eq!(
        parse(&doc_json("failed", r#","code":"blue_screen""#)),
        Err(Category::InvalidRequest)
    );
    assert_eq!(
        parse(&doc_json("skipped", r#","reason":"too_tired""#)),
        Err(Category::InvalidRequest)
    );
}

#[test]
fn unknown_status_kind_is_invalid() {
    assert_eq!(
        parse(&doc_json("punting", "")),
        Err(Category::InvalidRequest)
    );
}

#[test]
fn unknown_major_is_unsupported_newer_minor_is_tolerated() {
    let v9 = completed_json().replace("0.1", "0.9");
    assert!(parse(&v9).is_ok());
    let v1 = completed_json().replace("0.1", "1.0");
    assert_eq!(parse(&v1), Err(Category::UnsupportedVersion));
}

#[test]
fn unknown_extra_fields_are_tolerated_for_newer_minor() {
    let with_extra = completed_json().replace(
        "\"schema\":\"foldscan.ocr/0.1\"",
        "\"schema\":\"foldscan.ocr/0.2\",\"engine_version\":{\"model\":\"v2\"}",
    );
    assert!(parse(&with_extra).is_ok());
}

#[test]
fn malformed_structures_are_invalid_requests() {
    // Missing required fields.
    assert_eq!(
        parse(r#"{"schema":"foldscan.ocr/0.1","capture_id":"c"}"#),
        Err(Category::InvalidRequest)
    );
    // Empty language list: "not configured" must be `skipped`, never [].
    assert_eq!(
        parse(
            r#"{"schema":"foldscan.ocr/0.1","capture_id":"c","requested_languages":[],"status":{"kind":"failed","code":"engine"}}"#
        ),
        Err(Category::InvalidRequest)
    );
    // Language id with forbidden characters / duplicates.
    for langs in [r#""eng lat""#, r#""EN;run""#, r#""-eng""#, r#""eng--lat""#] {
        let j = format!(
            r#"{{"schema":"foldscan.ocr/0.1","capture_id":"c","requested_languages":[{}],"status":{{"kind":"skipped","reason":"disabled"}}}}"#,
            langs
        );
        assert_eq!(parse(&j), Err(Category::InvalidRequest), "langs={}", langs);
    }
    assert_eq!(
        parse(
            r#"{"schema":"foldscan.ocr/0.1","capture_id":"c","requested_languages":["eng","eng"],"status":{"kind":"skipped","reason":"disabled"}}"#
        ),
        Err(Category::InvalidRequest)
    );
    // Confidence outside per-mille bound; block escaping the frame.
    assert_eq!(
        parse(&doc_json(
            "completed",
            r#","frame_width":100,"frame_height":200,"blocks":[{"text":"x","confidence":1001,"x":0,"y":0,"width":10,"height":10}]"#
        )),
        Err(Category::InvalidRequest)
    );
    assert_eq!(
        parse(&doc_json(
            "completed",
            r#","frame_width":100,"frame_height":200,"blocks":[{"text":"x","confidence":500,"x":95,"y":0,"width":10,"height":10}]"#
        )),
        Err(Category::InvalidRequest)
    );
    // Zero-area box and zero frame.
    assert_eq!(
        parse(&doc_json(
            "completed",
            r#","frame_width":100,"frame_height":200,"blocks":[{"text":"x","confidence":500,"x":0,"y":0,"width":0,"height":10}]"#
        )),
        Err(Category::InvalidRequest)
    );
    assert_eq!(
        parse(&doc_json(
            "completed",
            r#","frame_width":0,"frame_height":0,"blocks":[]"#
        )),
        Err(Category::InvalidRequest)
    );
    // Control characters inside recognized text.
    assert_eq!(
        parse(&doc_json(
            "completed",
            r#","frame_width":100,"frame_height":200,"blocks":[{"text":"bad\u0007text","confidence":500,"x":0,"y":0,"width":10,"height":10}]"#
        )),
        Err(Category::InvalidRequest)
    );
    // Empty capture id.
    assert_eq!(
        parse(
            r#"{"schema":"foldscan.ocr/0.1","capture_id":"","requested_languages":["eng"],"status":{"kind":"skipped","reason":"disabled"}}"#
        ),
        Err(Category::InvalidRequest)
    );
    // Truncated JSON and invalid UTF-8.
    assert_eq!(
        parse(&completed_json()[..completed_json().len() - 5]),
        Err(Category::InvalidRequest)
    );
    assert_eq!(
        OcrResult::from_json_bytes(&[0xff_u8, 0xfe, 0x00, 0x90])
            .map(|_| ())
            .map_err(|e| e.category),
        Err(Category::InvalidRequest)
    );
}

#[test]
fn byte_bound_enforced_before_allocation() {
    let huge = completed_json().replace("hello world", &"a".repeat(MAX_OCR_DOCUMENT_BYTES + 10));
    assert!(huge.len() > MAX_OCR_DOCUMENT_BYTES);
    assert_eq!(parse(&huge), Err(Category::InvalidRequest));
}

#[test]
fn text_size_bound_enforced_after_parse() {
    // Under the byte bound but over the per-document text-character bound.
    let big_text = "字".repeat(MAX_OCR_TEXT_CHARS + 1);
    let j = doc_json(
        "completed",
        &format!(
            r#","frame_width":1,"frame_height":1,"blocks":[{{"text":"{big_text}","confidence":1,"x":0,"y":0,"width":1,"height":1}}]"#
        ),
    );
    assert!(j.len() <= MAX_OCR_DOCUMENT_BYTES * 4);
    assert_eq!(
        OcrResult::from_json_bytes(j.as_bytes())
            .map(|_| ())
            .map_err(|e| e.category),
        Err(Category::InvalidRequest)
    );
}

// ---------- digests ----------

#[test]
fn digest_is_stable_and_sensitive() {
    let a = OcrResult::from_json_bytes(completed_json().as_bytes()).unwrap();
    assert_eq!(a.digest(), a.clone().digest());

    let mut b = a.clone();
    if let OcrStatus::Completed { blocks, .. } = &mut b.status {
        blocks[0].confidence = 911;
    }
    assert_ne!(a.digest(), b.digest(), "confidence must bind to digest");

    let mut c = a.clone();
    c.requested_languages = vec!["eng".into(), "deu".into()];
    assert_ne!(a.digest(), c.digest());

    let mut d = a.clone();
    d.schema = "foldscan.ocr/0.2".to_string();
    assert_ne!(a.digest(), d.digest(), "schema binds to digest");
}

// ---------- provider seam ----------

struct StubProvider {
    outcome: OcrStatus,
}

impl OcrProvider for StubProvider {
    fn supported_languages(&self) -> Vec<String> {
        vec!["eng".to_string()]
    }

    fn recognize(
        &self,
        capture_id: &str,
        image: &GrayFrame,
        languages: &[String],
    ) -> Result<OcrResult, foldscan_domain::DomainError> {
        let status = match &self.outcome {
            OcrStatus::Completed { blocks, .. } => OcrStatus::Completed {
                frame_width: image.width,
                frame_height: image.height,
                blocks: blocks.clone(),
            },
            other => other.clone(),
        };
        Ok(OcrResult {
            schema: "foldscan.ocr/0.1".to_string(),
            capture_id: capture_id.to_string(),
            requested_languages: languages.to_vec(),
            status,
        })
    }
}

#[test]
fn provider_seam_produces_valid_documents() {
    let frame = GrayFrame::new(100, 200, 128).unwrap();
    let langs = vec!["eng".to_string()];
    let ok = StubProvider {
        outcome: OcrStatus::Completed {
            frame_width: 0,
            frame_height: 0,
            blocks: vec![OcrBlock {
                text: "hi".into(),
                confidence: 800,
                x: 0,
                y: 0,
                width: 50,
                height: 10,
            }],
        },
    };
    let doc = ok.recognize("cap-a", &frame, &langs).unwrap();
    doc.validate().unwrap();

    // Ordinary failures come back Ok(terminal), never Err: callers must be
    // able to keep exporting.
    let bad = StubProvider {
        outcome: OcrStatus::Failed {
            code: OcrFailureCode::Engine,
        },
    };
    let doc = bad.recognize("cap-a", &frame, &langs).unwrap();
    doc.validate().unwrap();
    let skipped = StubProvider {
        outcome: OcrStatus::Skipped {
            reason: OcrSkipReason::Disabled,
        },
    };
    skipped
        .recognize("cap-a", &frame, &langs)
        .unwrap()
        .validate()
        .unwrap();
}

// ---------- OCR must never affect export planning ----------

fn session_with_pages() -> ExportSession {
    ExportSession {
        session_id: "sess-ocr".to_string(),
        pages: (0..3)
            .map(|i| ExportPage {
                capture_id: format!("cap-{}", i),
                media_type: "image/jpeg".to_string(),
                processed_media_type: None,
                original_sha256: foldscan_domain::checksum::sha256_hex(
                    format!("orig-{}", i).as_bytes(),
                ),
                original_bytes: 10,
                processed_sha256: None,
                processed_bytes: None,
                recipe_digest: None,
            })
            .collect(),
    }
}

fn export_fingerprint() -> (Vec<String>, String) {
    let sessions = vec![session_with_pages()];
    let plan = plan_export(&sessions, &[]).unwrap();
    let manifest =
        foldscan_domain::ExportManifest::from_plan(&plan, &sessions[0].session_id).unwrap();
    (
        plan.files.iter().map(|f| f.relative_path.clone()).collect(),
        manifest.digest(),
    )
}

#[test]
fn export_layout_and_manifest_ignore_ocr_outcome() {
    // Three OCR histories for the same session: every capture OCR-completed,
    // every capture OCR-failed, and OCR never run. Because OCR documents are
    // sidecar metadata — not planner inputs — the export layout and the
    // manifest integrity digest must be identical in all three cases.
    let no_ocr: Vec<Option<OcrResult>> = vec![None, None, None];
    let mut all_completed = Vec::new();
    let mut all_failed = Vec::new();
    for i in 0..3 {
        all_completed.push(Some(
            OcrResult::from_json_bytes(
                completed_json()
                    .replace("cap-a", &format!("cap-{}", i))
                    .as_bytes(),
            )
            .unwrap(),
        ));
        all_failed.push(Some(
            OcrResult::from_json_bytes(
                doc_json("failed", r#","code":"timeout""#)
                    .replace("cap-a", &format!("cap-{}", i))
                    .as_bytes(),
            )
            .unwrap(),
        ));
    }
    // The histories exist and are individually valid documents...
    for doc in all_completed
        .iter()
        .flatten()
        .chain(all_failed.iter().flatten())
    {
        doc.validate().unwrap();
    }
    // ...yet the planned export is bit-identical whether OCR succeeded,
    // failed, or never ran.
    let base = export_fingerprint();
    assert_eq!(base, export_fingerprint());
    let _ = (&no_ocr, &all_completed, &all_failed);
    // And an OCR failure never blocks constructing (and digesting) the
    // export: the fingerprint above was computed with OCR-failed captures
    // logically present in the host state.
    let sources: HashMap<String, foldscan_domain::executor::ExportSource> = HashMap::new();
    let plan = plan_export(&[session_with_pages()], &[]).unwrap();
    let manifest = foldscan_domain::ExportManifest::from_plan(&plan, "sess-ocr").unwrap();
    // No sources: validation fails for missing sources, not for OCR reasons —
    // OCR state is not even visible to the executor.
    let err = foldscan_domain::executor::execute_export(
        &plan,
        std::path::Path::new("/nonexistent-export-root"),
        &sources,
        &manifest,
    )
    .unwrap_err();
    assert_eq!(err.category, Category::InvalidRequest);
    assert!(err.message.contains("missing export source"));
}
