# utf8-chunked

Incremental UTF-8 decoder that safely handles multi-byte characters split across chunk boundaries.

## The Problem

When reading byte streams in chunks (from network sockets, file I/O, subprocess pipes, etc.), multi-byte UTF-8 characters like CJK characters (한글, 漢字) and emoji (🦀) can be split across chunk boundaries.

For example, the Korean character '한' is encoded as 3 bytes (`0xED 0x95 0x9C`). If a 4096-byte chunk boundary falls between the second and third byte, `String::from_utf8()` will fail, and `String::from_utf8_lossy()` will permanently corrupt the character with replacement characters (U+FFFD).

## Solution

`utf8-chunked` buffers incomplete multi-byte sequences at chunk boundaries and prepends them to the next chunk, producing correct UTF-8 strings every time.

### Features

- **Zero-copy fast path**: Pure ASCII or complete UTF-8 chunks are returned without copying
- **`no_std` compatible core**: `Utf8Chunker` works without any dependencies
- **Optional `tokio` integration**: Stream adapter and codec for async byte streams
- **Minimal buffering**: At most 3 bytes buffered between chunks

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
utf8-chunked = "0.1"
```

With `tokio` async support:

```toml
[dependencies]
utf8-chunked = { version = "0.1", features = ["tokio"] }
```

## Usage

### Core API (sync, no dependencies)

```rust
use utf8_chunked::Utf8Chunker;

let mut chunker = Utf8Chunker::new();

// Simulate '한' (0xED 0x95 0x9C) split across two chunks
let chunk1 = &[0xED, 0x95]; // incomplete
let chunk2 = &[0x9C, b'!'];  // completes the character

assert_eq!(chunker.push(chunk1), None); // buffered
assert_eq!(chunker.push(chunk2), Some("한!".to_string()));
```

### Async Stream (with `tokio` feature)

```rust
use utf8_chunked::utf8_safe_stream;
use tokio::process::Command;
use tokio_stream::StreamExt;

let child = Command::new("some-program")
    .stdout(std::process::Stdio::piped())
    .spawn()?;

let stdout = child.stdout.unwrap();
let mut stream = utf8_safe_stream(stdout);

while let Some(chunk) = stream.next().await {
    print!("{}", chunk?); // always valid UTF-8
}
```

### Codec (with `tokio` feature)

```rust
use utf8_chunked::Utf8Codec;
use tokio_util::codec::FramedRead;
use tokio_stream::StreamExt;

let framed = FramedRead::new(reader, Utf8Codec::new());
tokio::pin!(framed);

while let Some(text) = framed.next().await {
    print!("{}", text?);
}
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `tokio` | No | Enables `utf8_safe_stream()` and `Utf8Codec` for async usage |

## Why not just use `from_utf8_lossy`?

`String::from_utf8_lossy` replaces incomplete sequences with `U+FFFD` (REPLACEMENT CHARACTER). This is **irreversible** -- once replaced, the original bytes are lost. If the missing bytes arrive in the next chunk, you get corrupted output instead of the correct character.

`utf8-chunked` buffers the incomplete bytes and waits for the rest, producing correct output.

## Comparison with Existing Crates

| Crate | Async | Buffering | no_std | Notes |
|-------|-------|-----------|--------|-------|
| `utf8-chunked` | Yes (optional) | Yes | Yes (core) | Codec + stream + sync API |
| `utf-8` | No | Yes | Yes | Sync only, no async integration |
| `utf8-tokio` | Yes | Yes | No | Unmaintained, limited API |
| `async-utf8-decoder` | Yes | Yes | No | Wraps reader, no codec support |

## Performance

The fast path (buffer empty + valid UTF-8 input) performs a single `std::str::from_utf8()` check with zero allocation. Only chunks containing split multi-byte sequences require buffering and copying.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
