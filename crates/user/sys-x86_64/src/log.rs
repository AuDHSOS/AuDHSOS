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
//! Invariant: a line that cannot go out is dropped and nothing is reported
//! about the dropping. A diagnostic that fails is not worth a second
//! diagnostic, and there is nowhere to send it.

use audhsos_abi::Error;
use user_rt::EndpointHandle;
use user_rt::message::Writer;

use crate::gate::Gate;

/// The label a log line is sent under.
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
    let mut writer = Writer::new();
    {
        let mut buffer = gate.writer();
        writer.bytes(&mut buffer, line)?;
        writer.finish(&mut buffer, LOG_LABEL)?;
    }
    match endpoint {
        Some(endpoint) => gate.ipc_send(endpoint),
        None => gate.debug_log().map(|_written| ()),
    }
}
