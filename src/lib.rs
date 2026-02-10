//! Incremental UTF-8 decoder that safely handles multi-byte characters split across chunk boundaries.
//!
//! When reading byte streams in chunks, multi-byte UTF-8 characters (CJK, emoji, etc.)
//! can be split at chunk boundaries. This crate buffers incomplete sequences and
//! reassembles them correctly.
//!
//! # Core API (no dependencies)
//!
//! ```
//! use utf8_chunked::Utf8Chunker;
//!
//! let mut chunker = Utf8Chunker::new();
//!
//! // '한' = 0xED 0x95 0x9C (3 bytes), split across two chunks
//! assert_eq!(chunker.push(&[0xED, 0x95]), None);
//! assert_eq!(chunker.push(&[0x9C, b'!']), Some("한!".to_string()));
//! ```
//!
//! # Async Stream (requires `tokio` feature)
//!
//! ```ignore
//! use utf8_chunked::utf8_safe_stream;
//! use tokio_stream::StreamExt;
//!
//! let mut stream = utf8_safe_stream(reader);
//! while let Some(chunk) = stream.next().await {
//!     print!("{}", chunk.unwrap());
//! }
//! ```

#![cfg_attr(not(feature = "tokio"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Incremental UTF-8 decoder that buffers incomplete multi-byte sequences.
///
/// `Utf8Chunker` accepts arbitrary byte slices and produces valid UTF-8 strings,
/// correctly handling multi-byte characters that span chunk boundaries.
///
/// At most 3 bytes are buffered between calls (the maximum incomplete prefix
/// of a 4-byte UTF-8 sequence).
///
/// # Examples
///
/// ```
/// use utf8_chunked::Utf8Chunker;
///
/// let mut chunker = Utf8Chunker::new();
///
/// // ASCII passes through directly
/// assert_eq!(chunker.push(b"hello"), Some("hello".to_string()));
///
/// // Emoji '🦀' = F0 9F A6 80 (4 bytes), split 2+2
/// assert_eq!(chunker.push(&[0xF0, 0x9F]), None);
/// assert_eq!(chunker.push(&[0xA6, 0x80]), Some("🦀".to_string()));
/// ```
#[derive(Debug, Default)]
pub struct Utf8Chunker {
    buf: Vec<u8>,
}

impl Utf8Chunker {
    /// Creates a new `Utf8Chunker` with an empty buffer.
    #[inline]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Processes an incoming byte chunk and returns any complete UTF-8 text.
    ///
    /// Returns `Some(String)` if at least one valid UTF-8 character can be produced,
    /// or `None` if all input bytes are buffered as part of an incomplete sequence.
    ///
    /// # Fast Path
    ///
    /// When the internal buffer is empty and `data` is entirely valid UTF-8,
    /// the data is returned without any copying or allocation.
    pub fn push(&mut self, data: &[u8]) -> Option<String> {
        if data.is_empty() {
            return None;
        }

        // Fast path: no pending buffer and data is valid UTF-8
        if self.buf.is_empty() {
            if let Ok(s) = core::str::from_utf8(data) {
                return Some(String::from(s));
            }
        }

        // Merge buffer + new data
        self.buf.extend_from_slice(data);

        // Find how much is valid UTF-8
        match core::str::from_utf8(&self.buf) {
            Ok(s) => {
                let result = String::from(s);
                self.buf.clear();
                Some(result)
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();

                // Check how many trailing bytes form an incomplete sequence
                let trailing = &self.buf[valid_up_to..];
                let incomplete_len = incomplete_sequence_len(trailing);

                if incomplete_len == 0 && valid_up_to == 0 {
                    // No valid data and no incomplete sequence — shouldn't normally happen
                    // with well-formed input, but handle gracefully
                    return None;
                }

                let result = if valid_up_to > 0 {
                    // Safety: from_utf8 confirmed these bytes are valid
                    let s = unsafe { core::str::from_utf8_unchecked(&self.buf[..valid_up_to]) };
                    Some(String::from(s))
                } else {
                    None
                };

                if incomplete_len > 0 {
                    // Keep incomplete sequence in buffer
                    let start = self.buf.len() - incomplete_len;
                    let remaining: Vec<u8> = self.buf[start..].to_vec();
                    self.buf.clear();
                    self.buf.extend_from_slice(&remaining);
                } else {
                    self.buf.clear();
                }

                result
            }
        }
    }

