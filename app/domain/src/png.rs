//! Bounded PNG encode/decode boundary for the grayscale frame model.
//!
//! The processing core (`image.rs`, `processing.rs`) works on raw
//! `GrayFrame` pixels and the export executor treats derivative bytes as
//! opaque. This module is the codec boundary between those raw frames and
//! PNG bytes — the format the export manifest uses by default (an omitted
//! processed media type means PNG per `docs/protocol.md`).
//!
//! Scope decisions (issue #25):
//! - **Encoder**: 8-bit non-interlaced grayscale only (color type 0,
//!   filter type 0 per scanline), deflate *stored* blocks. Output is small,
//!   spec-valid, and deterministic; compression is a later optimization.
//! - **Decoder**: baseline subset only — 8-bit, non-interlaced, color
//!   type 0. Color types 2/3/4/6, bit depths other than 8, and interlacing
//!   are rejected with a stable `InvalidRequest`, not guessed at. All five
//!   PNG scanline filters and all three deflate block types (stored, fixed
//!   Huffman, dynamic Huffman) are supported, across multiple IDAT chunks.
//! - **No new dependencies**: std-only, matching the crate convention.
//!
//! Untrusted-input discipline (the decoder is the first consumer of
//! device-supplied image bytes):
//! - IHDR dimensions are bounds-checked with the same pre-allocation limits
//!   as `GrayFrame` before anything is decompressed;
//! - the inflate destination is fixed at `height * (width + 1)` bytes from
//!   IHDR — the decoder never grows a buffer in response to stream content;
//! - total compressed bytes are capped at `MAX_CAPTURE_BYTES`;
//! - truncated streams, bad CRC/Adler-32, malformed Huffman tables, unknown
//!   critical chunks, and misplaced chunks are rejected with `DomainError`
//!   categories; no unbounded panics on malformed input.
//!
//! Evidence category: software code, verified by unit tests, third-party
//! generated fixtures (independently encoded PNGs decoded here, validated
//! with an independent decoder at generation time — see
//! `tests/fixtures/png/generate.py`) and committed expected digests. Not
//! device evidence, not optical-bench evidence, and *not* JPEG — JPEG
//! remains out of scope.

use crate::error::DomainError;
use crate::image::{check_bounds, GrayFrame};
use crate::limits::MAX_CAPTURE_BYTES;

/// PNG 8-byte file signature (ISO/IEC 15948 §5.2).
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Soft bound on total IDAT payload (compressed bytes) per image, reusing
/// the capture-file bound. Bounds are checked before decompression so the
/// inflated size is already capped by IHDR.
const MAX_IDAT_TOTAL: u64 = MAX_CAPTURE_BYTES;

// ---------------------------------------------------------------------------
// CRC-32 (PNG chunk checksum, polynomial 0xEDB88320) and Adler-32 (zlib).
// ---------------------------------------------------------------------------

/// CRC-32 as used by PNG chunks (ISO/IEC 15948 §5.3), IEEE polynomial.
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

/// Adler-32 as used by zlib streams (RFC 1950 §2.2), modulo 65521.
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

fn push_u32_be(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

/// Append one PNG chunk: length, type, data, CRC(type||data).
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    push_u32_be(out, data.len() as u32);
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    push_u32_be(out, crc32(&crc_input));
}

/// Wrap raw bytes in a zlib stream (RFC 1950) using deflate *stored*
/// blocks (RFC 1952 §3.2.4): no compression, no matches, trivially
/// decodable by any implementation, and easy to verify exactly.
///
/// Crate-visible so the PDF writer (`pdf.rs`) can embed uncompressed
/// image streams through the same already-tested stored-deflate path.
pub(crate) fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 + 11);
    // CMF = deflate (8) with 32K window (CINFO=7); FLG chosen so CMF*256+FLG
    // is a multiple of 31 (FCHECK), FDICT=0.
    out.push(0x78);
    out.push(0x01);
    let mut pos = 0usize;
    loop {
        let take = (data.len() - pos).min(0xffff);
        let (before, rest) = data.split_at(pos + take);
        let chunk = &before[pos..];
        let is_last = rest.is_empty();
        out.push(u8::from(is_last));
        // LEN and NLEN are 16-bit little-endian (RFC 1951 §3.2.4).
        let len = chunk.len() as u16;
        let nlen = !len;
        out.push((len & 0xff) as u8);
        out.push((len >> 8) as u8);
        out.push((nlen & 0xff) as u8);
        out.push((nlen >> 8) as u8);
        out.extend_from_slice(chunk);
        pos += take;
        if is_last {
            break;
        }
    }
    push_u32_be(&mut out, adler32(data));
    out
}

