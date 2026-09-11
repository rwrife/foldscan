//! Filesystem executor for collision-checked export plans.
//!
//! [`crate::export::plan_export`] produces an intended export layout but
//! deliberately writes nothing. This module materializes such a plan on disk
//! under the protocol's durable-write rule (docs/protocol.md § Durability):
//! content is staged under temporary names, flushed, checksum-verified, and
//! only then atomically renamed into its final planned location; the
//! portable manifest is written last, after every content file finalized.
//!
//! Guarantees this slice provides:
//! - **No overwrite, ever**: the export root must not already exist. The
//!   executor never merges into or replaces an existing directory; a failed
//!   run removes only the tree it created in this invocation.
//! - **Plan-bounded writes**: only paths listed in the plan are written.
//!   Sources supplied for unplanned paths are rejected before any disk
//!   mutation, so an executor call cannot smuggle extra files into a
//!   reviewed export.
//! - **Declared-bytes verification**: each original/derivative source file
//!   is rejected unless its size matches the plan's declared size *before*
//!   being read, and its streamed SHA-256 matches the checksum the plan
//!   recorded — the same pre-allocation discipline the importer uses.
//! - **Read-back verification**: staged bytes are re-hashed from disk before
//!   the rename, and recipe/manifest documents must re-parse to the exact
//!   document they were serialized from.
//! - **Rollback**: any failure after the root is created removes the whole
//!   export tree, leaving no partially finalized export and no `.part`
//!   leftovers to confuse a later run.
//!
//! Known limitations (later slices): one session per export run (multi-session
//! export layout is not finalized in the planner yet); content arrives as
//! source file paths only (in-memory derivative buffers are a later need);
//! parent-directory fsync is not attempted, matching the host-side durability
//! bar the protocol states for rename-capable filesystems.
//!
//! Evidence category: filesystem code exercised on synthetic fixtures in a
//! temporary directory. No physical device or optical-hardware evidence.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::checksum::sha256_hex;
use crate::error::DomainError;
use crate::export::{
    ContentKind, ExportManifest, ExportPage, ExportPlan, ExportSession, PlannedFile,
    MAX_EXPORT_MANIFEST_BYTES, RECIPES_DIR,
};
use crate::limits::MAX_CAPTURE_BYTES;
use crate::paths::safe_join;
use crate::recipe::{ProcessingRecipe, MAX_RECIPE_BYTES};

/// Suffix appended to a file's name while it is staged but not finalized.
/// Finalized exports must contain none of these.
pub const PART_SUFFIX: &str = ".foldscan-part";

/// Size of the streaming copy buffer for source files.
const COPY_BUF_BYTES: usize = 64 * 1024;

/// Summary of one successfully executed export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportExecution {
    /// The export root the files were written under.
    pub root: PathBuf,
    /// Number of planned files finalized (including the manifest).
    pub files_written: usize,
    /// Lowercase-hex SHA-256 of the finalized `export.json` bytes.
    pub manifest_sha256: String,
    /// [`ExportManifest::digest`] of the finalized manifest document.
    pub manifest_digest: String,
}

/// Materialize a validated [`ExportPlan`] plus one session's
/// [`ExportManifest`] under `root`.
///
/// `sources` maps each planned *original* and *derivative* relative path to
/// the host file holding its bytes. Recipe documents are serialized from the
/// plan itself; the manifest document is written last from `manifest`.
///
/// The executor refuses to write if `root` already exists, if a source is
/// missing or declared-size/checksum-mismatched, or if a source is supplied
/// for a path the plan never laid out. On any failure after the root was
/// created, the export tree created by this call is removed.
pub fn execute_export(
    plan: &ExportPlan,
    root: &Path,
    sources: &HashMap<String, PathBuf>,
    manifest: &ExportManifest,
) -> Result<ExportExecution, DomainError> {
    let session = validate_request_shape(plan, manifest, sources)?;

    if root.exists() {
        return Err(DomainError::invalid_request(
            "export root already exists; the executor never overwrites an existing directory",
        ));
    }
    // Parent directories may be created on the way; the export root itself
    // must be new (checked above, re-checked by create_dir for the race).
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            DomainError::storage_unavailable(format!(
                "cannot create export parent directory: {}",
                kind_of(&e)
            ))
        })?;
    }
    match fs::create_dir(root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(DomainError::invalid_request(
                "export root already exists; the executor never overwrites an existing directory",
            ));
        }
        Err(e) => {
            return Err(DomainError::storage_unavailable(format!(
                "cannot create export root: {}",
                kind_of(&e)
            )));
        }
    }

    match run_plan(plan, session, root, sources, manifest) {
        Ok(exec) => Ok(exec),
        Err(e) => {
            // Rollback: only the tree this call created is removed.
            let _ = fs::remove_dir_all(root);
            Err(e)
        }
    }
}

