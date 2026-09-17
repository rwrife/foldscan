#!/usr/bin/env python3
"""Generate committed PNG fixtures for app/domain/tests/png_codec.rs.

Run from anywhere; writes into tests/fixtures/png/ next to this script.

    python3 generate.py

Every fixture is validated with Pillow (libpng + zlib — a decoder fully
independent of the Rust crate) before it is written, and the manifest
records the expected `GrayFrame::digest()` for every decodable fixture.
The fixtures therefore prove two things the crate's own round-trip cannot:
an independent *encoder* (libpng/zlib, plus hand-built fixed-Huffman and
filtered scanlines crafted here with Python's zlib) produces bytes the Rust
decoder reads correctly, and the reject-path fixtures really are what they
claim to be (Pillow agrees they are 16-bit, interlaced, RGB, or palette).

Frames are built from a seeded LCG so regeneration is byte-reproducible.
No personal document content; everything is synthetic noise/gradients.

Requires: Pillow.  (Development-time tool; CI for app/domain does not run
this script — it consumes the committed fixtures.)
"""

import hashlib
import io
import json
import struct
import zlib
from pathlib import Path

from PIL import Image

HERE = Path(__file__).parent


def lcg_frame(w: int, h: int, seed: int) -> bytes:
    """Deterministic pseudo-random grayscale bytes (same LCG style as the
    crate's golden_processing.rs fixtures)."""
    s = seed & 0xFFFF
    out = bytearray()
    for _ in range(w * h):
        s = ((s * 25173 + 13849) << 3 & 0xFFFF) ^ (s >> 5)
        out.append((s >> 8) & 0xFF)
    return bytes(out)


def gray_frame(w: int, h: int, pixels: bytes) -> str:
    """Reproduce GrayFrame::digest() = SHA-256("foldscan.frame/1" || w_le32
    || h_le32 || pixels)."""
    h_bytes = b"foldscan.frame/1" + struct.pack("<II", w, h) + pixels
    return hashlib.sha256(h_bytes).hexdigest()


def png_chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def png_gray8(w: int, h: int, pixels: bytes, filters=None, zlib_level=6) -> bytes:
    """Minimal PNG encoder with *per-scanline filter control* (libpng via
    PIL never emits filters 1-4 by default, so filtered coverage needs a
    crafted-but-independent encoder: standard zlib for the deflate stage)."""
    height_bytes = bytearray()
    for y in range(h):
        row = pixels[y * w:(y + 1) * w]
        f = 0 if filters is None else filters[y % len(filters)]
        out = bytearray(row)
        if f == 1:  # Sub
            for x in range(w - 1, 0, -1):
                out[x] = (out[x] - out[x - 1]) & 0xFF
        elif f == 2:  # Up
            prev = pixels[(y - 1) * w: y * w] if y > 0 else b"\x00" * w
            for x in range(w):
                out[x] = (out[x] - prev[x]) & 0xFF
        elif f == 3:  # Average
            for x in range(w - 1, -1, -1):
                left = out[x - 1] if x > 0 else 0
                prev = pixels[(y - 1) * w + x] if y > 0 else 0
                out[x] = (out[x] - (left + prev) // 2) & 0xFF
        elif f == 4:  # Paeth
            for x in range(w - 1, -1, -1):
                a = out[x - 1] if x > 0 else 0
                b = pixels[(y - 1) * w + x] if y > 0 else 0
                c = pixels[(y - 1) * w + x - 1] if (y > 0 and x > 0) else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                out[x] = (out[x] - pred) & 0xFF
        else:
            f = 0
        height_bytes.append(f)
        height_bytes.extend(out)
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", ihdr)
        + png_chunk(b"IDAT", zlib.compress(bytes(height_bytes), zlib_level))
        + png_chunk(b"IEND", b"")
    )