/// Encode a frame as an 8-bit non-interlaced grayscale PNG (color type 0).
///
/// Deterministic: the same bytes always produce the same PNG. Every scanline
/// uses PNG filter type 0 (None) and the pixel data is wrapped in deflate
/// stored blocks, so output size is roughly raw size + ~0.02% framing.
/// Compression (real Huffman coding) is deliberately out of scope for this
/// slice — correctness of the container and of the *decode* side matters
/// more for untrusted-device handling, and derivative sizes are already
/// capped at 64 MiB by the export layer.
pub fn encode_png(frame: &GrayFrame) -> Result<Vec<u8>, DomainError> {
    // Frames are constructed bounds-checked, but re-check: this function is
    // public and takes any GrayFrame field shape.
    check_bounds(frame.width, frame.height)?;

    let mut raw = Vec::with_capacity(frame.height as usize * (frame.width as usize + 1));
    for y in 0..frame.height as usize {
        raw.push(0u8); // filter type: None
        let start = y * frame.width as usize;
        raw.extend_from_slice(&frame.pixels[start..start + frame.width as usize]);
    }

    let mut out = Vec::with_capacity(raw.len() + 64);
    out.extend_from_slice(&PNG_SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    push_u32_be(&mut ihdr, frame.width);
    push_u32_be(&mut ihdr, frame.height);
    ihdr.push(8); // bit depth
    ihdr.push(0); // color type: grayscale
    ihdr.push(0); // compression method: deflate
    ihdr.push(0); // filter method: adaptive (per-scanline filters exist; we use 0)
    ihdr.push(0); // interlace: none
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut out, b"IEND", b"");
    Ok(out)
}

// ---------------------------------------------------------------------------
// Inflate (zlib + RFC 1951) — bounded, std-only
// ---------------------------------------------------------------------------

/// A bit reader over a byte slice, LSB-first per RFC 1951 §3.1.
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u64,
    bit_cnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    /// Pull more bytes into the bit buffer (holds at most 56 bits).
    fn refill(&mut self) {
        while self.bit_cnt <= 55 && self.byte_pos < self.data.len() {
            self.bit_buf |= (self.data[self.byte_pos] as u64) << self.bit_cnt;
            self.byte_pos += 1;
            self.bit_cnt += 8;
        }
    }

    /// Read `n` bits (LSB-first, i.e. in stream order). Errors only on a
    /// truncated stream.
    fn read_bits(&mut self, n: u32) -> Result<u32, DomainError> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(DomainError::invalid_request("inflate: absurd bit width"));
        }
        self.refill();
        if self.bit_cnt < n {
            return Err(DomainError::invalid_request("inflate: truncated bitstream"));
        }
        let v = (self.bit_buf & ((1u64 << n) - 1)) as u32;
        self.bit_buf >>= n;
        self.bit_cnt -= n;
        Ok(v)
    }

    /// Discard bits until the next byte boundary.
    fn align_to_byte(&mut self) {
        let drop = self.bit_cnt % 8;
        self.bit_buf >>= drop;
        self.bit_cnt -= drop;
    }
}

/// The classic "inflate95" reversed-code leaf table: a flat lookup indexed
/// by the reversal of the received bits, holding `(symbol << 4) | len` or
/// `INVALID`. Unmatched prefixes are detected at decode time (a walk longer
/// than `max_len`, or a leaf whose length differs from the walk length), so
/// incomplete-but-structurally-sound code sets are tolerated exactly as much
/// as decoding them can be; over-subscribed sets are rejected at build.
const LEAF_INVALID: u32 = 0xFFFF_FFFF;

