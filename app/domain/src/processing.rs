//! Deterministic recipe application behind a narrow processing interface.
//!
//! This module realizes the issue #5 acceptance criterion "deterministic
//! perspective and illumination correction behind a processing interface
//! with ... golden fixtures ... and stable output", at the domain level:
//!
//! - A [`Processor`] trait is the interface the UI/executor layers call;
//!   [`DeterministicProcessor`] is the reference implementation. A future
//!   library-backed implementation can substitute behind the same interface
//!   without touching callers.
//! - [`apply_recipe`] executes a validated [`ProcessingRecipe`] op sequence
//!   on a [`GrayFrame`] using only the exact primitives from
//!   [`crate::image`]: quarter-turn rotation, strict crop, bilinear
//!   quadrilateral rectification, and integer fixed-point illumination
//!   correction. Every arithmetic step is a correctly-rounded IEEE-754
//!   single operation (`+ - * /`, `floor`, `round`) or pure integer math —
//!   never a transcendental or libm function whose result may differ
//!   between hosts. Applying the same recipe to the same frame therefore
//!   produces byte-identical output on any IEEE-754 platform, which is what
//!   the "stable output" criterion needs (verified by golden digests in
//!   `tests/golden_processing.rs`).
//! - Mapping model: quadrilateral rectification uses the bilinear corner
//!   map (the PDF /cropBox model). It is *exact* for parallelograms and a
//!   close approximation for general convex quads; a projective (homography)
//!   model is a possible future mapping tag, deliberately not smuggled in
//!   under `foldscan.recipe/0.1` because it would change output bytes.
//! - Input corruption is *detected, not crashed on*: frames with
//!   inconsistent payloads are rejected by [`GrayFrame::from_pixels`]
//!   bounds checks, recipes are re-validated before execution regardless of
//!   provenance, and malformed quadrilaterals are rejected with a clear
//!   user-level error rather than producing a garbled derivative.
//!
//! Scope boundaries: this core does not decode or encode JPEG/PNG, does not
//! run edge detection to *find* page corners, and does not dewarp. Those
//! are follow-up slices; the recipe vocabulary, export executor, and this
//! core already define the seams they plug into.
//!
//! Evidence category: deterministic software core exercised by unit tests
//! and golden-digest integration fixtures on synthetic frames. Not image
//! codec evidence, not physical device evidence.

use serde::Deserialize;

use crate::error::DomainError;
use crate::image::{crop, rotate, sample_bilinear, GrayFrame};
use crate::recipe::{OpKind, ProcessingRecipe, RecipeOp};

/// The processing interface: turn a source frame plus a validated recipe
/// into a derivative frame plus its content digest.
///
/// Implementations must be pure with respect to their inputs (no ambient
/// state, no clock, no filesystem) so results remain reviewable and
/// reproducible.
pub trait Processor {
    /// Apply `recipe` to `source`. The recipe is re-validated here; callers
    /// may validate earlier for UX, but the interface never trusts it.
    fn process(
        &self,
        source: &GrayFrame,
        recipe: &ProcessingRecipe,
    ) -> Result<ProcessedFrame, DomainError>;
}

/// Output of one processing run: the raw derivative frame plus its digest,
/// ready for host-codec encoding and export binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedFrame {
    pub frame: GrayFrame,
    /// Lowercase-hex frame digest (see [`GrayFrame::digest`]).
    pub digest: String,
}

/// Reference [`Processor`] implementation built only from the exact
/// primitives in [`crate::image`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicProcessor;

impl Processor for DeterministicProcessor {
    fn process(
        &self,
        source: &GrayFrame,
        recipe: &ProcessingRecipe,
    ) -> Result<ProcessedFrame, DomainError> {
        let frame = apply_recipe(source, recipe)?;
        let digest = frame.digest();
        Ok(ProcessedFrame { frame, digest })
    }
}

/// Execute a recipe's ops left to right. Re-validates the document first —
/// a recipe received over IPC or from disk may have been edited since the
/// UI last checked it.
pub fn apply_recipe(
    source: &GrayFrame,
    recipe: &ProcessingRecipe,
) -> Result<GrayFrame, DomainError> {
    recipe.validate()?;
    let mut frame = source.clone();
    for (i, op) in recipe.ops.iter().enumerate() {
        frame = apply_op(&frame, op).map_err(|e| {
            DomainError::new(
                e.category,
                format!("recipe operation {} ({:?}): {}", i, op.kind, e.message),
            )
        })?;
    }
    Ok(frame)
}

