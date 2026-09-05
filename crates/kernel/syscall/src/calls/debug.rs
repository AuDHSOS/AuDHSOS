// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `debug_log`.
//!
//! Invariant: the call reads the message area and never more of it than
//! the word count says, so a caller cannot make the kernel read past what
//! it filled in.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::{Buffer, SIZE};

use crate::dispatch::{Machine, Reply};
use crate::environment::Environment;

/// The bytes of one payload word, as they reach the console.
const WORD: usize = 8;

/// `debug_log`: writes the message area to the debug console. The label
/// carries how many bytes of the last word belong to the message, so a
/// line that is not a multiple of eight bytes long arrives as it was
/// written. A build without the console drops the bytes.
///
/// # Errors
///
/// [`Error::InvalidArgument`] when the message header names more words or
/// handles than the area holds; nothing is written then.
pub fn log<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    buffer: &[u8; SIZE],
) -> Result<Reply, Error> {
    let view = Buffer::new(buffer);
    let message = view.message()?;
    let mut written = 0_u64;
    for index in 0..message.word_count {
        let Some(word) = view.word(index) else {
            break;
        };
        let bytes = word.to_le_bytes();
        let last = index.saturating_add(1) == message.word_count;
        let take = if last {
            trailing_bytes(message.label)
        } else {
            WORD
        };
        let Some(slice) = bytes.get(..take) else {
            return Err(Error::InvalidArgument);
        };
        machine.environment.log(slice);
        written = written.saturating_add(u64::try_from(take).unwrap_or(0));
    }
    Ok(Reply::value(written))
}

/// How many bytes of the last word the message uses: the label says it,
/// and anything but one to eight means the whole word.
fn trailing_bytes(label: u64) -> usize {
    match label {
        1..=7 => usize::try_from(label).unwrap_or(WORD),
        _ => WORD,
    }
}
