// SPDX-License-Identifier: MIT OR Apache-2.0

//! The text a `RandomText` session asks the user to type to stop it.
//!
//! It is friction, not a lock (`docs/architecture.md`, "Data model"), but
//! it only adds friction if it can't be guessed or read: so it comes from
//! the operating system's cryptographically secure source, is compared in
//! constant time, and is never logged, stored or formatted with `{:?}`.

use std::fmt;

/// The characters a text is made of: lowercase ASCII letters and digits,
/// without `l`, `o`, `0` and `1`, which are easy to mistake for each other.
/// Exactly 32, so that 5 random bits pick one with no bias.
const ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// The bits of a random byte that pick a character of [`ALPHABET`].
const ALPHABET_MASK: u8 = 0b1_1111;

/// A text of random characters from [`ALPHABET`].
///
/// Its `Debug` output hides it, and dropping it overwrites it, as far as
/// the compiler lets safe code do that.
pub(super) struct StopText(Vec<u8>);

impl StopText {
    /// A new text of `len` random characters.
    ///
    /// # Errors
    ///
    /// The operating system's error if its random source fails.
    pub(super) fn generate(len: u32) -> Result<Self, getrandom::Error> {
        let len = usize::try_from(len).unwrap_or(0);
        let mut text = vec![0_u8; len];
        getrandom::fill(&mut text)?;
        for byte in &mut text {
            *byte = ALPHABET
                .get(usize::from(*byte & ALPHABET_MASK))
                .copied()
                .unwrap_or(b'a');
        }
        Ok(Self(text))
    }

    /// The text, for the engine's unit tests: no message carries it yet.
    #[cfg(test)]
    pub(super) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("the alphabet is ASCII")
    }

    /// Whether `typed` is exactly this text.
    ///
    /// The time it takes doesn't depend on which characters match: every
    /// byte is compared when the lengths are equal. Only the length can
    /// show, and it is no secret, as the session's lock holds it.
    pub(super) fn matches(&self, typed: &str) -> bool {
        let typed = typed.as_bytes();
        if typed.len() != self.0.len() {
            return false;
        }
        let mut difference = 0_u8;
        for (expected, given) in self.0.iter().zip(typed) {
            difference |= expected ^ given;
        }
        std::hint::black_box(difference) == 0
    }
}

impl fmt::Debug for StopText {
    /// Never shows the text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StopText(..)")
    }
}

impl Drop for StopText {
    fn drop(&mut self) {
        self.0.fill(0);
        // Keeps the compiler from dropping the writes as dead stores.
        std::hint::black_box(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(len: u32) -> String {
        let generated = StopText::generate(len).expect("the random source works");
        String::from_utf8(generated.0.clone()).expect("ASCII")
    }

    #[test]
    fn a_text_has_the_asked_length_and_only_alphabet_characters() {
        for len in [1, 8, 256] {
            let text = text(len);
            assert_eq!(text.len(), usize::try_from(len).expect("small"));
            assert!(text.bytes().all(|byte| ALPHABET.contains(&byte)), "{text}");
        }
    }

    #[test]
    fn two_texts_differ() {
        // 64 characters of 5 bits each: equal by chance with probability
        // 2^-320.
        assert_ne!(text(64), text(64));
    }

    #[test]
    fn only_the_exact_text_matches() {
        let stop = StopText::generate(12).expect("the random source works");
        let exact = String::from_utf8(stop.0.clone()).expect("ASCII");
        assert!(stop.matches(&exact));
        assert!(!stop.matches(&exact.to_uppercase()));
        assert!(!stop.matches(""));
        assert!(!stop.matches(exact.get(..11).expect("in range")));
        assert!(!stop.matches(&format!("{exact}a")));
        let mut changed = exact.into_bytes();
        changed[5] = if changed[5] == b'a' { b'b' } else { b'a' };
        assert!(!stop.matches(&String::from_utf8(changed).expect("ASCII")));
    }

    #[test]
    fn debug_never_shows_the_text() {
        let stop = StopText::generate(32).expect("the random source works");
        let shown = format!("{stop:?}");
        let text = String::from_utf8(stop.0.clone()).expect("ASCII");
        assert!(!shown.contains(&text), "{shown}");
        assert_eq!(shown, "StopText(..)");
    }

    #[test]
    fn the_alphabet_has_no_lookalikes_and_no_repeats() {
        for lookalike in *b"lo01" {
            assert!(!ALPHABET.contains(&lookalike));
        }
        let mut sorted = ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 32);
    }
}