fn apply_op(frame: &GrayFrame, op: &RecipeOp) -> Result<GrayFrame, DomainError> {
    match op.kind {
        OpKind::Rotate => {
            #[derive(Deserialize)]
            struct RotateParams {
                #[serde(default)]
                degrees: u16,
            }
            let p: RotateParams = parse_params(op)?;
            let turns = match p.degrees {
                0 => 0,
                90 => 1,
                180 => 2,
                270 => 3,
                _ => {
                    return Err(DomainError::invalid_request(format!(
                        "degrees {} is not a quarter turn",
                        p.degrees
                    )))
                }
            };
            Ok(rotate(frame, turns))
        }
        OpKind::Crop => {
            #[derive(Deserialize)]
            struct CropParams {
                #[serde(default)]
                x: u32,
                #[serde(default)]
                y: u32,
                width: u32,
                height: u32,
            }
            let p: CropParams = parse_params(op)?;
            crop(frame, p.x, p.y, p.width, p.height)
        }
        OpKind::Perspective => {
            #[derive(Deserialize)]
            struct PerspectiveParams {
                corners: [[f64; 2]; 4],
                #[serde(default)]
                width: Option<u32>,
                #[serde(default)]
                height: Option<u32>,
            }
            let p: PerspectiveParams = parse_params(op)?;
            perspective(frame, &p.corners, p.width, p.height)
        }
        OpKind::Illumination => {
            #[derive(Deserialize)]
            struct IlluminationParams {
                #[serde(default)]
                strength: Option<f64>,
                #[serde(default)]
                mode: Option<String>,
            }
            let p: IlluminationParams = parse_params(op)?;
            let strength = p.strength.unwrap_or(1.0);
            if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
                return Err(DomainError::invalid_request(
                    "illumination strength must be within 0.0..=1.0",
                ));
            }
            match p.mode.as_deref().unwrap_or("flatten") {
                "flatten" => Ok(flatten_illumination(frame, strength)),
                "shadow_lift" => Ok(shadow_lift(frame, strength)),
                "balance" => Ok(auto_balance(frame)),
                other => Err(DomainError::invalid_request(format!(
                    "illumination mode {:?} is not implemented",
                    other
                ))),
            }
        }
        OpKind::Dewarp => Err(DomainError::invalid_request(
            "dewarp is recognized but not yet implemented; no output is produced",
        )),
    }
}

/// Deserialize one op's parameters into its typed shape. Missing params
/// mean defaults; a non-object was already rejected at validation.
fn parse_params<T>(op: &RecipeOp) -> Result<T, DomainError>
where
    T: serde::de::DeserializeOwned,
{
    if op.params.is_null() {
        return serde_json::from_str("{}").map_err(internal_on_parse);
    }
    serde_json::from_value(op.params.clone()).map_err(internal_on_parse)
}

/// Parameter shapes are re-derivable from the validated document, so a
/// typed parse failure here is an internal inconsistency, not a user error.
fn internal_on_parse(e: serde_json::Error) -> DomainError {
    DomainError::internal(format!("recipe params failed typed parse: {}", e))
}

/// Corner-order convention for [`perspective`]: the four quadrilateral
/// vertices in source order top-left, top-right, bottom-right, bottom-left.
const CORNER_NAMES: [&str; 4] = ["top-left", "top-right", "bottom-right", "bottom-left"];

