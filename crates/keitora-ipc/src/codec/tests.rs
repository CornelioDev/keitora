// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

#[test]
fn only_whole_numbers_other_than_one_are_a_newer_version() {
    let version = |text: &str| {
        let raw = RawValue::from_string(text.to_owned()).expect("valid JSON");
        check_version(Some(&raw))
    };
    assert_eq!(version("1"), Ok(()));
    assert!(matches!(
        version("99999999999999999999999"),
        Err(ProtocolError::UnsupportedVersion { .. })
    ));
    assert_eq!(version("1.0"), Err(ProtocolError::Malformed));
    assert_eq!(version("-1"), Err(ProtocolError::Malformed));
    assert_eq!(check_version(None), Err(ProtocolError::Malformed));
}

#[test]
fn deep_nesting_is_malformed_not_a_crash() {
    let deep = format!(
        r#"{{"v":1,"id":"a","kind":"request","payload":{}}}"#,
        "[".repeat(100_000)
    );
    let error = decode(deep.as_bytes()).expect_err("rejected");
    assert_eq!(error.error(), &ProtocolError::Malformed);
}

#[test]
fn a_missing_id_is_malformed_without_an_id() {
    let error = decode(br#"{"v":1,"kind":"request","payload":{"type":"get_state"}}"#)
        .expect_err("rejected");
    assert_eq!(error.error(), &ProtocolError::Malformed);
    assert_eq!(error.id(), None);
}
