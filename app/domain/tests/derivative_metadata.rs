//! Synthetic metadata fixtures, not image codec or physical-device evidence.
use foldscan_domain::checksum::sha256_hex;
use foldscan_domain::error::Category;
use foldscan_domain::{plan_export, ExportManifest, ExportPage, ExportSession, ProcessingRecipe};

fn recipe() -> ProcessingRecipe {
    ProcessingRecipe {
        schema: "foldscan.recipe/0.1".into(),
        name: "identity".into(),
        ops: vec![],
    }
}

fn session(mask: u8) -> ExportSession {
    ExportSession {
        session_id: "sess-001".into(),
        pages: vec![ExportPage {
            capture_id: "page-001".into(),
            media_type: "image/jpeg".into(),
            original_sha256: sha256_hex(b"original"),
            original_bytes: 8,
            processed_sha256: (mask & 1 != 0).then(|| sha256_hex(b"derived")),
            processed_bytes: (mask & 2 != 0).then_some(7),
            recipe_digest: (mask & 4 != 0).then(|| recipe().digest()),
            processed_media_type: (mask & 8 != 0).then(|| "image/png".into()),
        }],
        document: None,
        ocr: Vec::new(),
    }
}

#[test]
fn executor_rejects_invalid_derivatives_without_creating_parent_or_touching_original() {
    use foldscan_domain::executor::{execute_export, ExportSource};
    use foldscan_domain::limits::MAX_CAPTURE_BYTES;
    use foldscan_domain::ContentKind;
    use std::collections::HashMap;

    for case in 0..18 {
        if matches!(case, 0 | 7 | 15) {
            continue;
        }
        let host = tempfile::TempDir::new().unwrap();
        let original = host.path().join("original.jpg");
        std::fs::write(&original, b"original").unwrap();
        let mut plan = plan_export(&[session(15)], &[recipe()]).unwrap();
        let mut manifest = ExportManifest::from_plan(&plan, "sess-001").unwrap();
        let mut sources = HashMap::new();
        for file in &plan.files {
            match file.content_kind {
                ContentKind::Original => {
                    sources.insert(file.relative_path.clone(), ExportSource::file(&original));
                }
                ContentKind::Derivative => {
                    sources.insert(
                        file.relative_path.clone(),
                        ExportSource::bytes(b"derived".to_vec()),
                    );
                }
                _ => {}
            }
        }
        if case < 16 {
            plan.sessions = vec![session(case)];
            let page = &plan.sessions[0].pages[0];
            manifest.pages[0].processed_sha256 = page.processed_sha256.clone();
            manifest.pages[0].processed_bytes = page.processed_bytes;
            manifest.pages[0].recipe_digest = page.recipe_digest.clone();
            if page.processed_sha256.is_none() {
                manifest.pages[0].processed_path = None;
                plan.files
                    .retain(|f| f.content_kind != ContentKind::Derivative);
                sources.retain(|path, _| !path.starts_with("processed/"));
            }
            if page.recipe_digest.is_none() {
                plan.files.retain(|f| f.content_kind != ContentKind::Recipe);
            }
        } else {
            let size = if case == 16 {
                MAX_CAPTURE_BYTES + 1
            } else {
                u64::MAX
            };
            plan.sessions[0].pages[0].processed_bytes = Some(size);
            manifest.pages[0].processed_bytes = Some(size);
        }
        let root = host.path().join("must-not-exist/export");
        let result = execute_export(&plan, &root, &sources, &manifest);
        assert!(
            !root.parent().unwrap().exists(),
            "case {case}: parent created: {result:?}"
        );
        assert_eq!(
            std::fs::read(&original).unwrap(),
            b"original",
            "case {case}"
        );
        let err = result.expect_err("invalid derivative must fail preflight");
        assert_eq!(
            err.category,
            Category::InvalidRequest,
            "case {case}: {err:?}"
        );
    }
}

