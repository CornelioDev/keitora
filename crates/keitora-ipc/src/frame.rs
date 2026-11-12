// SPDX-License-Identifier: MIT OR Apache-2.0

//! Line framing: cutting a stream of bytes into frames, with a hard limit on
//! the size of one.

use crate::error::ProtocolError;

/// The most bytes a frame may have, not counting the `\n` that ends it.
///
/// 8 MiB. The largest valid blocklist (500 targets, each with an application
/// name and four identifiers at their maximums, and 200 exceptions) has about
/// 2.3 MB of JSON, and up to twice that if every character needs an escape;
/// 8 MiB holds it with room to spare, and equals the biggest value the
/// service's store reads (`SQLITE_LIMIT_LENGTH`). It is also the most memory a
/// connection's reader holds. See ADR-0014.
pub const MAX_FRAME_LEN: usize = 8 * 1024 * 1024;

/// `MAX_FRAME_LEN` as the number a [`ProtocolError::FrameTooLarge`] carries.
/// The limit fits in a `u32`, which a unit test checks.
pub(crate) fn max_frame_len_u32() -> u32 {
    u32::try_from(MAX_FRAME_LEN).unwrap_or(u32::MAX)
}

/// The error for a frame over the limit.
pub(crate) fn frame_too_large() -> ProtocolError {
    ProtocolError::FrameTooLarge {
        max: max_frame_len_u32(),
    }
}

/// Cuts a stream of bytes into frames, one per line.
///
/// Feed it what each read returns, in order, with [`push`](Self::push). It
/// never holds more than [`MAX_FRAME_LEN`] bytes: when a line passes the
/// limit it reports [`ProtocolError::FrameTooLarge`] once, drops what it has,
/// and ignores the rest of that line without storing it, until the next
/// newline.
///
/// A frame is the bytes of a line, whatever they are: the reader passes NULs
/// and invalid UTF-8 on to [`decode`](crate::decode), which rejects them.
#[derive(Debug, Default)]
pub struct FrameReader {
    /// The start of the line in progress.
    buffer: Vec<u8>,
    /// Whether the line in progress is over the limit, so its rest is skipped.
    skipping: bool,
}

impl FrameReader {
    /// A reader at the start of a stream.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many bytes of an unfinished line the reader holds. Never more than
    /// [`MAX_FRAME_LEN`].
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Adds bytes from the stream and returns, in order, each line they
    /// complete: the frame without its `\n`, or the error for a line over the
    /// limit. Empty lines are skipped.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Vec<u8>, ProtocolError>> {
        let mut results = Vec::new();
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|&byte| byte == b'\n') {
            let (line, after) = rest.split_at_checked(newline).unwrap_or((rest, &[]));
            self.end_line(line, &mut results);
            // `after` starts with the newline, which is not part of any frame.
            rest = after.get(1..).unwrap_or(&[]);
        }
        self.extend_line(rest, &mut results);
        results
    }

    /// Adds bytes of a line that hasn't ended.
    fn extend_line(&mut self, bytes: &[u8], results: &mut Vec<Result<Vec<u8>, ProtocolError>>) {
        if self.skipping || bytes.is_empty() {
            return;
        }
        if self.buffer.len().saturating_add(bytes.len()) > MAX_FRAME_LEN {
            self.start_skipping(results);
        } else {
            self.buffer.extend_from_slice(bytes);
        }
    }

    /// Ends the line in progress, whose last bytes are `tail`.
    fn end_line(&mut self, tail: &[u8], results: &mut Vec<Result<Vec<u8>, ProtocolError>>) {
        if self.skipping {
            // Already reported when it passed the limit.
            self.skipping = false;
            return;
        }
        if self.buffer.len().saturating_add(tail.len()) > MAX_FRAME_LEN {
            results.push(Err(frame_too_large()));
            self.buffer = Vec::new();
            return;
        }
        let frame = if self.buffer.is_empty() {
            tail.to_vec()
        } else {
            let mut frame = std::mem::take(&mut self.buffer);
            frame.extend_from_slice(tail);
            frame
        };
        if !frame.is_empty() {
            results.push(Ok(frame));
        }
    }

    /// Reports that the line in progress passed the limit and drops it, along
    /// with the memory it held.
    fn start_skipping(&mut self, results: &mut Vec<Result<Vec<u8>, ProtocolError>>) {
        results.push(Err(frame_too_large()));
        self.buffer = Vec::new();
        self.skipping = true;
    }
}

#[cfg(test)]
mod tests;
