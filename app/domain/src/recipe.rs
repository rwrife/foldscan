//! Versioned, reviewable processing recipes.
//!
//! The companion's processing profiles are reviewable/exportable JSON with
//! conservative defaults (app/README § Setup flow) and a versioned portable
//! contract (docs/protocol.md principles). This module models *one half* of
//! that contract: the recipe document itself.
//!
//! Design rules:
//! - A recipe is a closed, versioned vocabulary. An unknown operation kind is
//!   a hard `InvalidRequest`, not a silently skipped step — applying a partial
//!   recipe would produce an image the user did not review.
//! - Bounds are enforced on the raw serialized form *before* trust: total
//!   recipe bytes, operation count, parameter string lengths, and JSON depth.
//! - A recipe's `digest()` is the lowercase-hex SHA-256 of its canonical
//!   serialization, so an exported page can be bound to the exact recipe that
//!   produced it (see [`crate::export`]).
//!
//! This module deliberately performs no image work. Applying a recipe is a
//! processing-pipeline concern (a later slice behind the processing
//! interface); here a recipe is validated data only.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::checksum::sha256_hex;
use crate::error::DomainError;
use crate::version::{parse_tag_version, ProtocolVersion};

/// Schema tag prefix for recipe documents (`foldscan.recipe/<major>.<minor>`).
pub const RECIPE_SCHEMA_PREFIX: &str = "foldscan.recipe/";

/// Maximum raw JSON bytes for one recipe document.
pub const MAX_RECIPE_BYTES: usize = 16 * 1024;

/// Maximum operations in one recipe.
pub const MAX_RECIPE_OPS: usize = 32;

/// Maximum length of any string inside recipe parameters.
pub const MAX_RECIPE_STRING_LEN: usize = 512;

/// Maximum nesting depth of a parameter JSON value.
pub const MAX_PARAM_DEPTH: u32 = 4;

/// The closed set of processing operations the vocabulary understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// Rotate by a quarter-turn multiple (90/180/270).
    Rotate,
    /// Rectangular crop in source pixel coordinates.
    Crop,
    /// Corner-selection homography to a rectangular output.
    Perspective,
    /// Illumination/shading flattening.
    Illumination,
    /// Curvature/crease dewarp.
    Dewarp,
}

/// One ordered operation in a recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeOp {
    pub kind: OpKind,
    #[serde(default)]
    pub params: Value,
}

/// A named, versioned processing recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessingRecipe {
    /// Schema tag, e.g. `foldscan.recipe/0.1`.
    pub schema: String,
    /// Human-facing recipe name (bounded, no control characters).
    pub name: String,
    /// Ordered operations applied left to right.
    #[serde(default)]
    pub ops: Vec<RecipeOp>,
}

impl ProcessingRecipe {
    /// Validate a recipe document against the closed vocabulary and bounds.
    pub fn validate(&self) -> Result<ProtocolVersion, DomainError> {
        let version = parse_tag_version(RECIPE_SCHEMA_PREFIX, &self.schema, "recipe schema")?;

        if self.name.is_empty() {
            return Err(DomainError::invalid_request("recipe name is empty"));
        }
        if self.name.len() > crate::limits::MAX_ID_LEN {
            return Err(DomainError::invalid_request(format!(
                "recipe name exceeds {} bytes",
                crate::limits::MAX_ID_LEN
            )));
        }
        if self.name.chars().any(|c| c == '\0' || c == '/') {
            return Err(DomainError::invalid_request(
                "recipe name contains forbidden characters",
            ));
        }

        if self.ops.len() > MAX_RECIPE_OPS {
            return Err(DomainError::invalid_request(format!(
                "recipe has {} operations, over the {} limit",
                self.ops.len(),
                MAX_RECIPE_OPS
            )));
        }
        for (i, op) in self.ops.iter().enumerate() {
            validate_params(i, op.kind, &op.params)?;
        }
        Ok(version)
    }

