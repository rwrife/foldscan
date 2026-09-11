//! Deterministic 8-bit grayscale frame model and exact image primitives.
//!
//! The companion's processing layer works on decoded grayscale frames. This
//! module is the boundary: it deliberately knows nothing about JPEG/PNG
//! decoding or encoding — host codecs own conversion between capture files
//! and frames, and the export layer owns writing derivative bytes. Keeping
//! the frame model raw and small is what makes processing output testable
//! and *bit-stable*: every primitive here is either pure integer arithmetic
//! or an IEEE-754 correctly-rounded single operation (`+ - * /`, `sqrt`,
//! `floor`, `round`), never a transcendental or a libm function whose
//! result can differ between hosts. Golden-fixture digests therefore hold
//! on any IEEE-754 platform, which is what the issue #5 acceptance
//! criterion "stable output" needs.
//!
//! Bounds are enforced before allocation (the same discipline the importer
//! uses): a frame's declared pixel count is capped, and pixel buffer
//! length must match `width * height` exactly.

use crate::checksum::sha256_hex;
use crate::error::DomainError;
use crate::limits::MAX_DIMENSION_PX;

/// Maximum total pixels in one processing frame (~64 MP). This is a host
/// processing bound, deliberately tighter than the per-axis manifest bound:
/// two axes at `MAX_DIMENSION_PX` would declare over a gigapixel.
pub const MAX_FRAME_PIXELS: u64 = 64_000_000;

/// Domain-separation header for frame digests, so a 2x4 and a 4x2 frame
/// with equal byte payloads can never collide.
const FRAME_DIGEST_HEADER: &[u8] = b"foldscan.frame/1";

/// An 8-bit grayscale raster in row-major order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrayFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl GrayFrame {
    /// A `width x height` frame filled with one value, bounds-checked.
    pub fn new(width: u32, height: u32, fill: u8) -> Result<Self, DomainError> {
        check_bounds(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![fill; (width as usize) * (height as usize)],
        })
    }

    /// Wrap an existing buffer, requiring the exact expected length.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, DomainError> {
        check_bounds(width, height)?;
        let expected = (width as usize) * (height as usize);
        if pixels.len() != expected {
            return Err(DomainError::invalid_request(format!(
                "frame payload has {} bytes, expected {} for {}x{}",
                pixels.len(),
                expected,
                width,
                height
            )));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Pixel value at (x, y). Caller is expected to keep coordinates in
    /// range; out-of-range reads panic in debug and wrap in release, so all
    /// internal callers clamp before sampling.
    pub fn at(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y as usize) * (self.width as usize) + (x as usize)]
    }

    /// Deterministic digest binding the frame's dimensions to its payload.
    pub fn digest(&self) -> String {
        let mut buf = Vec::with_capacity(FRAME_DIGEST_HEADER.len() + 8 + self.pixels.len());
        buf.extend_from_slice(FRAME_DIGEST_HEADER);
        buf.extend_from_slice(&self.width.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.extend_from_slice(&self.pixels);
        sha256_hex(&buf)
    }
}

/// Reject zero-area or over-bound frames *before* any allocation.
pub(crate) fn check_bounds(width: u32, height: u32) -> Result<(), DomainError> {
    if width == 0 || height == 0 {
        return Err(DomainError::invalid_request(
            "frame must have positive width and height",
        ));
    }
    if width as u64 > MAX_DIMENSION_PX || height as u64 > MAX_DIMENSION_PX {
        return Err(DomainError::invalid_request(format!(
            "frame dimension exceeds {} px",
            MAX_DIMENSION_PX
        )));
    }
    if width as u64 * height as u64 > MAX_FRAME_PIXELS {
        return Err(DomainError::invalid_request(format!(
            "frame declares {} pixels, over the {} limit",
            width as u64 * height as u64,
            MAX_FRAME_PIXELS
        )));
    }
    Ok(())
}