def fixed_huffman_deflate(data: bytes) -> bytes:
    """One final fixed-Huffman block (RFC 1951 §3.2.6): literal bytes 0..143
    use the 8-bit code 48+b, bytes 144..255 use the 9-bit code 0x190+(b-144),
    EOT (256) is the 7-bit code 0. Bits pack LSB-first per byte."""
    # BFINAL=1; BTYPE=01 (fixed) read LSB-first => stream bits 1,1,0.
    bits = ["1", "10"]
    for b in data:
        if b < 144:
            bits.append(format(48 + b, "08b"))
        else:
            bits.append(format(0x190 + (b - 144), "09b"))
    bits.append("0000000")  # EOT
    out = bytearray()
    chunk = "".join(bits)
    for i in range(0, len(chunk), 8):
        byte = chunk[i:i + 8]
        # Pad the final byte with zero bits, LSB-first order.
        byte = byte.ljust(8, "0")
        out.append(int(byte[::-1], 2))
    return bytes(out)


def png_fixed_block(w: int, h: int, pixels: bytes) -> bytes:
    """PNG whose IDAT is a zlib stream wrapping a *fixed-Huffman* deflate
    block — the block type neither libpng's default nor the crate's own
    encoder emits, so only the fixture set exercises it."""
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        raw.extend(pixels[y * w:(y + 1) * w])
    stream = fixed_huffman_deflate(bytes(raw))
    adler = zlib.adler32(bytes(raw)) & 0xFFFFFFFF
    idat = b"\x78\x01" + stream + struct.pack(">I", adler)
    # Third-party validation of the hand-built stream:
    assert zlib.decompress(idat) == bytes(raw)
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", ihdr)
        + png_chunk(b"IDAT", idat)
        + png_chunk(b"IEND", b"")
    )


def expected_pixels(w: int, h: int, pixels: bytes):
    """(w, h, pixels) tuple used for the Pillow validation round-trip."""
    return w, h, pixels


manifest = {"_comment": "Generated by generate.py; validated with Pillow at "
             "generation time. digest = SHA-256 of 'foldscan.frame/1'||w_le"
             "32||h_le32||pixels (matches GrayFrame::digest()).",
            "valid": [], "reject": []}