    /// Validate the raw serialized form *before* trusting it, then validate
    /// the parsed document. This is the entry point for untrusted recipe
    /// files imported from disk.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, DomainError> {
        if bytes.is_empty() {
            return Err(DomainError::invalid_request("recipe document is empty"));
        }
        if bytes.len() > MAX_RECIPE_BYTES {
            return Err(DomainError::invalid_request(format!(
                "recipe exceeds {} byte bound",
                MAX_RECIPE_BYTES
            )));
        }
        // Reject oversized embedded strings before serde allocates them.
        let raw = std::str::from_utf8(bytes)
            .map_err(|_| DomainError::invalid_request("recipe is not valid UTF-8"))?;
        let approx_strings: usize = raw.match_indices('"').count() / 2;
        if approx_strings * 2 > MAX_RECIPE_BYTES {
            return Err(DomainError::invalid_request(
                "recipe contains an implausible number of strings",
            ));
        }
        let recipe: ProcessingRecipe = serde_json::from_str(raw).map_err(|e| {
            let cut = e.to_string();
            let cut = cut.split(" at line").next().unwrap_or(&cut);
            DomainError::invalid_request(format!("recipe JSON invalid: {}", cut))
        })?;
        recipe.validate()?;
        Ok(recipe)
    }

    /// Deterministic lowercase-hex SHA-256 over a canonical serialization of
    /// this recipe (sorted object keys, sorted top-level fields), suitable for
    /// binding exported pages to the recipe that produced them.
    pub fn digest(&self) -> String {
        let canonical = canonical_json(&self.to_value());
        sha256_hex(canonical.as_bytes())
    }

    fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("recipe serialization is infallible")
    }
}

/// Recursively sort map keys so the digest is stable across map iterations.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", quote(k), canonical_json(&map[*k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Validate one operation's parameter object against the per-kind shape.
/// Parameters must be a JSON object; kind-specific checks cover the fields
/// the (future) processing pipeline would read, so an invalid recipe fails
/// at review time, not at render time.
fn validate_params(index: usize, kind: OpKind, params: &Value) -> Result<(), DomainError> {
    let ctx = format!("recipe operation {} ({:?})", index, kind);
    if params.is_null() {
        return Ok(()); // null params are treated as absent (defaults)
    }
    let obj = params
        .as_object()
        .ok_or_else(|| DomainError::invalid_request(format!("{} params must be an object", ctx)))?;

    check_depth(index, 0, params)?;

    match kind {
        OpKind::Rotate => {
            let deg = get_number(obj, "degrees", &ctx)?;
            if let Some(deg) = deg {
                if !matches!(deg, 0.0 | 90.0 | 180.0 | 270.0) {
                    return Err(DomainError::invalid_request(format!(
                        "{} degrees must be 0, 90, 180 or 270",
                        ctx
                    )));
                }
            }
        }
        OpKind::Crop => {
            for key in ["x", "y", "width", "height"] {
                let v = get_number(obj, key, &ctx)?.ok_or_else(|| {
                    DomainError::invalid_request(format!("{} missing {}", ctx, key))
                })?;
                if v < 0.0 {
                    return Err(DomainError::invalid_request(format!(
                        "{} {} must be non-negative",
                        ctx, key
                    )));
                }
                if (key == "width" || key == "height") && v == 0.0 {
                    return Err(DomainError::invalid_request(format!(
                        "{} {} must be positive",
                        ctx, key
                    )));
                }
            }
        }
        OpKind::Perspective => {
            let corners = obj
                .get("corners")
                .ok_or_else(|| DomainError::invalid_request(format!("{} missing corners", ctx)))?;
            let list = corners.as_array().ok_or_else(|| {
                DomainError::invalid_request(format!("{} corners must be an array", ctx))
            })?;
            if list.len() != 4 {
                return Err(DomainError::invalid_request(format!(
                    "{} corners must list exactly 4 points",
                    ctx
                )));
            }
            for (i, pt) in list.iter().enumerate() {
                let pair = pt.as_array().ok_or_else(|| {
                    DomainError::invalid_request(format!(
                        "{} corner {} must be a [x, y] pair",
                        ctx, i
                    ))
                })?;
                if pair.len() != 2 || !pair.iter().all(|v| v.is_number()) {
                    return Err(DomainError::invalid_request(format!(
                        "{} corner {} must be two numbers",
                        ctx, i
                    )));
                }
                for v in pair {
                    if v.as_f64().map(|f| f < 0.0).unwrap_or(true) {
                        return Err(DomainError::invalid_request(format!(
                            "{} corner {} coordinates must be non-negative",
                            ctx, i
                        )));
                    }
                }
            }
        }
        OpKind::Illumination => {
            if let Some(strength) = obj.get("strength") {
                let s = strength.as_f64().ok_or_else(|| {
                    DomainError::invalid_request(format!("{} strength must be a number", ctx))
                })?;
                if !(0.0..=1.0).contains(&s) {
                    return Err(DomainError::invalid_request(format!(
                        "{} strength must be within 0.0..=1.0",
                        ctx
                    )));
                }
            }
            if let Some(mode) = obj.get("mode") {
                let m = mode.as_str().ok_or_else(|| {
                    DomainError::invalid_request(format!("{} mode must be a string", ctx))
                })?;
                if !matches!(m, "flatten" | "balance" | "shadow_lift") {
                    return Err(DomainError::invalid_request(format!(
                        "{} mode {:?} is not a known value",
                        ctx, m
                    )));
                }
                check_string_len(index, m)?;
            }
        }
        OpKind::Dewarp => {
            if let Some(axes) = obj.get("axes") {
                let a = axes.as_str().ok_or_else(|| {
                    DomainError::invalid_request(format!("{} axes must be a string", ctx))
                })?;
                if !matches!(a, "x" | "y" | "both") {
                    return Err(DomainError::invalid_request(format!(
                        "{} axes {:?} is not a known value",
                        ctx, a
                    )));
                }
            }
        }
    }
    Ok(())
}

