// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

#[test]
fn the_limit_fits_in_a_u32() {
    assert_eq!(
        usize::try_from(max_frame_len_u32()).expect("fits"),
        MAX_FRAME_LEN
    );
}

#[test]
fn a_line_split_over_the_limit_across_pushes_is_reported_once() {
    let mut reader = FrameReader::new();
    let half = vec![b'a'; MAX_FRAME_LEN / 2 + 1];
    assert!(reader.push(&half).is_empty());
    assert_eq!(reader.push(&half), vec![Err(frame_too_large())]);
    assert_eq!(reader.buffered_len(), 0);
    assert!(reader.push(&half).is_empty());
    assert_eq!(reader.push(b"\nok\n"), vec![Ok(b"ok".to_vec())]);
}

#[test]
fn a_buffered_start_plus_a_tail_over_the_limit_is_refused() {
    let mut reader = FrameReader::new();
    assert!(reader.push(&vec![b'a'; MAX_FRAME_LEN]).is_empty());
    assert_eq!(reader.push(b"b\n"), vec![Err(frame_too_large())]);
    assert_eq!(reader.buffered_len(), 0);
}