/// Rectify the quadrilateral `corners` to a rectangular output by inverse
/// bilinear mapping: output pixel (ox, oy) samples the source at the
/// bilinear corner-map position of its normalized center (u, v).
///
/// Determinism: `u`, `v` are correctly-rounded divisions; each bilinear
/// weight and coordinate interpolation is a correctly-rounded `+ - * /`;
/// sampling and rounding are the fixed-point integer scheme in
/// [`crate::image`]. Convexity and non-degeneracy of the quad are validated
/// with exact integer shoelace/cross-product arithmetic on a 1/256-pixel
/// grid, so validation itself is host-independent.
///
/// Output dimensions default to the integer span of the quad (always
/// covering its whole pixel extent) and can be overridden by `width`/
/// `height`; both are bounds-checked like any frame allocation.
pub fn perspective(
    src: &GrayFrame,
    corners: &[[f64; 2]; 4],
    width: Option<u32>,
    height: Option<u32>,
) -> Result<GrayFrame, DomainError> {
    for (i, c) in corners.iter().enumerate() {
        if !c[0].is_finite() || !c[1].is_finite() || c[0] < 0.0 || c[1] < 0.0 {
            return Err(DomainError::invalid_request(format!(
                "perspective corner {} must be finite and non-negative",
                CORNER_NAMES[i]
            )));
        }
    }
    check_convex_non_degenerate(corners)?;

    let out_w = width.unwrap_or_else(|| span_width(corners)).max(2);
    let out_h = height.unwrap_or_else(|| span_height(corners)).max(2);
    crate::image::check_bounds(out_w, out_h)?;

    let [tl, tr, br, bl] = *corners;
    let mut out = GrayFrame {
        width: out_w,
        height: out_h,
        pixels: Vec::with_capacity((out_w as usize) * (out_h as usize)),
    };
    for oy in 0..out_h {
        let v = oy as f64 / (out_h - 1) as f64;
        for ox in 0..out_w {
            let u = ox as f64 / (out_w - 1) as f64;
            let top_x = lerp(tl[0], tr[0], u);
            let top_y = lerp(tl[1], tr[1], u);
            let bot_x = lerp(bl[0], br[0], u);
            let bot_y = lerp(bl[1], br[1], u);
            let fx = lerp(top_x, bot_x, v);
            let fy = lerp(top_y, bot_y, v);
            out.pixels.push(sample_bilinear(src, fx, fy));
        }
    }
    Ok(out)
}

/// Correctly-rounded f64 linear interpolation (two exact multiplies, one
/// exact add — each an IEEE-754 single operation).
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    (1.0 - t) * a + t * b
}

/// Quantize a coordinate to the 1/256-pixel integer grid (deterministic
/// single multiply + round).
fn q256(x: f64) -> i64 {
    (x * 256.0).round() as i64
}

/// Reject degenerate (zero-area) or non-convex quads using exact integer
/// arithmetic on the 1/256 grid: signed shoelace area must be non-zero and
/// every consecutive-edge cross product must share its sign (zero =
/// collinear, allowed).
fn check_convex_non_degenerate(corners: &[[f64; 2]; 4]) -> Result<(), DomainError> {
    let p: Vec<(i64, i64)> = corners.iter().map(|c| (q256(c[0]), q256(c[1]))).collect();

    let mut area2: i128 = 0;
    for i in 0..4 {
        let (x0, y0) = p[i];
        let (x1, y1) = p[(i + 1) % 4];
        area2 += (x0 as i128) * (y1 as i128) - (x1 as i128) * (y0 as i128);
    }
    if area2 == 0 {
        return Err(DomainError::invalid_request(
            "perspective quadrilateral has zero area",
        ));
    }
    let sign = area2 > 0;
    for i in 0..4 {
        let (x0, y0) = p[i];
        let (x1, y1) = p[(i + 1) % 4];
        let (x2, y2) = p[(i + 2) % 4];
        let cross =
            ((x1 - x0) as i128) * ((y2 - y1) as i128) - ((y1 - y0) as i128) * ((x2 - x1) as i128);
        if (cross < 0) == sign {
            return Err(DomainError::invalid_request(
                "perspective quadrilateral must be convex and non-degenerate",
            ));
        }
    }
    Ok(())
}

fn quad_extent(corners: &[[f64; 2]; 4]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for c in corners {
        min_x = min_x.min(c[0]);
        min_y = min_y.min(c[1]);
        max_x = max_x.max(c[0]);
        max_y = max_y.max(c[1]);
    }
    (min_x, min_y, max_x, max_y)
}

/// Default output width: the quad's integer span expanded to include the
/// whole pixel extent (ceil(max) - floor(min) + 1).
fn span_width(corners: &[[f64; 2]; 4]) -> u32 {
    let (min_x, _, max_x, _) = quad_extent(corners);
    (max_x.ceil() - min_x.floor() + 1.0).max(2.0) as u32
}

fn span_height(corners: &[[f64; 2]; 4]) -> u32 {
    let (_, min_y, _, max_y) = quad_extent(corners);
    (max_y.ceil() - min_y.floor() + 1.0).max(2.0) as u32
}