/// Rotate by quarter turns counter-clockwise (`turns` is reduced mod 4).
/// Pure data movement: exact and reversible by construction.
pub fn rotate(src: &GrayFrame, turns: u32) -> GrayFrame {
    match turns.rem_euclid(4) {
        0 => src.clone(),
        1 => {
            // 90 CCW: dest(x, y) = src(y, h - 1 - x)
            let (w, h) = (src.width, src.height);
            let mut out = GrayFrame {
                width: h,
                height: w,
                pixels: Vec::with_capacity(src.pixels.len()),
            };
            out.pixels.resize((w as usize) * (h as usize), 0);
            for y in 0..h {
                for x in 0..w {
                    let v = src.at(x, y);
                    let dy = w - 1 - x;
                    let dx = y;
                    out.pixels[(dy as usize) * (h as usize) + (dx as usize)] = v;
                }
            }
            out
        }
        2 => {
            let (w, h) = (src.width, src.height);
            let mut out = GrayFrame {
                width: w,
                height: h,
                pixels: Vec::with_capacity(src.pixels.len()),
            };
            out.pixels.resize((w as usize) * (h as usize), 0);
            for y in 0..h {
                for x in 0..w {
                    let v = src.at(x, y);
                    out.pixels[((h - 1 - y) as usize) * (w as usize) + ((w - 1 - x) as usize)] = v;
                }
            }
            out
        }
        _ => {
            // 270 CCW == 90 CW: dest(x, y) = src(w - 1 - y, x)
            let (w, h) = (src.width, src.height);
            let mut out = GrayFrame {
                width: h,
                height: w,
                pixels: Vec::with_capacity(src.pixels.len()),
            };
            out.pixels.resize((w as usize) * (h as usize), 0);
            for y in 0..h {
                for x in 0..w {
                    let v = src.at(x, y);
                    let dy = x;
                    let dx = w - 1 - y;
                    out.pixels[(dy as usize) * (h as usize) + (dx as usize)] = v;
                }
            }
            out
        }
    }
}

