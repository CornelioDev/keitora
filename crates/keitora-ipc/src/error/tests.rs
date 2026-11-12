// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

#[test]
fn an_unknown_core_problem_is_invalid() {
    assert_eq!(
        ValidationKind::from(ValidationErrorKind::TooLong { len: 9, max: 1 }),
        ValidationKind::TooLong
    );
}

#[test]
fn a_rejection_without_an_id_answers_as_unknown() {
    let response = DecodeError::new(ProtocolError::Malformed, None).into_response();
    assert_eq!(response.id().as_str(), "unknown");
    assert_eq!(
        response.message(),
        &Message::Response(Response::Error {
            error: ProtocolError::Malformed
        })
    );
}

#[test]
fn display_never_shows_more_than_the_error() {
    let error = DecodeError::new(ProtocolError::Malformed, None);
    assert_eq!(error.to_string(), "rejected frame: malformed message");
}
