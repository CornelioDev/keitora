// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

#[test]
fn ids_report_the_first_problem() {
    assert_eq!(check_id("", 4), Err(ValidationKind::Empty));
    assert_eq!(check_id("abcde", 4), Err(ValidationKind::TooLong));
    assert_eq!(check_id("a\nb", 4), Err(ValidationKind::ControlCharacter));
    assert_eq!(check_id("a_b", 4), Err(ValidationKind::NotAnId));
    assert_eq!(check_id("a-1", 4), Ok(()));
}

#[test]
fn the_unknown_id_is_a_valid_id() {
    assert_eq!(
        MessageId::new(UNKNOWN_ID).expect("valid"),
        MessageId::unknown()
    );
}

#[test]
fn a_bad_message_id_is_a_validation_error_on_id() {
    assert_eq!(
        MessageId::new("a b"),
        Err(ProtocolError::validation("id", ValidationKind::NotAnId))
    );
}

#[test]
fn a_bad_page_is_a_validation_error_on_the_limit() {
    assert_eq!(
        PageRequest::new(0, 0),
        Err(ProtocolError::validation(
            "page.limit",
            ValidationKind::OutOfRange
        ))
    );
}