#[test]
fn derivative_digests_must_be_lowercase_sha256() {
    let plan = plan_export(&[session(15)], &[recipe()]).unwrap();
    let complete = ExportManifest::from_plan(&plan, "sess-001").unwrap();
    let mut wrongly_accepted = vec![];
    for field in ["processed_sha256", "recipe_digest"] {
        for value in [
            "".to_string(),
            "A".repeat(64),
            "g".repeat(64),
            "a".repeat(63),
            "a".repeat(65),
        ] {
            let mut s = session(15);
            let mut manifest = complete.clone();
            if field == "processed_sha256" {
                s.pages[0].processed_sha256 = Some(value.clone());
                manifest.pages[0].processed_sha256 = Some(value.clone());
            } else {
                s.pages[0].recipe_digest = Some(value.clone());
                manifest.pages[0].recipe_digest = Some(value.clone());
            }
            for (entry, result) in [
                ("plan", plan_export(&[s], &[recipe()]).map(|_| ())),
                ("validate", manifest.validate().map(|_| ())),
                (
                    "parse",
                    ExportManifest::from_json_bytes(&serde_json::to_vec(&manifest).unwrap())
                        .map(|_| ()),
                ),
            ] {
                match result {
                    Ok(()) => wrongly_accepted.push((entry, field, value.clone())),
                    Err(err) => assert_eq!(err.category, Category::InvalidRequest),
                }
            }
        }
    }
    assert!(
        wrongly_accepted.is_empty(),
        "accepted malformed digests: {wrongly_accepted:?}"
    );
}

#[test]
fn derivative_byte_limit_is_checked_by_planner_and_manifest() {
    use foldscan_domain::limits::MAX_CAPTURE_BYTES;
    let plan = plan_export(&[session(15)], &[recipe()]).unwrap();
    let complete = ExportManifest::from_plan(&plan, "sess-001").unwrap();
    let mut wrongly_accepted = vec![];
    for size in [0, MAX_CAPTURE_BYTES, MAX_CAPTURE_BYTES + 1, u64::MAX] {
        let mut s = session(15);
        s.pages[0].processed_bytes = Some(size);
        let mut manifest = complete.clone();
        manifest.pages[0].processed_bytes = Some(size);
        let results = [
            ("plan", plan_export(&[s], &[recipe()]).map(|_| ())),
            ("validate", manifest.validate().map(|_| ())),
            (
                "parse",
                ExportManifest::from_json_bytes(&serde_json::to_vec(&manifest).unwrap())
                    .map(|_| ()),
            ),
        ];
        for (entry, result) in results {
            if size <= MAX_CAPTURE_BYTES {
                result.expect("in-bound declarations accepted without allocating payloads");
            } else {
                match result {
                    Ok(()) => wrongly_accepted.push((entry, size)),
                    Err(err) => assert_eq!(err.category, Category::InvalidRequest),
                }
            }
        }
    }
    assert!(
        wrongly_accepted.is_empty(),
        "accepted oversized derivatives: {wrongly_accepted:?}"
    );
}

#[test]
fn manifest_requires_complete_derivative_metadata() {
    let plan = plan_export(&[session(15)], &[recipe()]).unwrap();
    let complete = ExportManifest::from_plan(&plan, "sess-001").unwrap();
    let mut wrongly_accepted = vec![];
    for mask in 0..16 {
        let mut manifest = complete.clone();
        let page = &mut manifest.pages[0];
        if mask & 1 == 0 {
            page.processed_sha256 = None;
        }
        if mask & 2 == 0 {
            page.processed_bytes = None;
        }
        if mask & 4 == 0 {
            page.recipe_digest = None;
        }
        if mask & 8 == 0 {
            page.processed_path = None;
        }
        let parsed = ExportManifest::from_json_bytes(&serde_json::to_vec(&manifest).unwrap());
        if matches!(mask, 0 | 15) {
            manifest.validate().unwrap();
            assert_eq!(parsed.unwrap(), manifest);
        } else {
            for (entry, result) in [
                ("validate", manifest.validate().map(|_| ())),
                ("parse", parsed.map(|_| ())),
            ] {
                match result {
                    Ok(()) => wrongly_accepted.push((entry, mask)),
                    Err(err) => assert_eq!(err.category, Category::InvalidRequest),
                }
            }
        }
    }
    assert!(
        wrongly_accepted.is_empty(),
        "accepted partial metadata: {wrongly_accepted:?}"
    );
}

#[test]
fn planner_requires_complete_derivative_metadata() {
    let mut wrongly_accepted = vec![];
    for mask in 0..16 {
        let result = plan_export(&[session(mask)], &[recipe()]);
        if matches!(mask, 0 | 7 | 15) {
            let plan = result.expect("original-only or complete derivative");
            if mask != 0 {
                assert!(plan
                    .files
                    .iter()
                    .any(|f| f.relative_path == "processed/sess-001/page-001.png"));
            }
        } else {
            match result {
                Ok(_) => wrongly_accepted.push(mask),
                Err(err) => assert_eq!(err.category, Category::InvalidRequest),
            }
        }
    }
    assert!(
        wrongly_accepted.is_empty(),
        "accepted partial metadata masks: {wrongly_accepted:?}"
    );
}