/// Axis-aligned crop with strict bounds: the requested window must lie
/// entirely inside the source. Out-of-bounds crops are a user-request error,
/// never a silent clamp.
pub fn crop(
    src: &GrayFrame,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<GrayFrame, DomainError> {
    check_bounds(width, height)?;
    if x.checked_add(width)
        .ok_or_else(|| DomainError::invalid_request("crop window overflows u32"))?
        > src.width
        || y.checked_add(height)
            .ok_or_else(|| DomainError::invalid_request("crop window overflows u32"))?
            > src.height
    {
        return Err(DomainError::invalid_request(format!(
            "crop window {}x{} at ({},{}) exceeds source {}x{}",
            width, height, x, y, src.width, src.height
        )));
    }
    let mut out = GrayFrame {
        width,
        height,
        pixels: Vec::with_capacity((width as usize) * (height as usize)),
    };
    for oy in 0..height {
        let row_start = ((y + oy) as usize) * (src.width as usize) + (x as usize);
        out.pixels
            .extend_from_slice(&src.pixels[row_start..row_start + (width as usize)]);
    }
    Ok(out)
}

/// Bilinear-interpolated sample with edge clamping. Weights are fixed point
/// in 1/65536 (`BILINEAR_ONE`), and every step is integer arithmetic with
/// round-half-up divisions realized as `+ half` then shift, so results are
/// platform-independent by construction.
pub(crate) const BILINEAR_ONE: u32 = 1 << 16;

/// Integer bilinear weight (0..=BILINEAR_ONE) for a fractional part
/// `fract` in 0.0..1.0. The multiply and round are IEEE-exact.
pub(crate) fn weight_of(fract: f64) -> u32 {
    let w = (fract * BILINEAR_ONE as f64).round();
    // fract is clamped by callers; guard anyway so a NaN cannot poison the
    // sampling path.
    if !w.is_finite() || w <= 0.0 {
        0
    } else if w >= BILINEAR_ONE as f64 {
        BILINEAR_ONE
    } else {
        w as u32
    }
}

/// Sample `src` at float coordinates (fx, fy) with bilinear interpolation
/// and edge clamping. All rounding is the deterministic fixed-point scheme
/// above.
pub(crate) fn sample_bilinear(src: &GrayFrame, fx: f64, fy: f64) -> u8 {
    let max_x = src.width - 1;
    let max_y = src.height - 1;
    let fx = if fx.is_finite() {
        fx.clamp(0.0, max_x as f64)
    } else {
        0.0
    };
    let fy = if fy.is_finite() {
        fy.clamp(0.0, max_y as f64)
    } else {
        0.0
    };
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(max_x);
    let y1 = (y0 + 1).min(max_y);
    let wx = weight_of(fx - x0 as f64);
    let wy = weight_of(fy - y0 as f64);

    let a = src.at(x0, y0) as u32;
    let b = src.at(x1, y0) as u32;
    let c = src.at(x0, y1) as u32;
    let d = src.at(x1, y1) as u32;

    let top = (a * (BILINEAR_ONE - wx) + b * wx + (BILINEAR_ONE / 2)) >> 16;
    let bot = (c * (BILINEAR_ONE - wx) + d * wx + (BILINEAR_ONE / 2)) >> 16;
    let v = (top * (BILINEAR_ONE - wy) + bot * wy + (BILINEAR_ONE / 2)) >> 16;
    v.min(255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, pixels: &[u8]) -> GrayFrame {
        GrayFrame::from_pixels(w, h, pixels.to_vec()).expect("fixture frame")
    }

    #[test]
    fn bounds_reject_zero_area_and_oversize() {
        assert!(GrayFrame::new(0, 4, 0).is_err());
        assert!(GrayFrame::new(40_000, 40_000, 0).is_err());
        assert!(GrayFrame::new(32_768, 32_768, 0).is_err()); // over MAX_FRAME_PIXELS
        assert!(GrayFrame::from_pixels(2, 2, vec![0, 1, 2]).is_err());
    }

    #[test]
    fn rotate_quarter_turn_roundtrip_is_identity() {
        let src = frame(3, 2, &[1, 2, 3, 4, 5, 6]);
        let r = rotate(&rotate(&rotate(&rotate(&src, 1), 1), 1), 1);
        assert_eq!(r, src);
    }

    #[test]
    fn rotate90_matches_expected_layout() {
        let src = frame(3, 2, &[1, 2, 3, 4, 5, 6]); // rows: 1 2 3 / 4 5 6
        let r = rotate(&src, 1); // 90 CCW -> 2x3
        assert_eq!((r.width, r.height), (2, 3));
        assert_eq!(r.pixels, vec![3, 6, 2, 5, 1, 4]);
    }

    #[test]
    fn crop_clips_rows_and_rejects_oob() {
        let src = frame(4, 2, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let c = crop(&src, 1, 0, 2, 2).expect("crop");
        assert_eq!(c.pixels, vec![2, 3, 6, 7]);
        assert!(crop(&src, 2, 0, 3, 2).is_err());
        assert!(crop(&src, 0, 0, 0, 2).is_err());
    }

    #[test]
    fn digest_binds_dimensions_to_payload() {
        let a = frame(2, 2, &[1, 2, 3, 4]);
        let b = frame(4, 1, &[1, 2, 3, 4]);
        assert_ne!(a.digest(), b.digest());
        // Stable across independent construction.
        assert_eq!(a.digest(), frame(2, 2, &[1, 2, 3, 4]).digest());
    }

    #[test]
    fn bilinear_exact_at_lattice_and_midpoint() {
        let src = frame(2, 1, &[0, 200]);
        assert_eq!(sample_bilinear(&src, 0.0, 0.0), 0);
        assert_eq!(sample_bilinear(&src, 1.0, 0.0), 200);
        assert_eq!(sample_bilinear(&src, 0.5, 0.0), 100);
        // Clamping outside the frame repeats the border.
        assert_eq!(sample_bilinear(&src, -3.0, 0.0), 0);
        assert_eq!(sample_bilinear(&src, 9.0, 0.0), 200);
    }
}