fn get_number(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    ctx: &str,
) -> Result<Option<f64>, DomainError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| {
            DomainError::invalid_request(format!("{} {} must be a number", ctx, key))
        }),
    }
}

fn check_depth(index: usize, depth: u32, value: &Value) -> Result<(), DomainError> {
    if depth > MAX_PARAM_DEPTH {
        return Err(DomainError::invalid_request(format!(
            "recipe operation {} parameters nest deeper than {} levels",
            index, MAX_PARAM_DEPTH
        )));
    }
    match value {
        Value::Object(map) => {
            for (_, v) in map {
                check_depth(index, depth + 1, v)?;
            }
        }
        Value::Array(items) => {
            for v in items {
                check_depth(index, depth + 1, v)?;
            }
        }
        Value::String(s) => check_string_len(index, s)?,
        _ => {}
    }
    Ok(())
}

fn check_string_len(index: usize, s: &str) -> Result<(), DomainError> {
    if s.len() > MAX_RECIPE_STRING_LEN {
        return Err(DomainError::invalid_request(format!(
            "recipe operation {} contains a string over {} bytes",
            index, MAX_RECIPE_STRING_LEN
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(json: &str) -> ProcessingRecipe {
        ProcessingRecipe::from_json_bytes(json.as_bytes()).expect("fixture should parse")
    }

    #[test]
    fn valid_recipe_passes_and_digest_is_stable() {
        let r = recipe(
            r#"{"schema":"foldscan.recipe/0.1","name":"default","ops":[
                {"kind":"rotate","params":{"degrees":90}},
                {"kind":"crop","params":{"x":10,"y":10,"width":2000,"height":2800}},
                {"kind":"perspective","params":{"corners":[[0,0],[2000,0],[2000,2800],[0,2800]]}},
                {"kind":"illumination","params":{"mode":"flatten","strength":0.6}},
                {"kind":"dewarp","params":{"axes":"both"}}
            ]}"#,
        );
        assert_eq!(r.ops.len(), 5);
        // Same content built independently digests identically.
        let r2 = recipe(&serde_json::to_string(&r).unwrap());
        assert_eq!(r.digest(), r2.digest());
        assert_eq!(r.digest().len(), 64);
    }

    #[test]
    fn digest_changes_with_content() {
        let a = recipe(
            r#"{"schema":"foldscan.recipe/0.1","name":"a","ops":[{"kind":"rotate","params":{"degrees":90}}]}"#,
        );
        let b = recipe(
            r#"{"schema":"foldscan.recipe/0.1","name":"a","ops":[{"kind":"rotate","params":{"degrees":180}}]}"#,
        );
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn unknown_op_kind_is_rejected() {
        // serde rejects the closed enum with InvalidRequest before validate().
        let err = ProcessingRecipe::from_json_bytes(
            br#"{"schema":"foldscan.recipe/0.1","name":"x","ops":[{"kind":"deepfried"}]}"#,
        )
        .unwrap_err();
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn bad_major_version_is_rejected() {
        let err = recipe_err(r#"{"schema":"foldscan.recipe/9.1","name":"x","ops":[]}"#);
        assert_eq!(
            err.category,
            crate::error::Category::UnsupportedVersion,
            "unknown major must fail as version, not structure"
        );
    }

    #[test]
    fn newer_minor_is_accepted() {
        let r = recipe(r#"{"schema":"foldscan.recipe/0.9","name":"x","ops":[]}"#);
        assert!(r.validate().is_ok());
    }

    #[test]
    fn rotate_rejects_non_quarter_turn() {
        let err = recipe_err(
            r#"{"schema":"foldscan.recipe/0.1","name":"x","ops":[{"kind":"rotate","params":{"degrees":45}}]}"#,
        );
        assert_eq!(err.category, crate::error::Category::InvalidRequest);
    }

    #[test]
    fn crop_requires_positive_extent() {
        let err = recipe_err(
            r#"{"schema":"foldscan.recipe/0.1","name":"x","ops":[{"kind":"crop","params":{"x":0,"y":0,"width":0,"height":100}}]}"#,
        );
        assert!(err.message.contains("positive"));
    }

    #[test]
    fn perspective_needs_four_points() {
        let err = recipe_err(
            r#"{"schema":"foldscan.recipe/0.1","name":"x","ops":[{"kind":"perspective","params":{"corners":[[0,0],[1,0],[0,1]]}}]}"#,
        );
        assert!(err.message.contains("4 points"));
    }

    #[test]
    fn illumination_strength_out_of_range() {
        let err = recipe_err(
            r#"{"schema":"foldscan.recipe/0.1","name":"x","ops":[{"kind":"illumination","params":{"strength":2}}]}"#,
        );
        assert!(err.message.contains("0.0..=1.0"));
    }

    #[test]
    fn oversize_recipe_bytes_rejected() {
        let mut s = format!(
            r#"{{"schema":"foldscan.recipe/0.1","name":"{}","ops":[]"#,
            "n".repeat(20_000)
        );
        s.push('}');
        let err = recipe_err(&s);
        assert!(err.message.contains("byte bound"));
    }

    #[test]
    fn deep_nesting_rejected() {
        let err = recipe_err(
            r#"{"schema":"foldscan.recipe/0.1","name":"x","ops":[{"kind":"dewarp","params":{"a":[[[[["too deep"]]]]]}}]}"#,
        );
        assert!(err.message.contains("nest"));
    }

    #[test]
    fn too_many_ops_rejected() {
        let ops = vec![r#"{"kind":"rotate","params":{"degrees":90}}"#; MAX_RECIPE_OPS + 1];
        let json = format!(
            r#"{{"schema":"foldscan.recipe/0.1","name":"x","ops":[{}]}}"#,
            ops.join(",")
        );
        let err = recipe_err(&json);
        assert!(err.message.contains("operations"));
    }

    fn recipe_err(json: &str) -> DomainError {
        ProcessingRecipe::from_json_bytes(json.as_bytes()).unwrap_err()
    }
}