#[derive(Debug)]
struct HuffTable {
    leaves: Vec<u32>,
    max_len: u32,
}

impl HuffTable {
    /// Build canonical codes from per-symbol code lengths (RFC 1951 §3.2).
    fn new(lengths: &[u8]) -> Result<Self, DomainError> {
        let mut counts = [0i32; 16];
        let mut max_len = 0u32;
        for &l in lengths {
            if l > 15 {
                return Err(DomainError::invalid_request("inflate: code length over 15"));
            }
            counts[l as usize] += 1;
            if u32::from(l) > max_len {
                max_len = u32::from(l);
            }
        }
        if max_len == 0 {
            // Empty table: any decode attempt must fail (matches zlib's
            // "invalid stored block / incomplete set" behavior).
            return Ok(Self {
                leaves: Vec::new(),
                max_len: 0,
            });
        }
        // Over-subscription check: Kraft sum must not exceed 1. An
        // incomplete set is decodable as long as invalid prefixes are
        // detected during the walk, which is exactly what `decode` does.
        let mut left = 1i32;
        for l in 1..=max_len {
            left *= 2;
            left -= counts[l as usize];
            if left < 0 {
                return Err(DomainError::invalid_request(
                    "inflate: over-subscribed Huffman code lengths",
                ));
            }
        }

        // Symbols in canonical order: sorted by (length, symbol value).
        let mut symbols = vec![0u16; lengths.len()];
        let mut offs = [0i32; 16];
        for l in 1..max_len {
            offs[(l + 1) as usize] = offs[l as usize] + counts[l as usize];
        }
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }

        // Canonical code assignment, then reversed-code leaf expansion.
        let mut leaves = vec![LEAF_INVALID; 1usize << max_len];
        let mut code: u32 = 0; // next canonical code (MSB-first value)
        let mut idx = 0usize;
        for l in 1..=max_len {
            for _ in 0..counts[l as usize] {
                let sym = symbols[idx] as u32;
                idx += 1;
                // Reverse the low `l` bits of `code`.
                let mut rev = 0u32;
                let mut v = code;
                for _ in 0..l {
                    rev = (rev << 1) | (v & 1);
                    v >>= 1;
                }
                let mut k = rev as usize;
                while k < leaves.len() {
                    leaves[k] = (sym << 4) | l;
                    k += 1usize << l;
                }
                code += 1;
            }
            code <<= 1; // canonical base for the next length
        }
        Ok(Self { leaves, max_len })
    }

    /// Decode one symbol: walk bit by bit (first received bit is the code's
    /// most-significant bit) until the accumulated reversed bits hit a leaf
    /// whose stored length equals the walked length.
    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, DomainError> {
        if self.max_len == 0 {
            return Err(DomainError::invalid_request(
                "inflate: decode from empty table",
            ));
        }
        let mut code = 0usize;
        let mut len = 0u32;
        loop {
            len += 1;
            if len > self.max_len {
                return Err(DomainError::invalid_request("inflate: bad Huffman code"));
            }
            let bit = br
                .read_bits(1)
                .map_err(|_| DomainError::invalid_request("inflate: truncated Huffman code"))?;
            code = (code << 1) | (bit as usize);
            // Leaf index is the reversal of the walked code.
            let mut rev = 0usize;
            let mut v = code;
            for _ in 0..len {
                rev = (rev << 1) | (v & 1);
                v >>= 1;
            }
            let leaf = self.leaves[rev & (self.leaves.len() - 1)];
            if leaf != LEAF_INVALID && (leaf & 0xF) == len {
                return Ok((leaf >> 4) as u16);
            }
        }
    }
}