/// Global illumination correction: blend each pixel toward the estimated
/// white point by `strength`, in 1/256 fixed point with round-half-up.
/// `strength = 1.0` maps every pixel exactly onto the white point;
/// `strength = 0.0` is an exact passthrough.
pub fn flatten_illumination(src: &GrayFrame, strength: f64) -> GrayFrame {
    let white = estimate_illumination(src) as i64;
    let str_q = ((strength * 256.0).round() as i64).clamp(0, 256);
    let mut out = GrayFrame {
        width: src.width,
        height: src.height,
        pixels: Vec::with_capacity(src.pixels.len()),
    };
    for &p in &src.pixels {
        let p = p as i64;
        let v = (p * 256 + str_q * (white - p) + 128) >> 8;
        out.pixels.push(v.clamp(0, 255) as u8);
    }
    out
}

/// Shadow lift: pixels strictly darker than half the white point are
/// blended toward it by `strength`; brighter pixels are untouched. Pure
/// integer fixed point like [`flatten_illumination`].
pub fn shadow_lift(src: &GrayFrame, strength: f64) -> GrayFrame {
    let white = estimate_illumination(src) as i64;
    let half = white / 2;
    let str_q = ((strength * 256.0).round() as i64).clamp(0, 256);
    let mut out = GrayFrame {
        width: src.width,
        height: src.height,
        pixels: Vec::with_capacity(src.pixels.len()),
    };
    for &p in &src.pixels {
        let p = p as i64;
        let v = if p < half {
            (p * 256 + str_q * (half - p) + 128) >> 8
        } else {
            p
        };
        out.pixels.push(v.clamp(0, 255) as u8);
    }
    out
}

/// Auto white balance: scale so the frame mean maps onto mid-grey (128),
/// never darkening (gain >= 1.0), with integer fixed-point math.
pub fn auto_balance(src: &GrayFrame) -> GrayFrame {
    let total: u64 = src.pixels.iter().map(|&p| p as u64).sum();
    let n = src.pixels.len() as u64;
    // mean in 1/256 units, round-half-up, min 1 to avoid divide-by-zero.
    let mean_q = ((total * 256 + n / 2) / n).max(1);
    // gain in 1/256 units so that mean -> 128, floored at 256 (no darkening).
    let target_q: u64 = 128 * 256;
    let gain_q = ((target_q * 256 + mean_q / 2) / mean_q).max(256);
    let mut out = GrayFrame {
        width: src.width,
        height: src.height,
        pixels: Vec::with_capacity(src.pixels.len()),
    };
    for &p in &src.pixels {
        let v = (p as u64 * gain_q + 128) >> 8;
        out.pixels.push(v.min(255) as u8);
    }
    out
}

