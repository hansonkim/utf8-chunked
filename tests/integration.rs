use utf8_chunked::Utf8Chunker;

// ============================================================
// Boundary split scenarios
// ============================================================

#[test]
fn korean_3byte_split_at_every_position() {
    // '한' = ED 95 9C
    let bytes = [0xED, 0x95, 0x9C];

    // Split after 1 byte
    let mut c = Utf8Chunker::new();
    assert_eq!(c.push(&bytes[..1]), None);
    assert_eq!(c.push(&bytes[1..]), Some("한".into()));

    // Split after 2 bytes
    let mut c = Utf8Chunker::new();
    assert_eq!(c.push(&bytes[..2]), None);
    assert_eq!(c.push(&bytes[2..]), Some("한".into()));
}

#[test]
fn chinese_3byte_split() {
    // '世' = E4 B8 96
    let bytes = [0xE4, 0xB8, 0x96];

    let mut c = Utf8Chunker::new();
    assert_eq!(c.push(&bytes[..1]), None);
    assert_eq!(c.push(&bytes[1..]), Some("世".into()));
}

#[test]
fn emoji_4byte_split_at_every_position() {
    // '🦀' = F0 9F A6 80
    let bytes = [0xF0, 0x9F, 0xA6, 0x80];

    for split_at in 1..4 {
        let mut c = Utf8Chunker::new();
        assert_eq!(c.push(&bytes[..split_at]), None, "split_at={split_at}");
        assert_eq!(
            c.push(&bytes[split_at..]),
            Some("🦀".into()),
            "split_at={split_at}"
        );
    }
}

#[test]
fn emoji_4byte_split_into_individual_bytes() {
    // '🦀' = F0 9F A6 80
    let mut c = Utf8Chunker::new();
    assert_eq!(c.push(&[0xF0]), None);
    assert_eq!(c.push(&[0x9F]), None);
    assert_eq!(c.push(&[0xA6]), None);
    assert_eq!(c.push(&[0x80]), Some("🦀".into()));
}

#[test]
fn compound_emoji_split() {
    // '👨‍👩‍👧' is a family emoji composed of multiple code points with ZWJ
    let text = "👨\u{200D}👩\u{200D}👧";
    let bytes = text.as_bytes();

    // Split in the middle
    let mid = bytes.len() / 2;
    let mut c = Utf8Chunker::new();
    let part1 = c.push(&bytes[..mid]);
    let part2 = c.push(&bytes[mid..]);
    let combined = format!(
        "{}{}",
        part1.unwrap_or_default(),
        part2.unwrap_or_default()
    );
    assert_eq!(combined, text);
}

// ============================================================
// ASCII fast path
// ============================================================

#[test]
fn pure_ascii_fast_path() {
    let mut c = Utf8Chunker::new();
    let result = c.push(b"Hello, World! 12345");
    assert_eq!(result, Some("Hello, World! 12345".into()));
    assert!(c.is_empty());
}

#[test]
fn large_ascii_chunk() {
    let mut c = Utf8Chunker::new();
    let data = "A".repeat(65536);
    assert_eq!(c.push(data.as_bytes()), Some(data));
    assert!(c.is_empty());
}

// ============================================================
// Mixed content
// ============================================================

#[test]
fn mixed_ascii_cjk_emoji_stream() {
    let text = "Hello, 세계! 🌍 The world is beautiful. 日本語テスト";
    let bytes = text.as_bytes();

    // Simulate chunked reading with various sizes
    let chunk_sizes = [3, 7, 5, 11, 2, 4, 1, 8, 100];
    let mut c = Utf8Chunker::new();
    let mut result = String::new();
    let mut offset = 0;

    for &size in &chunk_sizes {
        if offset >= bytes.len() {
            break;
        }
        let end = (offset + size).min(bytes.len());
        if let Some(s) = c.push(&bytes[offset..end]) {
            result.push_str(&s);
        }
        offset = end;
    }
    if let Some(s) = c.flush() {
        result.push_str(&s);
    }

    assert_eq!(result, text);
}

#[test]
fn byte_at_a_time() {
    let text = "한글 🦀 test";
    let bytes = text.as_bytes();
    let mut c = Utf8Chunker::new();
    let mut result = String::new();

    for &b in bytes {
        if let Some(s) = c.push(&[b]) {
            result.push_str(&s);
        }
    }
    if let Some(s) = c.flush() {
        result.push_str(&s);
    }

    assert_eq!(result, text);
}

// ============================================================
// Flush / lossy behavior
// ============================================================