/// Length symbols 257..285: (base, extra bits), RFC 1951 §3.2.5.
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Distance symbols 0..29: (base, extra bits), RFC 1951 §3.2.5.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// Permutation order of the code-length alphabet (RFC 1951 §3.2.7).
const CLORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn fixed_tables() -> Result<(HuffTable, HuffTable), DomainError> {
    let mut lit = [0u8; 288];
    for (i, v) in lit.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let dist = [5u8; 30];
    Ok((HuffTable::new(&lit)?, HuffTable::new(&dist)?))
}

/// Read a dynamic-block header and build its literal/length and distance
/// tables (RFC 1951 §3.2.7).
fn dynamic_tables(br: &mut BitReader<'_>) -> Result<(HuffTable, HuffTable), DomainError> {
    let hlit = br.read_bits(5)? as usize + 257;
    let hdist = br.read_bits(5)? as usize + 1;
    let hclen = br.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(DomainError::invalid_request("inflate: bad dynamic header"));
    }
    let mut cl_lengths = [0u8; 19];
    for &slot in CLORDER.iter().take(hclen) {
        cl_lengths[slot] = br.read_bits(3)? as u8;
    }
    let cl_table = HuffTable::new(&cl_lengths)?;
    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut i = 0usize;
    while i < total {
        let sym = cl_table.decode(br)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(DomainError::invalid_request(
                        "inflate: RLE 16 with no previous length",
                    ));
                }
                let prev = lengths[i - 1];
                let rep = 3 + br.read_bits(2)? as usize;
                if i + rep > total {
                    return Err(DomainError::invalid_request("inflate: RLE 16 run"));
                }
                for _ in 0..rep {
                    lengths[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + br.read_bits(3)? as usize;
                if i + rep > total {
                    return Err(DomainError::invalid_request("inflate: RLE 17 run"));
                }
                for _ in 0..rep {
                    lengths[i] = 0;
                    i += 1;
                }
            }
            18 => {
                let rep = 11 + br.read_bits(7)? as usize;
                if i + rep > total {
                    return Err(DomainError::invalid_request("inflate: RLE 18 run"));
                }
                for _ in 0..rep {
                    lengths[i] = 0;
                    i += 1;
                }
            }
            _ => {
                return Err(DomainError::invalid_request(
                    "inflate: bad code-length symbol",
                ))
            }
        }
    }
    Ok((
        HuffTable::new(&lengths[..hlit])?,
        HuffTable::new(&lengths[hlit..])?,
    ))
}

