// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Asking a server something: the round trip, once per request.
//!
//! Each of these is the same three steps — encode into the buffer, make the
//! call, decode the answer — and they are here rather than in `user-proto`
//! because the middle step needs the gate and `user-proto` is a crate that
//! makes no system call.
//!
//! Invariant: a call that answers an error of the server gives that error
//! back, and one that answers something the protocol does not allow gives
//! [`Error::InvalidArgument`]; a caller cannot tell the two apart and does
//! not need to.

use audhsos_abi::Error;
use user_proto::{console, memory, name};
use user_rt::{EndpointHandle, MemoryHandle, Typed};
use user_sys_x86_64::Gate;

/// Puts `endpoint` under `name` at the name server.
///
/// # Errors
///
/// Whatever the server answered, and the errors of the call itself.
pub fn register(
    gate: &mut Gate,
    server: EndpointHandle,
    label: &[u8],
    endpoint: EndpointHandle,
) -> Result<(), Error> {
    let request = name::Request::Register {
        name: name::Name::new(label)?,
        endpoint: endpoint.handle(),
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    match name::Reply::decode(gate.reader())? {
        name::Reply::Registered(outcome) => outcome,
        name::Reply::Found(_) => Err(Error::InvalidArgument),
    }
}

/// Asks the name server for the endpoint `label` stands for.
///
/// # Errors
///
/// [`Error::NotFound`] when nothing is registered under the name, and the
/// errors of the call itself.
pub fn lookup(
    gate: &mut Gate,
    server: EndpointHandle,
    label: &[u8],
) -> Result<EndpointHandle, Error> {
    let request = name::Request::Lookup {
        name: name::Name::new(label)?,
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    match name::Reply::decode(gate.reader())? {
        name::Reply::Found(outcome) => outcome.map(EndpointHandle::from_handle),
        name::Reply::Registered(_) => Err(Error::InvalidArgument),
    }
}

/// Asks the memory server for `len` bytes aligned to `align`.
///
/// # Errors
///
/// Whatever the server answered, and the errors of the call itself.
pub fn allocate(
    gate: &mut Gate,
    server: EndpointHandle,
    len: u64,
    align: u64,
) -> Result<MemoryHandle, Error> {
    let request = memory::Request::Allocate { len, align };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    match memory::Reply::decode(gate.reader())? {
        memory::Reply::Allocated(outcome) => outcome.map(MemoryHandle::from_handle),
        memory::Reply::Released(_) => Err(Error::InvalidArgument),
    }
}

/// Gives a memory object back to the memory server.
///
/// # Errors
///
/// Whatever the server answered, and the errors of the call itself.
pub fn release(gate: &mut Gate, server: EndpointHandle, object: MemoryHandle) -> Result<(), Error> {
    let request = memory::Request::Release {
        memory: object.handle(),
    };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    match memory::Reply::decode(gate.reader())? {
        memory::Reply::Released(outcome) => outcome,
        memory::Reply::Allocated(_) => Err(Error::InvalidArgument),
    }
}

/// Writes `bytes` to the console driver, in as many messages as it takes.
///
/// # Errors
///
/// Whatever the driver answered, and the errors of the call itself.
pub fn write_line(gate: &mut Gate, server: EndpointHandle, bytes: &[u8]) -> Result<(), Error> {
    for chunk in bytes.chunks(console::MAX_CHUNK) {
        let request = console::Request::Write {
            bytes: console::Chunk::new(chunk)?,
        };
        request.encode(&mut gate.writer())?;
        gate.ipc_call(server)?;
        match console::Reply::decode(gate.reader())? {
            console::Reply::Written(outcome) => outcome?,
            console::Reply::Read(_) => return Err(Error::InvalidArgument),
        };
    }
    Ok(())
}

/// Asks the console driver for at most `max` bytes that have arrived, into
/// `into`, and answers with how many there were.
///
/// # Errors
///
/// Whatever the driver answered, and the errors of the call itself.
pub fn read_bytes(
    gate: &mut Gate,
    server: EndpointHandle,
    max: u64,
    into: &mut [u8],
) -> Result<usize, Error> {
    let request = console::Request::Read { max };
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    match console::Reply::decode(gate.reader())? {
        console::Reply::Read(outcome) => {
            let chunk = outcome?;
            let taken = chunk.len().min(into.len());
            if let (Some(slot), Some(from)) = (into.get_mut(..taken), chunk.as_bytes().get(..taken))
            {
                slot.copy_from_slice(from);
            }
            Ok(taken)
        }
        console::Reply::Written(_) => Err(Error::InvalidArgument),
    }
}
