//! Cross-checks for the PNG codec boundary (issue #25).
//!
//! These tests consume fixtures in `tests/fixtures/png/` that were produced
//! by *independent* encoders (libpng via Pillow, zlib via Python, and pypng)
//! and validated with an independent decoder (Pillow/libpng) at generation
//! time — see `generate.py` and `manifest.json` beside the fixtures. That is
//! what distinguishes this file from the crate's own round-trip unit tests:
//! the Rust decoder must read what other implementations actually write,
//! including filtered scanlines and fixed-Huffman deflate blocks those
//! encoders can emit but our own encoder never does.
//!
//! The reverse direction is pinned too: the byte-exact output of our
//! deterministic encoder is committed as `rust_encoded_sample_5x4.png`;
//! Pillow validated that file at generation time, and the test below fails
//! if the encoder's output bytes ever drift.
//!
//! Evidence category: software fixture evidence (third-party encoder and
//! decoder cross-checks on synthetic frames). No device, no optical bench.

use foldscan_domain::png::{decode_png, encode_png};
use foldscan_domain::GrayFrame;
use serde_json::Value;

fn manifest() -> Value {
    let raw = include_str!("fixtures/png/manifest.json");
    serde_json::from_str(raw).expect("fixture manifest parses")
}

fn fixture(name: &str) -> Vec<u8> {
    // include_bytes! needs a literal path, so map names to inclusions.
    match name {
        "pil_noise_32x32_seed7.png" => {
            include_bytes!("fixtures/png/pil_noise_32x32_seed7.png").to_vec()
        }
        "pil_noise_1x40_seed3.png" => {
            include_bytes!("fixtures/png/pil_noise_1x40_seed3.png").to_vec()
        }
        "pil_noise_40x1_seed4.png" => {
            include_bytes!("fixtures/png/pil_noise_40x1_seed4.png").to_vec()
        }
        "pil_flat_5x5.png" => include_bytes!("fixtures/png/pil_flat_5x5.png").to_vec(),
        "pil_gradient_17x5.png" => include_bytes!("fixtures/png/pil_gradient_17x5.png").to_vec(),
        "crafted_filters_11x9.png" => {
            include_bytes!("fixtures/png/crafted_filters_11x9.png").to_vec()
        }
        "crafted_fixed_block_6x6.png" => {
            include_bytes!("fixtures/png/crafted_fixed_block_6x6.png").to_vec()
        }
        "pil_1x1_white.png" => include_bytes!("fixtures/png/pil_1x1_white.png").to_vec(),
        "pil_1x1_black.png" => include_bytes!("fixtures/png/pil_1x1_black.png").to_vec(),
        "rgb_4x4.png" => include_bytes!("fixtures/png/rgb_4x4.png").to_vec(),
        "palette_4x4.png" => include_bytes!("fixtures/png/palette_4x4.png").to_vec(),
        "gray16_4x4.png" => include_bytes!("fixtures/png/gray16_4x4.png").to_vec(),
        "interlaced_8x8.png" => include_bytes!("fixtures/png/interlaced_8x8.png").to_vec(),
        "truncated_idat.png" => include_bytes!("fixtures/png/truncated_idat.png").to_vec(),
        "rust_encoded_sample_5x4.png" => {
            include_bytes!("fixtures/png/rust_encoded_sample_5x4.png").to_vec()
        }
        other => panic!("fixture {other} not wired into include_bytes list"),
    }
}

/// The sample frame pinned by `rust_encoded_sample_5x4.png`.
fn sample_frame() -> GrayFrame {
    let pixels = (0..(5 * 4)).map(|i| (i * 37) as u8).collect();
    GrayFrame::from_pixels(5, 4, pixels).expect("sample frame")
}

