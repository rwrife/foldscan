//! Deterministic single-file PDF writer embedding grayscale pages as image
//! XObjects (issue #27, part of #5).
//!
//! The companion's export path produces ordered processed page sets; a
//! portable single-file document is the next export boundary on the
//! acceptance list. This module serializes pages into a PDF 1.4 file where
//! each page is one full-page `/XObject /Image` sample image:
//!
//! - `/ColorSpace /DeviceGray`, `/BitsPerComponent 8` — the frame payload
//!   is wrapped in a zlib stream of deflate *stored* blocks. A zlib stream
//!   is exactly what PDF `/FlateDecode` expects (ISO 32000-1 §7.4.4, which
//!   cites RFC 1950), so no new codec work is needed: the writer reuses the
//!   PNG module's already-tested `zlib_stored` and self-verifies every
//!   embedded stream by re-inflating it (`zlib_decode`) before returning.
//! - 1 pixel = 1 PostScript point; each page's MediaBox equals its frame
//!   size and pages appear in input order.
//! - No compression tuning (stored blocks only, same stance as the PNG
//!   encoder), no text, no reading/parsing of PDFs, no encryption.
//!
//! Allocation discipline (hostile-input style, even though frames are
//! host-supplied): every frame is bounds-checked with the same rules as
//! `GrayFrame` *before* the output buffer grows (this also catches
//! hand-mutated `GrayFrame` structs), the page count is capped at
//! `MAX_PAGES_PER_DOCUMENT`, and a conservative upper bound on the final
//! document size (`MAX_DOCUMENT_BYTES`) is checked *before* assembly
//! allocates. Stream lengths and byte offsets must fit u32 with checked
//! arithmetic.
//!
//! Determinism: identical inputs produce byte-identical output — object
//! numbers, dictionary spellings, and the fixed file `/ID` are all frozen,
//! and the two golden tests pin the exact output digests.
//!
//! Self-check: after assembly, `check_document` re-parses the finished
//! bytes and verifies the header, `%%EOF`, trailer `/Size` and `/Root`,
//! `/Pages /Count`, that `startxref` lands on the `xref` keyword, that the
//! free entry and every in-use `xref` entry has the right width/type and
//! points exactly at its `<n> 0 obj` header, and that every embedded image
//! stream re-inflates to the exact source frame payload. A failure means
//! the writer produced a corrupt file: callers get `Category::InternalError`
//! and no bytes. (Byte-exact dictionary/body text is pinned separately by
//! the golden digests, not re-derived here.)
//!
//! Evidence category: software code verified by unit tests plus committed
//! fixture PDFs validated at generation time with an independent parser
//! (PyMuPDF — page count, per-page pixel dimensions, and per-page image
//! pixel digests; see `tests/fixtures/pdf/generate.py` and
//! `evidence/pdf-export.md`). Not device evidence, not a print-production
//! claim, and not a claim of full ISO 32000-1 conformance beyond the subset
//! those checks exercise.

use crate::error::DomainError;
use crate::image::{check_bounds, GrayFrame, MAX_FRAME_PIXELS};
use crate::limits::MAX_CAPTURE_BYTES;
use crate::png::{zlib_decode, zlib_stored};

/// Maximum pages in one exported document. Sized to match the protocol's
/// per-session capture bound; beyond this a document is a user-error, not a
/// silent truncation.
pub const MAX_PAGES_PER_DOCUMENT: usize = crate::limits::MAX_CAPTURES_PER_SESSION;

/// Conservative upper bound on one generated document (128 MiB). Checked
/// against a per-object overhead estimate plus worst-case stored-stream
/// framing *before* the document buffer is built. A single frame may still
/// declare up to `MAX_CAPTURE_BYTES`, so this bounds multi-page documents,
/// not single pages.
pub const MAX_DOCUMENT_BYTES: u64 = 128 * 1024 * 1024;

/// Fixed file identifier (`/ID` in the trailer). Kept constant so output is
/// byte-deterministic; the export executor's SHA-256 is the artifact's real
/// identity.
const FIXED_ID: &str = "00000000000000000000000000000000";

/// Objects per page: Page dict, Contents stream, and one combined image
/// dictionary + stream object — three per page, plus Catalog and Pages tree.
fn object_count(pages: usize) -> usize {
    pages * 3 + 2
}

/// Upper bound on the non-image bytes one page can contribute (page dict,
/// content stream, image dict text, xref entry). Deliberately loose; it
/// exists only so the pre-assembly size bound cannot be bypassed.
const OVERHEAD_PER_PAGE: u64 = 1024;