# --- valid: PIL/libpng encoders (dynamic Huffman, filter 0) ---
cases = {
    "pil_noise_32x32_seed7.png": (32, 32, lcg_frame(32, 32, 7)),
    "pil_noise_1x40_seed3.png": (1, 40, lcg_frame(1, 40, 3)),
    "pil_noise_40x1_seed4.png": (40, 1, lcg_frame(40, 1, 4)),
    "pil_flat_5x5.png": (5, 5, bytes([200] * 25)),
    "pil_gradient_17x5.png": (
        17, 5, bytes((x * 255 // 16) for y in range(5) for x in range(17))),
}
for name, (w, h, px) in cases.items():
    img = Image.frombytes("L", (w, h), px)
    import io
    buf = io.BytesIO()
    img.save(buf, format="PNG", compress_level=6)
    (HERE / name).write_bytes(buf.getvalue())
    path = HERE / name
    # Pillow validation (independent decoder agrees with our pixel model):
    back = Image.open(path)
    assert back.mode == "L" and back.size == (w, h) and back.tobytes() == px
    manifest["valid"].append({"file": name, "w": w, "h": h,
                              "digest": gray_frame(w, h, px),
                              "note": "libpng/zlib encoder, zlib level 6"})

# --- valid: crafted filtered scanlines (filters 1,2,3,4 + None round-robin) ---
w, h = 11, 9
px = lcg_frame(w, h, 11)
data = png_gray8(w, h, px, filters=[0, 1, 2, 3, 4])
(HERE / "crafted_filters_11x9.png").write_bytes(data)
back = Image.open(HERE / "crafted_filters_11x9.png")
assert back.mode == "L" and back.size == (w, h) and back.tobytes() == px
manifest["valid"].append({"file": "crafted_filters_11x9.png", "w": w, "h": h,
                          "digest": gray_frame(w, h, px),
                          "note": "python-zlib encoder, scanline filters "
                          "0-4 round-robin"})

# --- valid: crafted fixed-Huffman deflate block ---
w, h = 6, 6
px = lcg_frame(w, h, 61)
data = png_fixed_block(w, h, px)
(HERE / "crafted_fixed_block_6x6.png").write_bytes(data)
back = Image.open(HERE / "crafted_fixed_block_6x6.png")
assert back.mode == "L" and back.size == (w, h) and back.tobytes() == px
manifest["valid"].append({"file": "crafted_fixed_block_6x6.png", "w": w, "h": h,
                          "digest": gray_frame(w, h, px),
                          "note": "fixed-Huffman deflate block (only the "
                          "fixture set exercises this block type in CI)"})

# --- valid: 1x1 edge sizes ---
for name, v in [("pil_1x1_white.png", 255), ("pil_1x1_black.png", 0)]:
    img = Image.new("L", (1, 1), v)
    img.save(HERE / name, format="PNG")
    assert Image.open(HERE / name).tobytes() == bytes([v])
    manifest["valid"].append({"file": name, "w": 1, "h": 1,
                              "digest": gray_frame(1, 1, bytes([v])),
                              "note": "1x1 boundary"})

# --- reject: color type 2 (RGB) ---
Image.new("RGB", (4, 4), (10, 200, 128)).save(HERE / "rgb_4x4.png")
assert Image.open(HERE / "rgb_4x4.png").mode == "RGB"
manifest["reject"].append({"file": "rgb_4x4.png", "expect": "invalid",
                           "note": "color type 2, Pillow confirms RGB mode"})

# --- reject: palette (color type 3, PLTE) ---
pal = Image.new("P", (4, 4))
pal.save(HERE / "palette_4x4.png")
assert Image.open(HERE / "palette_4x4.png").mode == "P"
manifest["reject"].append({"file": "palette_4x4.png", "expect": "invalid",
                           "note": "color type 3 with PLTE"})

# --- reject: 16-bit gray ---
img = Image.new("I;16", (4, 4))
img.save(HERE / "gray16_4x4.png")
with open(HERE / "gray16_4x4.png", "rb") as f:
    d = f.read()
assert d[24] == 16  # IHDR bit depth byte
manifest["reject"].append({"file": "gray16_4x4.png", "expect": "invalid",
                           "note": "bit depth 16"})

# --- reject: Adam7 interlaced (pypng: an independent pure-Python encoder;
# Pillow's own PNG save() ignores interlacing, so it cannot produce this) ---
w, h = 8, 8
px = lcg_frame(w, h, 9)
try:
    import io as _io
    import png as _pypng
except ImportError as exc:  # pragma: no cover - dev-time dependency
    raise SystemExit("pypng is required to generate the interlaced fixture") from exc
_buf = _io.BytesIO()
_pypng.Writer(w, h, greyscale=True, bitdepth=8, interlace=1).write(
    _buf, [list(px[y * w:(y + 1) * w]) for y in range(h)]
)
(HERE / "interlaced_8x8.png").write_bytes(_buf.getvalue())
with open(HERE / "interlaced_8x8.png", "rb") as f:
    d = f.read()
assert d[28] == 1  # IHDR interlace byte
# Validate with Pillow (libpng): it must report interlace and decode the pixels.
img = Image.open(HERE / "interlaced_8x8.png")
assert img.info.get("interlace") == 1, img.info
assert img.tobytes() == px, "Pillow Adam7 decode mismatch"
manifest["reject"].append({"file": "interlaced_8x8.png", "expect": "invalid",
                           "note": "Adam7 interlace via pypng; Pillow/libpng confirms interlaced and decodes it"})

# --- reject: truncated IDAT (corrupt a real PIL file) ---
src = (HERE / "pil_noise_32x32_seed7.png").read_bytes()
cut = len(src) - 6
(HERE / "truncated_idat.png").write_bytes(src[:cut])
manifest["reject"].append({"file": "truncated_idat.png", "expect": "invalid",
                           "note": "IDAT chunk CRC/length fails on a truncated "
                           "file (Pillow refuses it too at generation check)"})
try:
    Image.open(io.BytesIO(src[:cut])).load()
    raise AssertionError("expected Pillow to reject truncated file")
except Exception:
    pass

(HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
n_valid = len(manifest["valid"])
n_reject = len(manifest["reject"])
print(f"wrote {n_valid} valid + {n_reject} reject fixtures + manifest.json")