/// Consume symbols of one Huffman-compressed block (fixed or dynamic),
/// appending literals and copying matches into `out`, bounded by `expect`.
fn inflate_block_symbols(
    br: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    expect: usize,
    lit_table: &HuffTable,
    dist_table: &HuffTable,
) -> Result<(), DomainError> {
    loop {
        let sym = lit_table.decode(br)?;
        if sym < 256 {
            if out.len() == expect {
                return Err(DomainError::invalid_request(
                    "inflate: stream exceeds declared frame size",
                ));
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(()); // end of block
        } else {
            let lidx = (sym - 257) as usize;
            if lidx >= LEN_BASE.len() {
                return Err(DomainError::invalid_request("inflate: bad length symbol"));
            }
            let len = LEN_BASE[lidx] as usize + br.read_bits(LEN_EXTRA[lidx])? as usize;
            let dsym = dist_table.decode(br)? as usize;
            if dsym >= DIST_BASE.len() {
                return Err(DomainError::invalid_request("inflate: bad distance symbol"));
            }
            let dist = DIST_BASE[dsym] as usize + br.read_bits(DIST_EXTRA[dsym])? as usize;
            if dist == 0 || dist > out.len() {
                return Err(DomainError::invalid_request(
                    "inflate: distance before output start",
                ));
            }
            if out.len() + len > expect {
                return Err(DomainError::invalid_request(
                    "inflate: match overruns declared frame size",
                ));
            }
            let src_start = out.len() - dist;
            for k in 0..len {
                let b = out[src_start + k];
                out.push(b);
            }
        }
    }
}

/// Inflate exactly `expect` bytes of raw deflate from `src`, bounded:
/// output never exceeds `expect`, and a stream that overruns, ends early,
/// or contains a structural violation is an error.
fn inflate_bounded(src: &[u8], expect: usize) -> Result<Vec<u8>, DomainError> {
    let mut out: Vec<u8> = Vec::with_capacity(expect);
    let mut br = BitReader::new(src);

    loop {
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                // Stored block (RFC 1951 §3.2.4): align, then LEN/NLEN
                // (little-endian) and LEN raw bytes.
                br.align_to_byte();
                let len = br.read_bits(16)? as usize;
                let nlen = br.read_bits(16)? as u16;
                if len as u16 != !nlen {
                    return Err(DomainError::invalid_request("inflate: bad stored LEN/NLEN"));
                }
                if out.len() + len > expect {
                    return Err(DomainError::invalid_request(
                        "inflate: stream exceeds declared frame size",
                    ));
                }
                // Stream byte position, accounting for bits still held.
                let start = br.byte_pos - (br.bit_cnt / 8) as usize;
                if start + len > br.data.len() {
                    return Err(DomainError::invalid_request(
                        "inflate: truncated stored data",
                    ));
                }
                let chunk = br.data[start..start + len].to_vec();
                out.extend_from_slice(&chunk);
                br.bit_buf = 0;
                br.bit_cnt = 0;
                br.byte_pos = start + len;
            }
            1 | 2 => {
                let owned = if btype == 1 {
                    fixed_tables()?
                } else {
                    dynamic_tables(&mut br)?
                };
                inflate_block_symbols(&mut br, &mut out, expect, &owned.0, &owned.1)?;
            }
            _ => {
                return Err(DomainError::invalid_request(
                    "inflate: reserved block type 3",
                ));
            }
        }
        if bfinal == 1 {
            break;
        }
    }

    if out.len() != expect {
        return Err(DomainError::invalid_request(format!(
            "inflate: produced {} bytes, declared frame needs {}",
            out.len(),
            expect
        )));
    }
    Ok(out)
}