/// 8x8 mean-pooled illumination estimate with 3x3 box smoothing and an
/// integer 95th-percentile white point. Frames smaller than 8x8 fall back
/// to the plain frame maximum. All pool arithmetic is integer
/// round-half-up; the percentile uses exact integer index math
/// (`len * 19 / 20`) so no floating-point boundary can shift the pick.
pub fn estimate_illumination(src: &GrayFrame) -> u8 {
    const POOL: usize = 8;
    if src.width < POOL as u32 || src.height < POOL as u32 {
        return src.pixels.iter().copied().max().unwrap_or(0);
    }
    let gw = (src.width as usize).div_ceil(POOL);
    let gh = (src.height as usize).div_ceil(POOL);
    let mut cells = vec![0u128; gw * gh];
    let mut cnts = vec![0u32; gw * gh];
    for (y, row) in src.pixels.chunks(src.width as usize).enumerate() {
        let cy = (y / POOL).min(gh - 1);
        for (x, &v) in row.iter().enumerate() {
            let cx = (x / POOL).min(gw - 1);
            let i = cy * gw + cx;
            cells[i] += v as u128;
            cnts[i] += 1;
        }
    }
    let means: Vec<u32> = cells
        .iter()
        .zip(&cnts)
        .map(|(&s, &c)| {
            let c = c as u128;
            ((s + c / 2) / c).min(255) as u32
        })
        .collect();

    let mut sm = vec![0u32; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let mut acc: u64 = 0;
            let mut n: u64 = 0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let ny = gy as i32 + dy;
                    let nx = gx as i32 + dx;
                    if ny < 0 || nx < 0 {
                        continue;
                    }
                    let (ny, nx) = (ny as usize, nx as usize);
                    if ny >= gh || nx >= gw {
                        continue;
                    }
                    acc += means[ny * gw + nx] as u64;
                    n += 1;
                }
            }
            sm[gy * gw + gx] = ((acc + n / 2) / n.max(1)) as u32;
        }
    }

    sm.sort_unstable();
    let idx = (sm.len() * 19 / 20).min(sm.len() - 1);
    sm[idx].clamp(1, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::RecipeOp;
    use serde_json::json;

    fn frame(w: u32, h: u32, pixels: Vec<u8>) -> GrayFrame {
        GrayFrame::from_pixels(w, h, pixels).expect("fixture")
    }

    fn recipe(ops: Vec<(&str, serde_json::Value)>) -> ProcessingRecipe {
        ProcessingRecipe {
            schema: "foldscan.recipe/0.1".to_string(),
            name: "unit".to_string(),
            ops: ops
                .into_iter()
                .map(|(k, v)| RecipeOp {
                    kind: match k {
                        "rotate" => OpKind::Rotate,
                        "crop" => OpKind::Crop,
                        "perspective" => OpKind::Perspective,
                        "illumination" => OpKind::Illumination,
                        _ => OpKind::Dewarp,
                    },
                    params: v,
                })
                .collect(),
        }
    }

    #[test]
    fn empty_recipe_is_identity() {
        let src = frame(3, 2, vec![10, 20, 30, 40, 50, 60]);
        let out = apply_recipe(&src, &recipe(vec![])).expect("identity");
        assert_eq!(out, src);
    }

    #[test]
    fn ops_compose_left_to_right() {
        let src = frame(4, 2, vec![10, 20, 30, 40, 50, 60, 70, 80]);
        let r = recipe(vec![
            ("rotate", json!({"degrees": 90})),
            ("illumination", json!({"mode": "flatten", "strength": 0.0})),
        ]);
        let out = apply_recipe(&src, &r).expect("applied");
        // strength 0 must be a pure passthrough of the rotated frame.
        assert_eq!(out, rotate(&src, 1));
    }

    #[test]
    fn strength_zero_flatten_is_exact_passthrough() {
        let src = frame(10, 2, vec![5; 20]);
        assert_eq!(flatten_illumination(&src, 0.0), src);
    }

    #[test]
    fn flatten_full_strength_maps_to_white_point() {
        let mut px = vec![40u8; 16 * 16];
        px[0] = 250;
        let src = frame(16, 16, px);
        let white = estimate_illumination(&src);
        assert!(white > 40);
        let out = flatten_illumination(&src, 1.0);
        assert!(out.pixels.iter().all(|&p| p == white));
    }

    #[test]
    fn shadow_lift_only_touches_dark_pixels() {
        let mut px = vec![200u8; 16 * 16];
        px[8 * 16 + 8] = 5;
        let src = frame(16, 16, px);
        let out = shadow_lift(&src, 1.0);
        // Bright majority unchanged; the dark pixel lifted below half-white.
        assert_eq!(out.at(0, 0), 200);
        assert!(out.at(8, 8) > 5);
    }

    #[test]
    fn auto_balance_maps_uniform_mean_to_midgrey() {
        let src = frame(4, 4, vec![50; 16]);
        let out = auto_balance(&src);
        assert!(out.pixels.iter().all(|&p| p == out.at(0, 0)));
        assert!(out.at(0, 0) >= 126 && out.at(0, 0) <= 128);
    }

    #[test]
    fn auto_balance_never_darkens() {
        let src = frame(4, 4, vec![200; 16]);
        let out = auto_balance(&src);
        assert_eq!(out, src);
    }

    #[test]
    fn crop_op_rejects_out_of_bounds_window() {
        let src = frame(4, 4, vec![0; 16]);
        let r = recipe(vec![(
            "crop",
            json!({"x": 2, "y": 0, "width": 4, "height": 4}),
        )]);
        let err = apply_recipe(&src, &r).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
        assert!(err.message.contains("exceeds source"));
    }

    #[test]
    fn identity_perspective_preserves_a_flat_frame() {
        let src = frame(8, 8, vec![123; 64]);
        let corners = [[0.0, 0.0], [7.0, 0.0], [7.0, 7.0], [0.0, 7.0]];
        let out = perspective(&src, &corners, Some(4), Some(4)).expect("flat");
        assert_eq!(out.pixels, vec![123; 16]);
    }

    #[test]
    fn perspective_integer_lattice_hits_exact_source_pixels() {
        let mut px = vec![0u8; 8 * 8];
        for (i, p) in px.iter_mut().enumerate() {
            *p = (i * 3 % 251) as u8;
        }
        let src = frame(8, 8, px);
        // Integer-aligned rect 1..=6 mapped to 6x6: every output pixel must
        // equal the corresponding source pixel exactly (integer coordinates
        // make sampling weights exact).
        let corners = [[1.0, 1.0], [6.0, 1.0], [6.0, 6.0], [1.0, 6.0]];
        let out = perspective(&src, &corners, Some(6), Some(6)).expect("rect");
        for y in 0..6u32 {
            for x in 0..6u32 {
                assert_eq!(out.at(x, y), src.at(x + 1, y + 1));
            }
        }
    }

    #[test]
    fn perspective_rejects_degenerate_and_concave_quads() {
        let src = frame(8, 8, vec![0; 64]);
        // All corners identical: zero area.
        let err = perspective(&src, &[[2.0, 2.0]; 4], None, None).unwrap_err();
        assert!(err.message.contains("zero area"));
        // Concave quad (third vertex dented inward): rejected by convexity.
        let concave = [[1.0, 1.0], [6.0, 1.0], [3.0, 3.0], [1.0, 6.0]];
        let err = perspective(&src, &concave, None, None).unwrap_err();
        assert!(err.message.contains("convex"));
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn perspective_rejects_non_finite_corners() {
        let src = frame(8, 8, vec![0; 64]);
        let bad = [[0.0, 0.0], [f64::NAN, 1.0], [6.0, 6.0], [1.0, 6.0]];
        let err = perspective(&src, &bad, None, None).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn perspective_output_is_repeatable() {
        let mut px = vec![0u8; 16 * 16];
        for (i, p) in px.iter_mut().enumerate() {
            *p = (i % 251) as u8;
        }
        let src = frame(16, 16, px);
        let corners = [[3.0, 0.0], [12.0, 3.0], [9.0, 12.0], [0.0, 9.0]];
        let a = perspective(&src, &corners, Some(6), Some(6)).expect("quad");
        let b = perspective(&src, &corners, Some(6), Some(6)).expect("quad again");
        assert_eq!((a.width, a.height), (6, 6));
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn default_output_span_covers_quad_extent() {
        let src = frame(16, 16, vec![7; 256]);
        let corners = [[2.5, 1.25], [13.5, 2.0], [12.0, 14.75], [1.0, 13.0]];
        let out = perspective(&src, &corners, None, None).expect("span");
        // ceil(13.5)-floor(1.0)+1 = 14 ; ceil(14.75)-floor(1.25)+1 = 15
        assert_eq!((out.width, out.height), (14, 15));
    }

    #[test]
    fn dewarp_reports_not_implemented() {
        let src = frame(4, 4, vec![1; 16]);
        let r = recipe(vec![("dewarp", json!({"axes": "x"}))]);
        let err = apply_recipe(&src, &r).unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
        assert!(err.message.contains("not yet implemented"));
    }

    #[test]
    fn unknown_illumination_mode_reports_not_implemented() {
        let src = frame(4, 4, vec![1; 16]);
        let r = recipe(vec![(
            "illumination",
            json!({"mode": "saturate", "strength": 1.0}),
        )]);
        // mode "saturate" passes closed-vocabulary validation? No: recipe
        // validation only admits flatten/balance/shadow_lift, so this fails
        // at validate() before execution.
        let err = apply_recipe(&src, &r).unwrap_err();
        assert!(err.message.contains("not a known value"));
    }

    #[test]
    fn op_errors_are_attributed_with_index_and_kind() {
        let src = frame(4, 4, vec![1; 16]);
        let r = recipe(vec![
            ("rotate", json!({"degrees": 90})),
            ("crop", json!({"x": 0, "y": 0, "width": 99, "height": 99})),
        ]);
        let err = apply_recipe(&src, &r).unwrap_err();
        assert!(err.message.starts_with("recipe operation 1 (Crop)"));
    }

    #[test]
    fn processor_interface_binds_frame_digest() {
        let src = frame(16, 16, vec![200; 256]);
        let r = recipe(vec![("illumination", json!({"strength": 1.0}))]);
        let p = DeterministicProcessor;
        let out = p.process(&src, &r).expect("processed");
        assert_eq!(out.digest, out.frame.digest());
        assert_eq!(out.digest.len(), 64);
    }
}
