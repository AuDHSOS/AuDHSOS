// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Saying something, through whichever channel the program has.
//!
//! Before the console driver runs there is no log endpoint, and the only
//! way out is `debug_log`, which writes through the kernel's own console —
//! the one it gives up the moment somebody takes the serial port over. Once
//! the driver is up the program has an endpoint and uses it. The two are
//! one function here, because the callers are the same callers before and
//! after and the difference is a handle they either hold or do not.
//!
//! The two channels carry the bytes differently, and they have to. A
//! message to a server is a field of a protocol: a word saying how many
//! bytes there are, then the bytes. `debug_log` has no protocol above it —
//! the kernel writes what the message area holds — so the bytes go in
//! plain, and the label says how many of the last word belong to the line.
//!
//! Invariant: a line that cannot go out is dropped and nothing is reported
//! about the dropping. A diagnostic that fails is not worth a second
//! diagnostic, and there is nowhere to send it.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::WORD;
use audhsos_abi::layout::MAX_MESSAGE_WORDS;
use user_rt::EndpointHandle;
use user_rt::message::Writer;

use crate::gate::Gate;

/// The label a log line is sent under when it goes to an endpoint.
pub const LOG_LABEL: u64 = u64::from_be_bytes(*b"LOGLINE\0");

/// Writes `line` to `endpoint`, or to the kernel console when there is no
/// endpoint.
///
/// # Errors
///
/// Whatever the send or the call answered, and the codec error of a line
/// that does not fit the message area.
pub fn write_line(
    gate: &mut Gate,
    endpoint: Option<EndpointHandle>,
    line: &[u8],
) -> Result<(), Error> {
    match endpoint {
        Some(endpoint) => {
            let mut writer = Writer::new();
            {
                let mut buffer = gate.writer();
                writer.bytes(&mut buffer, line)?;
                writer.finish(&mut buffer, LOG_LABEL)?;
            }
            gate.ipc_send(endpoint)
        }
        None => log_to_kernel(gate, line),
    }
}

/// Writes `line` through `debug_log`, which reads the message area as
/// bytes and takes the count of the last word out of the label.
fn log_to_kernel(gate: &mut Gate, line: &[u8]) -> Result<(), Error> {
    let words = line.len().div_ceil(WORD).min(MAX_MESSAGE_WORDS);
    let trailing = match line.len().wrapping_rem(WORD) {
        0 => WORD,
        rest => rest,
    };
    {
        let mut buffer = gate.writer();
        for (index, chunk) in line.chunks(WORD).take(words).enumerate() {
            let mut word = [0u8; WORD];
            if let Some(slot) = word.get_mut(..chunk.len()) {
                slot.copy_from_slice(chunk);
            }
            buffer.set_word(index, u64::from_le_bytes(word));
        }
        buffer.set_label(u64::try_from(trailing).unwrap_or(0));
        buffer.set_counts(words, 0)?;
    }
    gate.debug_log().map(|_written| ())
}