/// All checks that can run *before* the root exists. Anything failing here
/// means the executor touched no disk state at all.
fn validate_request_shape<'a>(
    plan: &'a ExportPlan,
    manifest: &ExportManifest,
    sources: &HashMap<String, PathBuf>,
) -> Result<&'a ExportSession, DomainError> {
    if plan.sessions.len() != 1 {
        return Err(DomainError::invalid_request(
            "this executor slice writes one session per export run; use one plan per session",
        ));
    }
    let session = &plan.sessions[0];
    if session.session_id != manifest.session_id {
        return Err(DomainError::invalid_request(
            "export manifest session does not match the export plan",
        ));
    }
    manifest.validate()?;

    // Planner invariant: exactly one manifest entry, planned last.
    let manifest_count = plan
        .files
        .iter()
        .filter(|f| f.content_kind == ContentKind::Manifest)
        .count();
    if manifest_count != 1
        || plan.files.last().map(|f| f.content_kind) != Some(ContentKind::Manifest)
    {
        return Err(DomainError::invalid_request(
            "export plan must list exactly one manifest file and it must be planned last",
        ));
    }

    // Declared bounds on originals must already hold before any allocation.
    for page in &session.pages {
        if page.original_bytes > MAX_CAPTURE_BYTES {
            return Err(DomainError::invalid_request(format!(
                "original {} declares {} bytes, over the per-file limit",
                page.capture_id, page.original_bytes
            )));
        }
    }

    // Sources must cover exactly the original/derivative paths, no more.
    let content_paths: HashSet<&str> = plan
        .files
        .iter()
        .filter(|f| {
            matches!(
                f.content_kind,
                ContentKind::Original | ContentKind::Derivative
            )
        })
        .map(|f| f.relative_path.as_str())
        .collect();
    for key in sources.keys() {
        if !content_paths.contains(key.as_str()) {
            return Err(DomainError::invalid_request(format!(
                "export source supplied for unplanned path: {}",
                key
            )));
        }
    }
    for f in plan.files.iter().filter(|f| {
        matches!(
            f.content_kind,
            ContentKind::Original | ContentKind::Derivative
        )
    }) {
        if f.capture_id.is_none() {
            return Err(DomainError::internal(
                "content file without capture_id in plan",
            ));
        }
        if !sources.contains_key(&f.relative_path) {
            return Err(DomainError::invalid_request(format!(
                "missing export source for planned file: {}",
                f.relative_path
            )));
        }
    }
    Ok(session)
}

