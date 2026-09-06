// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The HTTP response against arbitrary bytes, in arbitrary pieces: no
//! input and no split may panic, every byte the decoder hands back came
//! out of the bytes it was given, and every call either takes input or
//! says it needs more.
//!
//! The split is the point of this target. A parser of a line-oriented
//! format has one bug that a whole-message test cannot find: the state it
//! keeps between the piece that ended mid-line and the piece that finishes
//! it. So the first byte of the input names the size of the pieces the
//! rest is fed in, and the mutator gets to choose where every boundary
//! falls.
//!
//! Two doors are driven. The decoder is the front one, from the status
//! line through the field list to whichever of the four framings the head
//! decided on. `finish` is the other: it is what tells a body that ends
//! at a close from one that was cut off, and it is reached with the
//! decoder in whatever state the input left it in.

use net_http::request::Method;
use net_http::response::{Decoder, Event};

/// How many fields the decoder holds.
const HEADERS: usize = 16;

/// How much room the head is given.
const HEAD: usize = 2048;

/// The largest input this target takes.
const MAX_INPUT: usize = 4096;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let Some((first, rest)) = bytes.split_first() else {
        return;
    };
    if rest.len() > MAX_INPUT {
        return;
    }
    // The low six bits of the first byte name the pieces, so that a
    // mutator can put a boundary anywhere without changing the message.
    let piece = usize::from(first & 0x3F).max(1);
    decode(rest, piece, Method::Get);
    decode(rest, piece, Method::Head);
});

/// Feeds `bytes` in pieces of `piece` and checks what comes back.
fn decode(bytes: &[u8], piece: usize, method: Method) {
    let mut buffer = [0u8; HEAD];
    let mut decoder = Decoder::<HEADERS>::new(&mut buffer, method);
    let mut body = 0usize;
    let mut at = 0usize;
    let mut turns = 0usize;
    'outer: while at < bytes.len() {
        let end = at.saturating_add(piece).min(bytes.len());
        let Some(mut chunk) = bytes.get(at..end) else {
            break;
        };
        while !chunk.is_empty() {
            let Ok((consumed, event)) = decoder.feed(chunk) else {
                return;
            };
            assert!(
                consumed <= chunk.len(),
                "the decoder took more than it was given"
            );
            if let Event::Body(part) = event {
                assert!(
                    part.len() <= chunk.len(),
                    "a body piece longer than the input it came from"
                );
                body = body.saturating_add(part.len());
            }
            turns = turns.saturating_add(1);
            assert!(turns <= 200_000, "the decoder went round without end");
            if consumed == 0 {
                // Nothing was taken, so nothing in this piece will be:
                // either more input is needed or the message is done.
                assert!(
                    matches!(event, Event::NeedMore | Event::Done),
                    "the decoder took nothing and had something to say"
                );
                if matches!(event, Event::Done) {
                    break 'outer;
                }
                break;
            }
            let Some(left) = chunk.get(consumed..) else {
                break;
            };
            chunk = left;
        }
        at = end;
    }
    assert!(
        body <= bytes.len(),
        "more body than there were bytes to make it of"
    );
    // Whatever state the input left it in, the close either completes the
    // message or says it was cut off, and never panics.
    let closed = decoder.finish();
    if closed.is_ok() {
        assert!(decoder.is_done(), "a message that closed and is not done");
    }
    // A decoder that is done says so to every further call.
    if decoder.is_done() {
        assert_eq!(decoder.feed(b"anything"), Ok((0, Event::Done)));
    }
}