#[test]
fn flush_produces_replacement_for_incomplete() {
    let mut c = Utf8Chunker::new();
    // Push only the first 2 bytes of a 3-byte char
    c.push(&[0xE4, 0xB8]);
    let flushed = c.flush().unwrap();
    assert!(flushed.contains('\u{FFFD}'));
}

#[test]
fn flush_after_complete_data_returns_none() {
    let mut c = Utf8Chunker::new();
    c.push(b"complete");
    assert_eq!(c.flush(), None);
}

// ============================================================
// Edge cases
// ============================================================

#[test]
fn empty_push_does_not_affect_buffer() {
    let mut c = Utf8Chunker::new();
    assert_eq!(c.push(&[0xF0, 0x9F]), None);
    assert_eq!(c.buffered_len(), 2);
    assert_eq!(c.push(b""), None);
    assert_eq!(c.buffered_len(), 2); // unchanged
    assert_eq!(c.push(&[0xA6, 0x80]), Some("🦀".into()));
}

#[test]
fn multiple_multibyte_chars_in_sequence() {
    let text = "가나다라마바사아자차카타파하";
    let bytes = text.as_bytes();
    let mut c = Utf8Chunker::new();
    let mut result = String::new();

    // Process 4 bytes at a time (will split 3-byte chars)
    for chunk in bytes.chunks(4) {
        if let Some(s) = c.push(chunk) {
            result.push_str(&s);
        }
    }
    if let Some(s) = c.flush() {
        result.push_str(&s);
    }

    assert_eq!(result, text);
}

#[test]
fn two_byte_latin_extended() {
    // Test various 2-byte UTF-8 characters: é ñ ü ß
    let text = "café résumé naïve über straße";
    let bytes = text.as_bytes();
    let mut c = Utf8Chunker::new();
    let mut result = String::new();

    for chunk in bytes.chunks(3) {
        if let Some(s) = c.push(chunk) {
            result.push_str(&s);
        }
    }
    if let Some(s) = c.flush() {
        result.push_str(&s);
    }

    assert_eq!(result, text);
}

// ============================================================
// tokio feature tests
// ============================================================

#[cfg(feature = "tokio")]
mod tokio_tests {
    use tokio_stream::StreamExt;
    use utf8_chunked::{utf8_safe_stream, Utf8Codec};

    #[tokio::test]
    async fn utf8_safe_stream_basic() {
        let reader = tokio_util::io::StreamReader::new(
            tokio_stream::once(Ok::<_, std::io::Error>(
                tokio_util::bytes::Bytes::from("Hello, 세계! 🦀".as_bytes().to_vec()),
            )),
        );

        let mut stream = utf8_safe_stream(reader);
        let mut result = String::new();
        while let Some(chunk) = stream.next().await {
            result.push_str(&chunk.unwrap());
        }
        assert_eq!(result, "Hello, 세계! 🦀");
    }

    #[tokio::test]
    async fn utf8_safe_stream_split_chunks() {
        // '한' = ED 95 9C, send in two chunks
        let chunks: Vec<Result<tokio_util::bytes::Bytes, std::io::Error>> = vec![
            Ok(tokio_util::bytes::Bytes::from_static(&[
                b'H', b'i', 0xED, 0x95,
            ])),
            Ok(tokio_util::bytes::Bytes::from_static(&[0x9C, b'!'])),
        ];

        let stream = tokio_stream::iter(chunks);
        let reader = tokio_util::io::StreamReader::new(stream);

        let mut utf8_stream = utf8_safe_stream(reader);
        let mut result = String::new();
        while let Some(chunk) = utf8_stream.next().await {
            result.push_str(&chunk.unwrap());
        }
        assert_eq!(result, "Hi한!");
    }

    #[tokio::test]
    async fn codec_with_framed_read() {
        use tokio_util::codec::FramedRead;

        let chunks: Vec<Result<tokio_util::bytes::Bytes, std::io::Error>> = vec![
            Ok(tokio_util::bytes::Bytes::from_static(&[0xF0, 0x9F])),
            Ok(tokio_util::bytes::Bytes::from_static(&[
                0xA6, 0x80, b' ', b'R', b'u', b's', b't',
            ])),
        ];

        let stream = tokio_stream::iter(chunks);
        let reader = tokio_util::io::StreamReader::new(stream);
        let mut framed = FramedRead::new(reader, Utf8Codec::new());

        let mut result = String::new();
        while let Some(text) = framed.next().await {
            result.push_str(&text.unwrap());
        }
        assert_eq!(result, "🦀 Rust");
    }
}