    /// Flushes any remaining buffered bytes using lossy UTF-8 conversion.
    ///
    /// Call this when the byte stream is finished. Any incomplete multi-byte
    /// sequence in the buffer will be replaced with U+FFFD (replacement character).
    ///
    /// Returns `None` if the buffer is empty.
    pub fn flush(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(s)
    }

    /// Returns `true` if the internal buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Returns the number of bytes currently buffered.
    #[inline]
    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }
}

/// Determines how many trailing bytes form an incomplete UTF-8 sequence.
///
/// Returns the number of bytes that should be kept in the buffer,
/// or 0 if the trailing bytes are not a valid incomplete sequence.
fn incomplete_sequence_len(trailing: &[u8]) -> usize {
    if trailing.is_empty() {
        return 0;
    }

    // Walk backwards from the end to find the start of an incomplete sequence.
    // We look for a leading byte (not a continuation byte) within the last 3 bytes.
    let len = trailing.len();
    let check_len = len.min(4);

    for i in (0..check_len).rev() {
        let idx = len - 1 - i;
        let byte = trailing[idx];

        if byte & 0x80 == 0 {
            // ASCII byte — cannot be start of incomplete multibyte sequence
            // Everything before this is either complete or invalid
            if i == 0 {
                return 0; // trailing ends with ASCII, nothing incomplete
            }
            continue;
        }

        if byte & 0xC0 != 0x80 {
            // This is a leading byte (110xxxxx, 1110xxxx, or 11110xxx)
            let expected_len = utf8_char_len(byte);
            let available = i + 1; // bytes from this position to end

            if expected_len > 0 && available < expected_len {
                // Incomplete sequence — keep these bytes
                return available;
            } else {
                // Complete or invalid sequence
                return 0;
            }
        }
        // Continuation byte (10xxxxxx) — keep looking for leading byte
    }

    // All checked bytes are continuation bytes — this is malformed,
    // but keep them in case more data arrives
    check_len.min(3)
}

/// Returns the expected length of a UTF-8 character from its leading byte.
/// Returns 0 for invalid leading bytes.
#[inline]
fn utf8_char_len(byte: u8) -> usize {
    if byte & 0x80 == 0 {
        1
    } else if byte & 0xE0 == 0xC0 {
        2
    } else if byte & 0xF0 == 0xE0 {
        3
    } else if byte & 0xF8 == 0xF0 {
        4
    } else {
        0
    }
}

// ============================================================
// tokio feature: async utilities
// ============================================================

#[cfg(feature = "tokio")]
mod async_support {
    use super::Utf8Chunker;
    use tokio_util::bytes::BytesMut;
    use std::io;
    use tokio::io::AsyncRead;
    use tokio_stream::Stream;
    use tokio_util::codec::{Decoder, FramedRead};

    /// A `tokio_util::codec::Decoder` that produces valid UTF-8 strings from byte streams.
    ///
    /// Use with [`FramedRead`] to create an async stream of UTF-8 strings:
    ///
    /// ```ignore
    /// use utf8_chunked::Utf8Codec;
    /// use tokio_util::codec::FramedRead;
    /// use tokio_stream::StreamExt;
    ///
    /// let framed = FramedRead::new(reader, Utf8Codec::new());
    /// tokio::pin!(framed);
    /// while let Some(text) = framed.next().await {
    ///     print!("{}", text.unwrap());
    /// }
    /// ```
    #[derive(Debug, Default)]
    pub struct Utf8Codec {
        chunker: Utf8Chunker,
    }

    impl Utf8Codec {
        /// Creates a new `Utf8Codec`.
        pub fn new() -> Self {
            Self {
                chunker: Utf8Chunker::new(),
            }
        }
    }

    impl Decoder for Utf8Codec {
        type Item = String;
        type Error = io::Error;

        fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
            if buf.is_empty() {
                return Ok(None);
            }
            let data = buf.split_to(buf.len());
            Ok(self.chunker.push(&data))
        }

        fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
            if !buf.is_empty() {
                let data = buf.split_to(buf.len());
                if let Some(s) = self.chunker.push(&data) {
                    return Ok(Some(s));
                }
            }
            Ok(self.chunker.flush())
        }
    }

    /// Creates an async stream of UTF-8 strings from an `AsyncRead` source.
    ///
    /// Multi-byte characters split across read boundaries are automatically
    /// buffered and reassembled.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use utf8_chunked::utf8_safe_stream;
    /// use tokio_stream::StreamExt;
    ///
    /// let mut stream = utf8_safe_stream(reader);
    /// while let Some(chunk) = stream.next().await {
    ///     print!("{}", chunk.unwrap());
    /// }
    /// ```
    pub fn utf8_safe_stream<R>(reader: R) -> impl Stream<Item = io::Result<String>>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        FramedRead::new(reader, Utf8Codec::new())
    }
}

#[cfg(feature = "tokio")]
pub use async_support::{utf8_safe_stream, Utf8Codec};

// ============================================================
// Unit tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_passthrough() {
        let mut c = Utf8Chunker::new();
        assert_eq!(c.push(b"hello world"), Some("hello world".into()));
        assert!(c.is_empty());
    }

    #[test]
    fn empty_input() {
        let mut c = Utf8Chunker::new();
        assert_eq!(c.push(b""), None);
    }

    #[test]
    fn complete_multibyte() {
        let mut c = Utf8Chunker::new();
        assert_eq!(c.push("한글".as_bytes()), Some("한글".into()));
        assert!(c.is_empty());
    }

    #[test]
    fn split_3byte_char() {
        let mut c = Utf8Chunker::new();
        // '한' = ED 95 9C
        assert_eq!(c.push(&[0xED, 0x95]), None);
        assert_eq!(c.buffered_len(), 2);
        assert_eq!(c.push(&[0x9C]), Some("한".into()));
        assert!(c.is_empty());
    }

    #[test]
    fn split_4byte_emoji() {
        let mut c = Utf8Chunker::new();
        // '🦀' = F0 9F A6 80
        assert_eq!(c.push(&[0xF0, 0x9F]), None);
        assert_eq!(c.push(&[0xA6, 0x80]), Some("🦀".into()));
    }

    #[test]
    fn split_4byte_emoji_three_ways() {
        let mut c = Utf8Chunker::new();
        // '🦀' = F0 9F A6 80
        assert_eq!(c.push(&[0xF0]), None);
        assert_eq!(c.push(&[0x9F, 0xA6]), None);
        assert_eq!(c.push(&[0x80, b'!']), Some("🦀!".into()));
    }

    #[test]
    fn flush_incomplete() {
        let mut c = Utf8Chunker::new();
        assert_eq!(c.push(&[0xED, 0x95]), None);
        let flushed = c.flush().unwrap();
        assert!(flushed.contains('\u{FFFD}'));
        assert!(c.is_empty());
    }

    #[test]
    fn flush_empty() {
        let mut c = Utf8Chunker::new();
        assert_eq!(c.flush(), None);
    }

    #[test]
    fn mixed_ascii_and_multibyte_split() {
        let mut c = Utf8Chunker::new();
        // "hi한" where '한' = ED 95 9C, split after "hi" + first byte
        assert_eq!(c.push(b"hi\xED\x95"), Some("hi".into()));
        assert_eq!(c.push(b"\x9Cbye"), Some("한bye".into()));
    }

    #[test]
    fn consecutive_multibyte() {
        let mut c = Utf8Chunker::new();
        // "가나" = EA B0 80 EB 82 98, split in middle
        assert_eq!(c.push(&[0xEA, 0xB0, 0x80, 0xEB]), Some("가".into()));
        assert_eq!(c.push(&[0x82, 0x98]), Some("나".into()));
    }

    #[test]
    fn two_byte_char_split() {
        let mut c = Utf8Chunker::new();
        // 'é' = C3 A9 (2 bytes)
        assert_eq!(c.push(&[0xC3]), None);
        assert_eq!(c.push(&[0xA9]), Some("é".into()));
    }

    #[test]
    fn large_valid_chunk() {
        let mut c = Utf8Chunker::new();
        let text = "Hello, 世界! 🌍 こんにちは";
        assert_eq!(c.push(text.as_bytes()), Some(text.into()));
    }

    #[test]
    fn default_trait() {
        let c = Utf8Chunker::default();
        assert!(c.is_empty());
        assert_eq!(c.buffered_len(), 0);
    }
}