/// Worst-case size of `zlib_stored` output for `raw_len` bytes: the
/// encoder's capacity formula plus slack (2-byte header, ≤5 bytes framing
/// per 64 KiB block, 4-byte Adler). `raw_len` is already capped at
/// `MAX_CAPTURE_BYTES`, so no overflow is possible.
fn stored_stream_bound(raw_len: u64) -> u64 {
    raw_len + raw_len / 65_535 + 64
}

/// Object numbering: 1 Catalog, 2 Pages, then per page i (0-based):
/// Page `3+3i`, Contents `4+3i`, image `5+3i`.
fn page_obj_num(i: usize) -> usize {
    3 + i * 3
}

/// Per-page data the self-check needs to re-verify the finished bytes.
struct PageCheck<'a> {
    w: u32,
    h: u32,
    p_len: u32,
    s_len: u32,
    payload: &'a [u8],
}

/// Export `pages` as a PDF 1.4 document with one image page per frame, in
/// order. See the module docs for format and bound decisions.
pub fn export_pdf(pages: &[GrayFrame]) -> Result<Vec<u8>, DomainError> {
    // Pass 1: validate every frame and bound the total size *before*
    // assembly allocates anything proportional to the input.
    if pages.is_empty() {
        return Err(DomainError::invalid_request(
            "pdf: document must contain at least one page",
        ));
    }
    if pages.len() > MAX_PAGES_PER_DOCUMENT {
        return Err(DomainError::invalid_request(format!(
            "pdf: document declares {} pages, over the {MAX_PAGES_PER_DOCUMENT} limit",
            pages.len()
        )));
    }
    let mut total: u64 = 0;
    for page in pages {
        check_bounds(page.width, page.height)?;
        let expected = (page.width as u64)
            .checked_mul(page.height as u64)
            .ok_or_else(|| DomainError::invalid_request("pdf: frame pixel count overflows"))?;
        if expected > MAX_FRAME_PIXELS {
            return Err(DomainError::invalid_request(format!(
                "pdf: frame declares {expected} pixels, over the {MAX_FRAME_PIXELS} limit"
            )));
        }
        if page.pixels.len() as u64 != expected {
            return Err(DomainError::invalid_request(format!(
                "pdf: frame payload has {} bytes, expected {} for {}x{}",
                page.pixels.len(),
                expected,
                page.width,
                page.height
            )));
        }
        if expected > MAX_CAPTURE_BYTES {
            return Err(DomainError::invalid_request(format!(
                "pdf: frame payload {expected} bytes exceeds {MAX_CAPTURE_BYTES}"
            )));
        }
        total = total
            .checked_add(stored_stream_bound(expected) + OVERHEAD_PER_PAGE)
            .ok_or_else(|| DomainError::invalid_request("pdf: size bound overflows"))?;
    }
    if total > MAX_DOCUMENT_BYTES {
        return Err(DomainError::invalid_request(format!(
            "pdf: document would exceed the {MAX_DOCUMENT_BYTES} byte bound"
        )));
    }

    // Compress every page's payload once and record self-check data.
    let n_pages = pages.len();
    let n_objs = object_count(n_pages);
    let n_total = n_objs + 1; // + the free xref entry
    let mut streams = Vec::with_capacity(n_pages);
    let mut checks = Vec::with_capacity(n_pages);
    for page in pages {
        let stream = zlib_stored(&page.pixels);
        let s_len = u32::try_from(stream.len())
            .map_err(|_| DomainError::internal("pdf: compressed stream exceeds u32 length"))?;
        let p_len = u32::try_from(page.pixels.len())
            .map_err(|_| DomainError::internal("pdf: payload exceeds u32 length"))?;
        checks.push(PageCheck {
            w: page.width,
            h: page.height,
            p_len,
            s_len,
            payload: &page.pixels,
        });
        streams.push(stream);
    }

    // Pass 2: assemble the file with running byte offsets.
    let mut out: Vec<u8> = Vec::with_capacity((total as usize).min(16 * 1024 * 1024));
    out.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
    let mut offsets = vec![0u32; n_objs + 1];

    let mark = |out: &[u8]| -> Result<u32, DomainError> {
        u32::try_from(out.len()).map_err(|_| DomainError::internal("pdf: offsets exceed u32"))
    };

    offsets[1] = mark(&out)?;
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets[2] = mark(&out)?;
    out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Count ");
    out.extend_from_slice(n_pages.to_string().as_bytes());
    out.extend_from_slice(b" /Kids [");
    for i in 0..n_pages {
        out.extend_from_slice(page_obj_num(i).to_string().as_bytes());
        out.extend_from_slice(b" 0 R ");
    }
    out.extend_from_slice(b"] >>\nendobj\n");

    for (i, chk) in checks.iter().enumerate() {
        let page_num = page_obj_num(i);
        let content_num = page_num + 1;
        let image_num = page_num + 2;

        offsets[page_num] = mark(&out)?;
        out.extend_from_slice(format!(
            "{page_num} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << /Im0 {im} 0 R >> >> /Contents {c} 0 R >>\nendobj\n",
            w = chk.w,
            h = chk.h,
            im = image_num,
            c = content_num
        )
        .as_bytes());

        // Content is a stream object: build its body first so /Length is
        // exact, then wrap. (It draws the page's image XObject scaled to
        // the MediaBox.)
        offsets[content_num] = mark(&out)?;
        let body = format!("q\n1 0 0 {} 0 0 cm\n/Im0 Do\nQ", chk.h);
        out.extend_from_slice(
            format!(
                "{content_num} 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                body.len(),
                body
            )
            .as_bytes(),
        );

        offsets[image_num] = mark(&out)?;
        out.extend_from_slice(format!(
            "{image_num} 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {len} >>\nstream\n",
            w = chk.w,
            h = chk.h,
            len = chk.s_len
        )
        .as_bytes());
        out.extend_from_slice(&streams[i]);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    let xref_off =
        u32::try_from(out.len()).map_err(|_| DomainError::internal("pdf: offsets exceed u32"))?;
    out.extend_from_slice(format!("xref\n0 {n_total}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {n_total} /Root 1 0 R /ID [<{}> <{}>] >>\nstartxref\n{xref_off}\n%%EOF\n",
            FIXED_ID, FIXED_ID
        )
        .as_bytes(),
    );

    check_document(&out, &checks, n_total).map_err(|e| {
        DomainError::internal(format!("pdf writer self-check failed: {}", e.message))
    })?;
    Ok(out)
}

/// Locate every occurrence of `needle`; error unless it occurs exactly `k`
/// times.
fn find_only(doc: &[u8], needle: &[u8], k: usize) -> Result<Vec<usize>, DomainError> {
    let mut hits = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= doc.len() {
        match doc[i..].windows(needle.len()).position(|w| w == needle) {
            Some(rel) => {
                hits.push(i + rel);
                i += rel + 1;
                if hits.len() > k {
                    break;
                }
            }
            None => break,
        }
    }
    if hits.len() != k {
        return Err(DomainError::internal(format!(
            "self-check: pattern {:?} occurs {} times, expected {k}",
            String::from_utf8_lossy(needle),
            hits.len()
        )));
    }
    Ok(hits)
}

/// Parse the first integer in `doc[at..at+label.len()+64]`, requiring
/// `label` at the start and skipping any non-digit bytes after it.
fn number_after(doc: &[u8], at: usize, label: &str) -> Result<u64, DomainError> {
    let window = doc
        .get(at..doc.len().min(at + label.len() + 64))
        .ok_or_else(|| DomainError::internal(format!("self-check: {label} slice empty")))?;
    if &window[..label.len()] != label.as_bytes() {
        return Err(DomainError::internal(format!(
            "self-check: expected {label} at {at}"
        )));
    }
    let mut i = label.len();
    while i < window.len() && !window[i].is_ascii_digit() {
        i += 1;
    }
    let start = i;
    while i < window.len() && window[i].is_ascii_digit() {
        i += 1;
    }
    if start == i {
        return Err(DomainError::internal(format!(
            "self-check: no number after {label}"
        )));
    }
    std::str::from_utf8(&window[start..i])
        .map_err(|_| DomainError::internal("self-check: non-utf8 digits"))?
        .parse::<u64>()
        .map_err(|_| DomainError::internal("self-check: number overflow"))
}

/// Re-parse the finished document and verify its structure and payload
/// fidelity. Any mismatch is a writer bug: return an error, emit nothing.
fn check_document(doc: &[u8], checks: &[PageCheck<'_>], n_total: usize) -> Result<(), DomainError> {
    if !doc.starts_with(b"%PDF-1.4\n") || !doc.ends_with(b"%%EOF\n") {
        return Err(DomainError::internal("self-check: header/EOF missing"));
    }
    let n_pages = checks.len();
    let n_objs = object_count(n_pages);

    // Trailer: /Size and /Root.
    let trailer_at = find_only(doc, b"trailer\n<< ", 1)?[0];
    if number_after(doc, trailer_at, "trailer\n<< /Size")? != n_total as u64 {
        return Err(DomainError::internal("self-check: /Size mismatch"));
    }
    let trailer_span = doc
        .get(trailer_at..doc.len().min(trailer_at + 256))
        .ok_or_else(|| DomainError::internal("self-check: trailer slice empty"))?;
    if !trailer_span
        .windows(b"/Root 1 0 R".len())
        .any(|w| w == b"/Root 1 0 R")
    {
        return Err(DomainError::internal("self-check: trailer /Root missing"));
    }

    // Pages tree: /Count.
    let pages_at = find_only(doc, b"2 0 obj\n<< /Type /Pages /Count ", 1)?[0];
    if number_after(doc, pages_at + b"2 0 obj\n".len(), "<< /Type /Pages /Count")? != n_pages as u64
    {
        return Err(DomainError::internal("self-check: /Count mismatch"));
    }

    // xref header lands exactly at startxref.
    let sx_at = find_only(doc, b"startxref", 1)?[0];
    let xref_off = number_after(doc, sx_at, "startxref")? as usize;
    let header = format!("xref\n0 {n_total}\n");
    if !doc
        .get(xref_off..)
        .is_some_and(|r| r.starts_with(header.as_bytes()))
    {
        return Err(DomainError::internal("self-check: startxref misplaced"));
    }

    // Every xref entry: fixed 20-byte width, correct type, and (for in-use
    // objects) an offset that points exactly at that object's header.
    let entries_at = xref_off + header.len();
    let table = doc
        .get(entries_at..entries_at + n_total * 20)
        .ok_or_else(|| DomainError::internal("self-check: xref table overruns file"))?;
    let mut obj_offsets = vec![0usize; n_objs + 1];
    for (i, entry) in table.as_chunks::<20>().0.iter().enumerate() {
        // Entry layout (PDF 32000-1 Table 18, 20 bytes): [0..10) offset,
        // space, [11..16) generation, space, [17] type ('f'/'n'), [18..20) EOL.
        if i == 0 {
            if &entry[..19] != b"0000000000 65535 f " {
                return Err(DomainError::internal("self-check: free entry malformed"));
            }
            continue;
        }
        if entry[17] != b'n' {
            return Err(DomainError::internal(format!(
                "self-check: entry {i} not in-use"
            )));
        }
        let off: usize = std::str::from_utf8(&entry[0..10])
            .map_err(|_| DomainError::internal("self-check: xref offset not utf8"))?
            .parse()
            .map_err(|_| DomainError::internal("self-check: xref offset malformed"))?;
        obj_offsets[i] = off;
        let want = format!("{i} 0 obj");
        if !doc
            .get(off..off + want.len())
            .is_some_and(|w| w == want.as_bytes())
        {
            return Err(DomainError::internal(format!(
                "self-check: xref offset for object {i} does not point at its header"
            )));
        }
    }

    // Per page: image stream payload re-inflates to the exact source frame.
    for (i, chk) in checks.iter().enumerate() {
        let image_num = page_obj_num(i) + 2;
        let image_at = obj_offsets[image_num];
        let marker = doc
            .get(image_at..doc.len().min(image_at + 600))
            .ok_or_else(|| DomainError::internal("self-check: image object slice empty"))?;
        let want_dict = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            chk.w, chk.h, chk.s_len
        );
        let header_len = format!("{image_num} 0 obj\n").len();
        if !marker
            .get(header_len..header_len + want_dict.len())
            .is_some_and(|w| w == want_dict.as_bytes())
        {
            return Err(DomainError::internal(format!(
                "self-check: image object {image_num} dictionary mismatch"
            )));
        }
        let payload_start = image_at + header_len + want_dict.len();
        let payload_end = payload_start + chk.s_len as usize;
        if doc.get(payload_end..payload_end + b"\nendstream\nendobj".len())
            != Some(&b"\nendstream\nendobj"[..])
        {
            return Err(DomainError::internal(
                "self-check: image stream footer mismatch",
            ));
        }
        let inflated =
            zlib_decode(&doc[payload_start..payload_end], chk.p_len as usize).map_err(|e| {
                DomainError::internal(format!(
                    "self-check: stream re-inflate failed: {}",
                    e.message
                ))
            })?;
        if inflated != chk.payload {
            return Err(DomainError::internal(
                "self-check: decoded image bytes differ from source frame",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::sha256_hex;
    use crate::error::Category;

    fn lcg_page(w: u32, h: u32, seed: u16) -> GrayFrame {
        // Same frozen 16-bit LCG as tests/golden_processing.rs and the PDF
        // fixture generator.
        struct Lcg(u16);
        impl Lcg {
            fn next(&mut self) -> u8 {
                self.0 = self
                    .0
                    .wrapping_mul(25173)
                    .wrapping_add(13849)
                    .wrapping_shl(3)
                    ^ self.0.wrapping_shr(5);
                (self.0 >> 8) as u8
            }
        }
        let mut rng = Lcg(seed);
        let pixels = (0..(w * h)).map(|_| rng.next()).collect();
        GrayFrame::from_pixels(w, h, pixels).expect("fixture frame")
    }

    #[test]
    fn empty_document_is_rejected() {
        let err = export_pdf(&[]).unwrap_err();
        assert_eq!(err.category, Category::InvalidRequest);
    }

    #[test]
    fn oversized_single_page_is_rejected_before_allocating() {
        // Declares 64,008,001 pixels with an empty buffer: the pixel-count
        // gate must fire before anything materializes.
        let bad = GrayFrame {
            width: 8001,
            height: 8001,
            pixels: Vec::new(),
        };
        let err = export_pdf(std::slice::from_ref(&bad)).unwrap_err();
        assert_eq!(err.category, Category::InvalidRequest);
    }

    #[test]
    fn mismatched_payload_is_rejected() {
        let mut bad = lcg_page(4, 3, 1);
        bad.pixels.truncate(11);
        let err = export_pdf(std::slice::from_ref(&bad)).unwrap_err();
        assert_eq!(err.category, Category::InvalidRequest);
    }

    #[test]
    fn zero_dimension_frame_is_rejected() {
        let bad = GrayFrame {
            width: 4,
            height: 0,
            pixels: Vec::new(),
        };
        let err = export_pdf(std::slice::from_ref(&bad)).unwrap_err();
        assert_eq!(err.category, Category::InvalidRequest);
    }

    #[test]
    fn page_count_bound_is_enforced() {
        // Reuse one tiny frame MAX+1 times: the count gate fires even
        // though the byte bound would never be reached.
        let tiny = lcg_page(1, 1, 9);
        let pages = vec![tiny; MAX_PAGES_PER_DOCUMENT + 1];
        let err = export_pdf(&pages).unwrap_err();
        assert_eq!(err.category, Category::InvalidRequest);
    }

    #[test]
    fn total_bound_is_enforced() {
        // Three frames at the per-frame pixel maximum (64,000,000 px) pass
        // every per-frame gate but exceed MAX_DOCUMENT_BYTES.
        let big = GrayFrame {
            width: 8_000,
            height: 8_000,
            pixels: vec![7u8; 64_000_000],
        };
        let pages = vec![big; 3];
        let err = export_pdf(&pages).unwrap_err();
        assert_eq!(err.category, Category::InvalidRequest);
    }

    #[test]
    fn output_is_deterministic_and_single_page_digest_pinned() {
        let a = export_pdf(&[lcg_page(5, 4, 11)]).expect("one-page doc");
        let b = export_pdf(&[lcg_page(5, 4, 11)]).expect("again");
        assert_eq!(a, b);
        assert_eq!(
            sha256_hex(&a),
            "9413e09ab8b7762480862ada3bca909e04175184d8ece6cf9a6d00f220d31bc3",
            "deterministic 1-page output digest drifted"
        );
    }

    #[test]
    fn three_page_digest_pinned_and_structurally_valid() {
        let doc = export_pdf(&[lcg_page(16, 12, 1), lcg_page(11, 7, 2), lcg_page(40, 1, 3)])
            .expect("three-page doc");
        assert_eq!(
            sha256_hex(&doc),
            "ca6b12880167c815e1fc53eee9af5c55f18a5b0de3a320f58d1bb83720c12772",
            "deterministic 3-page output digest drifted"
        );
        assert!(doc.starts_with(b"%PDF-1.4\n"));
        assert!(doc.ends_with(b"%%EOF\n"));
        // 11 objects (3*3+2) plus the free entry in the xref header line.
        assert!(doc.windows(13).any(|w| w.starts_with(b"xref\n0 12\n")));
        assert!(doc.windows(10).any(|w| w.starts_with(b"/Count 3 /")));
        let sx = doc
            .windows(b"startxref".len())
            .rposition(|w| w == b"startxref")
            .unwrap();
        let xref_off: usize = String::from_utf8_lossy(&doc[sx + b"startxref".len()..])
            .trim_start()
            .lines()
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(doc[xref_off..].starts_with(b"xref\n"));
        assert!(doc.windows(12).any(|w| w.starts_with(b"<< /Size 12")));
    }
}