fn run_plan(
    plan: &ExportPlan,
    session: &ExportSession,
    root: &Path,
    sources: &HashMap<String, PathBuf>,
    manifest: &ExportManifest,
) -> Result<ExportExecution, DomainError> {
    let mut files_written = 0usize;
    let mut manifest_sha256 = String::new();

    for file in &plan.files {
        match file.content_kind {
            ContentKind::Original => {
                let page = page_for(session, file)?;
                let source = source_for(sources, file)?;
                copy_source_verified(
                    root,
                    file,
                    source,
                    &page.original_sha256,
                    page.original_bytes,
                )?;
            }
            ContentKind::Derivative => {
                let page = page_for(session, file)?;
                let (Some(sha), Some(bytes)) =
                    (page.processed_sha256.as_ref(), page.processed_bytes)
                else {
                    return Err(DomainError::internal(
                        "derivative page lacks declared checksum or size",
                    ));
                };
                let source = source_for(sources, file)?;
                copy_source_verified(root, file, source, sha, bytes)?;
            }
            ContentKind::Recipe => {
                let bytes = recipe_document(plan, file)?;
                write_verified(root, file, &bytes)?;
            }
            ContentKind::Manifest => {
                let bytes = manifest_document(manifest)?;
                manifest_sha256 = write_verified(root, file, &bytes)?;
            }
        }
        files_written += 1;
    }

    // Final pass: every planned file must now exist on disk, and staging
    // leftovers must be gone (the protocol's recovery rule ignores `.part`
    // entries; a clean run must leave none).
    for file in &plan.files {
        let final_path = safe_join(root, &file.relative_path)?;
        if !final_path.is_file() {
            return Err(DomainError::internal(format!(
                "planned file missing after finalize: {}",
                file.relative_path
            )));
        }
    }
    assert_no_staged_files(root)?;

    Ok(ExportExecution {
        root: root.to_path_buf(),
        files_written,
        manifest_sha256,
        manifest_digest: manifest.digest(),
    })
}

fn page_for<'a>(
    session: &'a ExportSession,
    file: &PlannedFile,
) -> Result<&'a ExportPage, DomainError> {
    let cid = file
        .capture_id
        .as_deref()
        .ok_or_else(|| DomainError::internal("content file without capture_id in plan"))?;
    session
        .pages
        .iter()
        .find(|p| p.capture_id == cid)
        .ok_or_else(|| {
            DomainError::internal("content file references a capture missing from the session")
        })
}

fn source_for<'a>(
    sources: &'a HashMap<String, PathBuf>,
    file: &PlannedFile,
) -> Result<&'a Path, DomainError> {
    sources
        .get(&file.relative_path)
        .map(|p| p.as_path())
        .ok_or_else(|| {
            DomainError::invalid_request(format!(
                "missing export source for planned file: {}",
                file.relative_path
            ))
        })
}

/// Copy `source` to the file's final planned location through a staged
/// temporary name, verifying declared size, live-stream checksum, and
/// read-back checksum before the atomic rename.
fn copy_source_verified(
    root: &Path,
    file: &PlannedFile,
    source: &Path,
    want_sha: &str,
    want_bytes: u64,
) -> Result<(), DomainError> {
    let meta = fs::metadata(source).map_err(|_| {
        DomainError::storage_unavailable(format!(
            "export source missing or unreadable: {}",
            file.relative_path
        ))
    })?;
    if !meta.is_file() {
        return Err(DomainError::storage_unavailable(format!(
            "export source is not a regular file: {}",
            file.relative_path
        )));
    }
    // Declared size must match *before* reading contents.
    if meta.len() != want_bytes {
        return Err(DomainError::checksum_mismatch(format!(
            "export source {} is {} bytes on disk but the plan declares {}",
            file.relative_path,
            meta.len(),
            want_bytes
        )));
    }

    let mut reader = BufReader::new(fs::File::open(source).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot open export source: {}", kind_of(&e)))
    })?);
    let (mut staged, staged_path) = create_staged(root, file)?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_BUF_BYTES];
    let mut total = 0u64;
    let copy_result: Result<(), DomainError> = loop {
        let n = reader.read(&mut buf).map_err(|e| {
            DomainError::storage_unavailable(format!("cannot read export source: {}", kind_of(&e)))
        })?;
        if n == 0 {
            break Ok(());
        }
        hasher.update(&buf[..n]);
        staged.write_all(&buf[..n]).map_err(|e| {
            DomainError::storage_unavailable(format!("cannot write staged export: {}", kind_of(&e)))
        })?;
        total += n as u64;
        if total > want_bytes {
            break Err(DomainError::checksum_mismatch(format!(
                "export source grew beyond its declared size during copy: {}",
                file.relative_path
            )));
        }
    };
    if let Err(e) = copy_result {
        drop(staged);
        let _ = fs::remove_file(&staged_path);
        return Err(e);
    }
    if total != want_bytes {
        drop(staged);
        let _ = fs::remove_file(&staged_path);
        return Err(DomainError::checksum_mismatch(format!(
            "export source truncated during copy: {}",
            file.relative_path
        )));
    }
    if let Err(e) = staged.flush().and_then(|()| staged.sync_all()) {
        drop(staged);
        let _ = fs::remove_file(&staged_path);
        return Err(DomainError::storage_unavailable(format!(
            "cannot flush staged export: {}",
            kind_of(&e)
        )));
    }
    drop(staged);

    let streamed = hex_digest(&mut hasher);
    if streamed != want_sha {
        let _ = fs::remove_file(&staged_path);
        return Err(DomainError::checksum_mismatch(format!(
            "export source checksum does not match the plan: {}",
            file.relative_path
        )));
    }
    if let Err(e) = verify_staged(&staged_path, want_sha, want_bytes) {
        let _ = fs::remove_file(&staged_path);
        return Err(e);
    }
    finalize_staged(root, file, &staged_path)
}