#[test]
fn third_party_encoded_fixtures_decode_to_expected_frames() {
    let m = manifest();
    let valid = m["valid"].as_array().expect("valid array");
    assert!(valid.len() >= 8, "fixture set unexpectedly small");
    for entry in valid {
        let name = entry["file"].as_str().unwrap();
        let want_w = entry["w"].as_u64().unwrap() as u32;
        let want_h = entry["h"].as_u64().unwrap() as u32;
        let want_digest = entry["digest"].as_str().unwrap();

        let frame = decode_png(&fixture(name))
            .unwrap_or_else(|e| panic!("{name}: third-party fixture rejected: {e}"));
        assert_eq!(frame.width, want_w, "{name} width");
        assert_eq!(frame.height, want_h, "{name} height");
        assert_eq!(frame.digest(), want_digest, "{name} pixel digest");
    }
}

#[test]
fn unsupported_format_fixtures_are_rejected_without_panics() {
    let m = manifest();
    let reject = m["reject"].as_array().expect("reject array");
    assert!(reject.len() >= 4, "reject fixture set unexpectedly small");
    for entry in reject {
        let name = entry["file"].as_str().unwrap();
        let result = std::panic::catch_unwind(|| decode_png(&fixture(name)));
        let err = match result {
            Err(_) => panic!("{name}: decoder panicked on malformed input"),
            Ok(Ok(frame)) => panic!("{name}: decoder accepted it ({:?})", frame.digest()),
            Ok(Err(e)) => e,
        };
        assert_eq!(
            err.category,
            foldscan_domain::Category::InvalidRequest,
            "{name}: unexpected rejection category"
        );
    }
}

#[test]
fn encoder_output_is_byte_stable_against_pillow_validated_sample() {
    // `rust_encoded_sample_5x4.png` is the committed, Pillow-validated
    // output of this encoder on `sample_frame()`. Any byte drift (framing,
    // filter bytes, CRC, deflate wrapping) fails this test and means users'
    // stored derivative digests would change.
    let out = encode_png(&sample_frame()).expect("encode sample");
    assert_eq!(
        out,
        fixture("rust_encoded_sample_5x4.png"),
        "encoder output drifted from the committed, independently decoded sample"
    );
    // And our own decoder must of course read it back identically.
    assert_eq!(decode_png(&out).expect("decode own sample"), sample_frame());
}

#[test]
fn idat_split_across_chunks_decodes() {
    // Build a valid PNG with our encoder, then re-wrap its single IDAT into
    // three chunks. Multi-IDAT is legal and common; the decoder must accept
    // it. We rewrite chunk framing while preserving every chunk CRC by
    // recomputing nothing: each new chunk gets its own correct CRC here.
    let png = encode_png(&sample_frame()).expect("encode");
    // Locate the single IDAT chunk (after signature+IHDR).
    let ihdr_len = u32::from_be_bytes([png[8], png[9], png[10], png[11]]) as usize;
    let idat_off = 8 + 12 + ihdr_len; // 8 sig + IHDR(12+len)
    assert_eq!(&png[idat_off + 4..idat_off + 8], b"IDAT");
    let idat_len = u32::from_be_bytes([
        png[idat_off],
        png[idat_off + 1],
        png[idat_off + 2],
        png[idat_off + 3],
    ]) as usize;
    let idat_data = &png[idat_off + 8..idat_off + 8 + idat_len];
    let iend = &png[idat_off + 8 + idat_len + 4..]; // IEND chunk bytes

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }
    let mut out = png[..idat_off].to_vec();
    let parts = [
        &idat_data[..idat_data.len() / 3],
        &idat_data[idat_data.len() / 3..2 * idat_data.len() / 3],
        &idat_data[2 * idat_data.len() / 3..],
    ];
    for part in parts {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(b"IDAT");
        out.extend_from_slice(part);
        let mut crc_in = b"IDAT".to_vec();
        crc_in.extend_from_slice(part);
        out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
    }
    out.extend_from_slice(iend);

    assert_eq!(decode_png(&out).expect("multi-IDAT decode"), sample_frame());
}
