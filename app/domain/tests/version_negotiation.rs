//! Version negotiation tests: unknown major fails safe, newer minor is
//! tolerated when required fields remain, unknown fields are ignored.

use foldscan_domain::manifest::{DeviceManifest, SessionManifest};
use foldscan_domain::version::{parse_session_schema, ProtocolVersion};

fn device(json: &str) -> DeviceManifest {
    serde_json::from_str(json).unwrap()
}

#[test]
fn supported_major_accepted() {
    let d = device(
        r#"{"protocol":{"major":0,"minor":1},"device_id":"d1","firmware_version":"dev","capabilities":[]}"#,
    );
    assert!(d.validate().is_ok());
}

#[test]
fn newer_minor_accepted() {
    let d = device(
        r#"{"protocol":{"major":0,"minor":99},"device_id":"d1","firmware_version":"dev","capabilities":[]}"#,
    );
    assert!(d.validate().is_ok(), "newer minor must remain compatible");
}

#[test]
fn unknown_major_rejected() {
    let d = device(
        r#"{"protocol":{"major":1,"minor":0},"device_id":"d1","firmware_version":"dev","capabilities":[]}"#,
    );
    let err = d.validate().unwrap_err();
    assert_eq!(err.category, foldscan_domain::Category::UnsupportedVersion);
}

#[test]
fn unknown_extra_fields_tolerated() {
    let d = device(
        r#"{"protocol":{"major":0,"minor":1},"device_id":"d1","firmware_version":"dev","capabilities":[],"future_field":{"a":1}}"#,
    );
    assert!(d.validate().is_ok());
}

#[test]
fn session_schema_parses_and_validates() {
    let v = parse_session_schema("foldscan.session/0.1").unwrap();
    assert_eq!(v, ProtocolVersion { major: 0, minor: 1 });
    assert!(parse_session_schema("foldscan.session/9.0").is_err());
    assert!(parse_session_schema("other/0.1").is_err());
    assert!(parse_session_schema("foldscan.session/0").is_err());
    assert!(parse_session_schema("foldscan.session/x.y").is_err());
}

#[test]
fn missing_required_field_is_invalid_not_panic() {
    // `schema` missing => serde error classified as invalid_request upstream.
    let res: Result<SessionManifest, _> = serde_json::from_str(r#"{"captures":[]}"#);
    assert!(res.is_err());
}

#[test]
fn version_struct_from_json() {
    let v: ProtocolVersion = serde_json::from_str(r#"{"major":0,"minor":2}"#).unwrap();
    assert!(v.validate().is_ok());
}