/// Write in-memory document bytes to the file's planned location through a
/// staged temporary name with read-back checksum verification.
fn write_verified(root: &Path, file: &PlannedFile, bytes: &[u8]) -> Result<String, DomainError> {
    let want_sha = sha256_hex(bytes);
    let (mut staged, staged_path) = create_staged(root, file)?;
    let write = staged
        .write_all(bytes)
        .and_then(|()| staged.flush())
        .and_then(|()| staged.sync_all());
    drop(staged);
    if let Err(e) = write {
        let _ = fs::remove_file(&staged_path);
        return Err(DomainError::storage_unavailable(format!(
            "cannot write staged export document: {}",
            kind_of(&e)
        )));
    }
    if let Err(e) = verify_staged(&staged_path, &want_sha, bytes.len() as u64) {
        let _ = fs::remove_file(&staged_path);
        return Err(e);
    }
    finalize_staged(root, file, &staged_path)?;
    Ok(want_sha)
}

fn create_staged(root: &Path, file: &PlannedFile) -> Result<(fs::File, PathBuf), DomainError> {
    let target = safe_join(root, &file.relative_path)?;
    let parent = target
        .parent()
        .ok_or_else(|| DomainError::internal("planned export path has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot create export directory: {}", kind_of(&e)))
    })?;
    let staged_path = staged_path_for(&target);
    let handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged_path)
        .map_err(|e| {
            DomainError::storage_unavailable(format!("cannot stage export file: {}", kind_of(&e)))
        })?;
    Ok((handle, staged_path))
}

fn staged_path_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!("{}{}", name, PART_SUFFIX))
}

/// Re-open the staged file from disk and confirm it holds exactly the
/// expected digest and length.
fn verify_staged(staged: &Path, want_sha: &str, want_bytes: u64) -> Result<(), DomainError> {
    let (sha, len) = hash_file_limited(staged, want_bytes)?;
    if sha != want_sha || len != want_bytes {
        return Err(DomainError::checksum_mismatch(
            "staged export file failed read-back verification",
        ));
    }
    Ok(())
}

fn finalize_staged(root: &Path, file: &PlannedFile, staged: &Path) -> Result<(), DomainError> {
    let target = safe_join(root, &file.relative_path)?;
    fs::rename(staged, &target).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot finalize export file: {}", kind_of(&e)))
    })
}

fn hash_file_limited(path: &Path, cap: u64) -> Result<(String, u64), DomainError> {
    let file = fs::File::open(path).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot re-read staged export: {}", kind_of(&e)))
    })?;
    let mut reader = BufReader::new(file).take(cap + 1);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; COPY_BUF_BYTES];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            DomainError::storage_unavailable(format!(
                "cannot re-read staged export: {}",
                kind_of(&e)
            ))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex_digest(&mut hasher), total))
}