/// Decode a zlib stream (RFC 1950) into exactly `expect` bytes.
///
/// Crate-visible so the PDF writer (`pdf.rs`) can round-trip-verify the
/// image streams it embeds before emitting the document.
pub(crate) fn zlib_decode(src: &[u8], expect: usize) -> Result<Vec<u8>, DomainError> {
    if src.len() < 6 {
        return Err(DomainError::invalid_request("zlib: stream too short"));
    }
    let cmf = src[0];
    let flg = src[1];
    if cmf & 0x0f != 8 {
        return Err(DomainError::invalid_request(
            "zlib: unsupported compression method",
        ));
    }
    if cmf >> 4 > 7 {
        return Err(DomainError::invalid_request(
            "zlib: unsupported window size",
        ));
    }
    if ((u16::from(cmf)) << 8 | u16::from(flg)) % 31 != 0 {
        return Err(DomainError::invalid_request("zlib: header FCHECK failed"));
    }
    if flg & 0x20 != 0 {
        return Err(DomainError::invalid_request(
            "zlib: preset dictionary unsupported",
        ));
    }
    let payload_end = src.len() - 4;
    let stored_adler = u32::from_be_bytes([
        src[payload_end],
        src[payload_end + 1],
        src[payload_end + 2],
        src[payload_end + 3],
    ]);
    let out = inflate_bounded(&src[2..payload_end], expect)?;
    if adler32(&out) != stored_adler {
        return Err(DomainError::checksum_mismatch(
            "zlib: Adler-32 does not match decompressed data",
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// PNG chunk parsing + decode
// ---------------------------------------------------------------------------

fn is_critical(chunk: &[u8; 4]) -> bool {
    // RFC 15948 §5.3: an uppercase first letter marks a critical chunk.
    chunk[0].is_ascii_uppercase()
}

/// Paeth predictor (ISO/IEC 15948 §6.4).
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (i32::from(a), i32::from(b), i32::from(c));
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Reverse the per-scanline PNG filters over `raw` (w+1 bytes per row).
fn unfilter(raw: &[u8], width: usize, height: usize) -> Result<Vec<u8>, DomainError> {
    let bpp = 1usize; // 8-bit grayscale: one byte per pixel
    let mut pixels = vec![0u8; width * height];
    let mut prev = vec![0u8; width];
    let mut cur = vec![0u8; width];
    for y in 0..height {
        let base = y * (width + 1);
        let filter = raw[base];
        let line = &raw[base + 1..base + 1 + width];
        cur.copy_from_slice(line);
        match filter {
            0 => {}
            1 => {
                for x in 1..width {
                    cur[x] = cur[x].wrapping_add(cur[x - 1]);
                }
            }
            2 => {
                for x in 0..width {
                    cur[x] = cur[x].wrapping_add(prev[x]);
                }
            }
            3 => {
                for x in 0..width {
                    let left = if x >= bpp { cur[x - 1] as u32 } else { 0 };
                    cur[x] = cur[x].wrapping_add(((left + u32::from(prev[x])) / 2) as u8);
                }
            }
            4 => {
                for x in 0..width {
                    let a = if x >= bpp { cur[x - 1] } else { 0 };
                    let b = prev[x];
                    let c = if x >= bpp { prev[x - 1] } else { 0 };
                    cur[x] = cur[x].wrapping_add(paeth(a, b, c));
                }
            }
            _ => {
                return Err(DomainError::invalid_request(format!(
                    "png: invalid scanline filter type {filter}"
                )))
            }
        }
        pixels[y * width..(y + 1) * width].copy_from_slice(&cur);
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(pixels)
}

/// Decode PNG bytes into a `GrayFrame`.
///
/// Accepts only 8-bit, non-interlaced, color type 0 (grayscale). Everything
/// else about the input is treated as hostile: sizes are validated before
/// allocation, the inflate target is fixed by IHDR, checksums are enforced,
/// and malformed structures return `DomainError` categories rather than
/// panicking.
pub fn decode_png(bytes: &[u8]) -> Result<GrayFrame, DomainError> {
    if bytes.len() < 8 + 12 || bytes[..8] != PNG_SIGNATURE {
        return Err(DomainError::invalid_request(
            "png: bad signature or too short",
        ));
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    // Chunk state machine: 0 = before IDAT, 1 = collecting IDATs, 2 = after IDATs.
    let mut phase = 0u8;
    let mut idat = Vec::new();
    let mut idat_total: u64 = 0;
    let mut saw_ihdr = false;
    let mut saw_iend = false;

    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&bytes[pos + 4..pos + 8]);
        let data_start = pos + 8;
        let data_end = match data_start.checked_add(len) {
            Some(e) if e + 4 <= bytes.len() => e,
            _ => {
                return Err(DomainError::invalid_request(
                    "png: truncated or absurd chunk",
                ))
            }
        };
        let want_crc = u32::from_be_bytes([
            bytes[data_end],
            bytes[data_end + 1],
            bytes[data_end + 2],
            bytes[data_end + 3],
        ]);
        let mut crc_input = Vec::with_capacity(4 + len);
        crc_input.extend_from_slice(&kind);
        crc_input.extend_from_slice(&bytes[data_start..data_end]);
        if crc32(&crc_input) != want_crc {
            return Err(DomainError::checksum_mismatch(format!(
                "png: CRC mismatch in {} chunk",
                String::from_utf8_lossy(&kind)
            )));
        }
        let data = &bytes[data_start..data_end];

        match &kind {
            b"IHDR" => {
                if saw_ihdr || phase != 0 || len != 13 {
                    return Err(DomainError::invalid_request("png: misplaced or bad IHDR"));
                }
                saw_ihdr = true;
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                let depth = data[8];
                let color = data[9];
                let compression = data[10];
                let filter = data[11];
                let interlace = data[12];
                if depth != 8 {
                    return Err(DomainError::invalid_request(format!(
                        "png: unsupported bit depth {depth} (8-bit only)"
                    )));
                }
                if color != 0 {
                    return Err(DomainError::invalid_request(format!(
                        "png: unsupported color type {color} (grayscale only)"
                    )));
                }
                if compression != 0 || filter != 0 {
                    return Err(DomainError::invalid_request(
                        "png: unsupported compression/filter method",
                    ));
                }
                if interlace != 0 {
                    return Err(DomainError::invalid_request("png: interlacing unsupported"));
                }
                // Bounds *before* any decompression or allocation.
                check_bounds(width, height)?;
            }
            b"IDAT" => {
                if !saw_ihdr || phase == 2 {
                    return Err(DomainError::invalid_request("png: misplaced IDAT"));
                }
                phase = 1;
                idat_total += len as u64;
                if idat_total > MAX_IDAT_TOTAL {
                    return Err(DomainError::invalid_request(
                        "png: compressed data over bound",
                    ));
                }
                idat.extend_from_slice(data);
            }
            b"PLTE" | b"tRNS" => {
                // Palette/transparency belong to color types we reject; on a
                // grayscale image they are a spec violation or an
                // unsupported feature either way.
                return Err(DomainError::invalid_request(format!(
                    "png: unsupported chunk {}",
                    String::from_utf8_lossy(&kind)
                )));
            }
            b"IEND" => {
                if len != 0 {
                    return Err(DomainError::invalid_request("png: IEND must be empty"));
                }
                saw_iend = true;
                break;
            }
            other => {
                if is_critical(other) {
                    return Err(DomainError::invalid_request(format!(
                        "png: unknown critical chunk {}",
                        String::from_utf8_lossy(other)
                    )));
                }
                // Ancillary chunks (gAMA, pHYs, tEXt, …) are ignored.
            }
        }
        pos = data_end + 4;
    }

    if !saw_ihdr || phase != 1 || !saw_iend {
        return Err(DomainError::invalid_request(
            "png: missing IHDR, IDAT, or IEND",
        ));
    }

    let w = width as usize;
    let h = height as usize;
    // PNG scanline-interleaved raw size: one filter byte per scanline.
    let expect = h * (w + 1);
    let raw = zlib_decode(&idat, expect)?;
    let pixels = unfilter(&raw, w, h)?;
    GrayFrame::from_pixels(width, height, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame() -> GrayFrame {
        let pixels = (0..(5 * 4)).map(|i| (i * 37) as u8).collect();
        GrayFrame::from_pixels(5, 4, pixels).unwrap()
    }

    #[test]
    fn crc32_matches_known_vector() {
        // Standard check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn adler32_matches_known_vectors() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn paeth_matches_spec() {
        // Nearest predictor per ISO 15948 §6.4 worked examples.
        assert_eq!(paeth(10, 12, 8), 12); // pb smallest -> b
        assert_eq!(paeth(10, 20, 8), 20); // pb smallest -> b
        assert_eq!(paeth(30, 20, 10), 30); // pa smallest -> a
        assert_eq!(paeth(0, 0, 0), 0);
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let frame = sample_frame();
        let png = encode_png(&frame).unwrap();
        let back = decode_png(&png).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn encode_rejects_out_of_bounds_frame() {
        // Hand-build a frame shape that violates IHDR bounds.
        let frame = GrayFrame {
            width: 40_000,
            height: 40_000,
            pixels: vec![0u8; 4],
        };
        let err = encode_png(&frame).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn decode_rejects_garbage() {
        let err = decode_png(b"not a png at all, but long enough to try").unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn decode_rejects_truncated_chunk() {
        let frame = sample_frame();
        let mut png = encode_png(&frame).unwrap();
        let cut = png.len() - 3;
        png.truncate(cut);
        let err = decode_png(&png).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn decode_rejects_bad_crc() {
        let frame = sample_frame();
        let mut png = encode_png(&frame).unwrap();
        // Flip a bit inside IHDR data (signature=8 bytes, len=4, type=4).
        png[20] ^= 0x01;
        let err = decode_png(&png).unwrap_err();
        assert_eq!(err.category, crate::error::Category::ChecksumMismatch);
    }

    #[test]
    fn decode_rejects_oversized_ihdr_dimensions() {
        // Craft a minimal IHDR-only claim of 40000x40000 (> MAX_DIMENSION_PX).
        let mut out = Vec::new();
        out.extend_from_slice(&PNG_SIGNATURE);
        let mut ihdr = Vec::new();
        push_u32_be(&mut ihdr, 40_000);
        push_u32_be(&mut ihdr, 40_000);
        ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
        write_chunk(&mut out, b"IHDR", &ihdr);
        write_chunk(&mut out, b"IDAT", b"x\x01\x00\x00");
        write_chunk(&mut out, b"IEND", b"");
        let err = decode_png(&out).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn zlib_stored_blocks_span_multiple_64k_chunks() {
        let data: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let z = zlib_stored(&data);
        let back = zlib_decode(&z, data.len()).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn zlib_stored_handles_empty_payload() {
        let z = zlib_stored(&[]);
        let back = zlib_decode(&z, 0).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn inflate_rejects_overrun_target() {
        let data = vec![7u8; 32];
        let z = zlib_stored(&data);
        // Declared frame smaller than the stream actually produces.
        let err = zlib_decode(&z, 8).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn inflate_rejects_bad_stored_len() {
        // Corrupt a stored block's LEN (bytes 2-5 of the zlib stream after
        // the 2-byte header are: block flags, LEN lo, LEN hi, NLEN ...).
        let z = zlib_stored(b"hello world");
        let mut bad = z.clone();
        bad[4] ^= 0xFF; // high byte of LEN
        let err = zlib_decode(&bad, 11).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn zlib_rejects_bad_adler() {
        let data = b"payload bytes".to_vec();
        let mut z = zlib_stored(&data);
        let n = z.len();
        z[n - 1] ^= 0xFF;
        let err = zlib_decode(&z, data.len()).unwrap_err();
        assert_eq!(err.category, crate::error::Category::ChecksumMismatch);
    }

    #[test]
    fn unfilter_sub_and_up() {
        let width = 4;
        let raw = vec![
            1, 3, 3, 3, 3, // Sub row -> 3,6,9,12
            2, 1, 1, 1, 1, // Up over previous -> 4,7,10,13
        ];
        let px = unfilter(&raw, width, 2).unwrap();
        assert_eq!(px, vec![3, 6, 9, 12, 4, 7, 10, 13]);
    }

    #[test]
    fn unfilter_average() {
        // line1 filter 3 (Average): rec(x) = enc + floor((left+up)/2)
        let raw = vec![0, 10, 20, 30, 3, 8, 12, 17];
        // x0: 8+(0+10)/2=13; x1: 12+(13+20)/2=28; x2: 17+(28+30)/2=46
        let px = unfilter(&raw, 3, 2).unwrap();
        assert_eq!(px, vec![10, 20, 30, 13, 28, 46]);
    }

    #[test]
    fn unfilter_paeth() {
        // line1 filter 4: rec(x) = enc + Paeth(left, up, upleft)
        let raw = vec![0, 10, 20, 30, 4, 1, 2, 3];
        // x0: pred(0,10,0)=10 -> 11; x1: pred(11,20,10)=20 -> 22;
        // x2: pred(22,30,20)=30 -> 33
        let px = unfilter(&raw, 3, 2).unwrap();
        assert_eq!(px, vec![10, 20, 30, 11, 22, 33]);
    }

    #[test]
    fn unfilter_rejects_bad_filter_id() {
        let raw = vec![9, 1, 2, 3];
        let err = unfilter(&raw, 3, 1).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn huff_rejects_over_subscribed_lengths() {
        // Two 1-bit codes plus one 2-bit code violates the Kraft sum.
        let err = HuffTable::new(&[1, 1, 2]).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn fixed_tables_build() {
        let (lit, dist) = fixed_tables().unwrap();
        assert!(lit.max_len > 0 && dist.max_len > 0);
    }
}
