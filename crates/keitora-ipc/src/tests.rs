// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

#[test]
fn the_limits_are_the_documented_ones() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(MAX_PAGE_LIMIT, 100);
    assert_eq!(MAX_STOP_TEXT_BYTES, 1024);
    assert_eq!(MAX_FRAME_LEN, 8 * 1024 * 1024);
}

#[test]
fn a_request_survives_encode_reader_decode() {
    let id = MessageId::new("a-1").expect("a valid id");
    let envelope = Envelope::new(id, Message::Request(Request::Subscribe));
    let bytes = encode(&envelope).expect("encodes");
    let frames = FrameReader::new().push(&bytes);
    let frame = frames.into_iter().next().expect("one frame").expect("ok");
    assert_eq!(decode(&frame).expect("decodes"), envelope);
}