fn hex_digest(hasher: &mut Sha256) -> String {
    let out = hasher.clone().finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Serialize a referenced recipe document and prove it round-trips through
/// the untrusted-document parser back to the exact recipe it came from.
fn recipe_document(plan: &ExportPlan, file: &PlannedFile) -> Result<Vec<u8>, DomainError> {
    let prefix = format!("{}/", RECIPES_DIR);
    let rest = file
        .relative_path
        .strip_prefix(&prefix)
        .ok_or_else(|| DomainError::internal("recipe path outside the recipes directory"))?;
    let digest = rest
        .strip_suffix(".json")
        .ok_or_else(|| DomainError::internal("recipe path without .json suffix"))?;
    let recipe = plan
        .recipes
        .iter()
        .find(|r| r.digest() == digest)
        .ok_or_else(|| {
            DomainError::invalid_request(format!(
                "recipe document {} is not present in the export plan",
                digest
            ))
        })?;
    let bytes = serde_json::to_vec_pretty(recipe)
        .map_err(|_| DomainError::internal("recipe serialization failed"))?;
    if bytes.len() > MAX_RECIPE_BYTES {
        return Err(DomainError::internal(
            "recipe document exceeds its byte bound after serialization",
        ));
    }
    let back = ProcessingRecipe::from_json_bytes(&bytes)?;
    if back != *recipe {
        return Err(DomainError::internal(
            "recipe document failed read-back round-trip validation",
        ));
    }
    Ok(bytes)
}

/// Serialize the portable manifest and prove it round-trips through the
/// untrusted-document parser before it is ever written.
fn manifest_document(manifest: &ExportManifest) -> Result<Vec<u8>, DomainError> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|_| DomainError::internal("manifest serialization failed"))?;
    if bytes.len() > MAX_EXPORT_MANIFEST_BYTES {
        return Err(DomainError::invalid_request(
            "export manifest exceeds its byte bound after serialization",
        ));
    }
    let back = ExportManifest::from_json_bytes(&bytes)?;
    if back != *manifest {
        return Err(DomainError::internal(
            "manifest failed read-back round-trip validation",
        ));
    }
    Ok(bytes)
}

/// Confirm the finalized tree contains no staged (`.part`) files at any depth.
fn assert_no_staged_files(dir: &Path) -> Result<(), DomainError> {
    let entries = fs::read_dir(dir).map_err(|e| {
        DomainError::storage_unavailable(format!("cannot audit export tree: {}", kind_of(&e)))
    })?;
    for entry in entries {
        let path = entry
            .map_err(|e| {
                DomainError::storage_unavailable(format!(
                    "cannot audit export tree: {}",
                    kind_of(&e)
                ))
            })?
            .path();
        if path.is_dir() {
            assert_no_staged_files(&path)?;
        } else if path
            .file_name()
            .map(|n| n.to_string_lossy().ends_with(PART_SUFFIX))
            .unwrap_or(false)
        {
            return Err(DomainError::internal(
                "leftover staged export file after finalize",
            ));
        }
    }
    Ok(())
}

fn kind_of(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "not-found",
        std::io::ErrorKind::PermissionDenied => "permission-denied",
        std::io::ErrorKind::AlreadyExists => "already-exists",
        _ => "io-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_path_keeps_name_and_appends_suffix() {
        let p = Path::new("/tmp/export/originals/s-1/p1.jpg");
        assert_eq!(
            staged_path_for(p),
            PathBuf::from("/tmp/export/originals/s-1/p1.jpg.foldscan-part")
        );
    }

    #[test]
    fn hex_digest_matches_sha256_hex() {
        let mut h = Sha256::new();
        h.update(b"foldscan");
        assert_eq!(hex_digest(&mut h), sha256_hex(b"foldscan"));
    }
}
