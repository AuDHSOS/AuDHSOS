// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one function that moves a message from one IPC buffer to another.
//!
//! Invariants: a message the header or the handles of which are refused
//! changes nothing at all — not a word of the receiver's buffer and not one
//! entry of its handle list; a handle that arrives carries the rights and
//! the badge the sender's handle carried and adds a reference to the object
//! it names; the receiver's handle count says how many arrived and never
//! how many were sent.
//!
//! The order is what makes that true. The header is validated first, then
//! every handle is resolved in the sender's list and checked for
//! [`Rights::TRANSFER`], and only then are the words copied and the handles
//! installed. A handle without that right therefore fails before any other
//! handle of the same message is installed.
//!
//! The reserved label range is not checked here but at the entry of
//! `ipc_send` and `ipc_call`, where the message is one a user thread wrote.
//! The fault message the kernel builds is its own and carries a reserved
//! label by construction; a check inside this function would refuse exactly
//! the message it exists for.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, WORD, WORDS};
use audhsos_abi::layout::MAX_MESSAGE_HANDLES;
use audhsos_abi::{Error, Rights};
use kernel_objects::handle_table::Entry;
use kernel_objects::object::ProcessId;
use kernel_objects::store::Objects;

/// What a transfer moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Transferred {
    /// How many payload words arrived.
    pub words: u16,
    /// How many handles arrived, which is at most how many were sent.
    pub handles: u8,
    /// Some handle did not fit into the receiver's list. The message is
    /// delivered all the same, and the receiver's status word carries
    /// [`audhsos_abi::ipc_buffer::Status::PARTIAL`]: this is the error flag
    /// of 2.6.2 and not an error.
    pub truncated: bool,
}

/// Moves the message in `from` into `to`.
///
/// The sender keeps its handles; the receiver gets a second handle to each
/// object with the same rights and the same badge, and the object gains a
/// reference for it.
///
/// The two processes may be one process, so the handle list is read out of
/// the store and written back rather than passed in twice: two mutable
/// borrows of one list would be two views of the same thing, and whichever
/// was written back last would win.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a header whose counts are above what the
/// message area holds; [`Error::InvalidHandle`] for a handle word the
/// sender does not hold; [`Error::AccessDenied`] for a handle without
/// [`Rights::TRANSFER`]. Nothing is copied and nothing is installed in any
/// of the three cases.
pub fn transfer<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    from: &[u8; SIZE],
    to: &mut [u8; SIZE],
    objects: &mut Objects<NP, NT, NM, NH>,
    sender: ProcessId,
    receiver: ProcessId,
) -> Result<Transferred, Error> {
    let reader = Buffer::new(from);
    let message = reader.message().map_err(Error::from)?;

    // Every handle first, so that one without `TRANSFER` refuses the
    // message before another of the same message is installed.
    let mut carried = [None; MAX_MESSAGE_HANDLES];
    for (index, slot) in carried.iter_mut().enumerate().take(message.handle_count) {
        let handle = reader.handle(index).ok_or(Error::InvalidHandle)?;
        let entry = *objects.handles.lookup(sender, handle)?;
        if !entry.rights.contains(Rights::TRANSFER) {
            return Err(Error::AccessDenied);
        }
        *slot = Some(entry);
    }

    let words = copy_words(from, to, message.word_count);
    let mut writer = BufferMut::new(to);
    writer.set_label(message.label);

    let mut handles: u8 = 0;
    let mut truncated = false;
    for entry in carried.iter().flatten() {
        let Some(handle) = install(objects, receiver, *entry) else {
            truncated = true;
            break;
        };
        BufferMut::new(to).set_handle(usize::from(handles), handle);
        handles = handles.saturating_add(1);
    }
    let mut writer = BufferMut::new(to);
    writer
        .set_counts(message.word_count, usize::from(handles))
        .map_err(Error::from)?;
    Ok(Transferred {
        words,
        handles,
        truncated,
    })
}

/// Copies `count` payload words and returns how many were copied.
///
/// The count has been checked against the message area, so the range lies
/// inside both pages and the copy is one move of bytes.
fn copy_words(from: &[u8; SIZE], to: &mut [u8; SIZE], count: usize) -> u16 {
    let end = WORDS.saturating_add(count.saturating_mul(WORD));
    if let Some(source) = from.get(WORDS..end)
        && let Some(target) = to.get_mut(WORDS..end)
    {
        target.copy_from_slice(source);
    }
    u16::try_from(count).unwrap_or(0)
}

/// Installs a second handle to what `entry` names in `receiver`, and adds
/// the reference that handle holds. `None` says the receiver had no slot.
fn install<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    receiver: ProcessId,
    entry: Entry,
) -> Option<audhsos_abi::Handle> {
    let handle = objects.install_handle(receiver, entry).ok()?;
    // The reference the new handle holds. It cannot fail: the entry names an
    // object that was live a moment ago, when the handle of the sender was
    // resolved, and the count is bounded by the handle arena and the region
    // tables of the machine together — far below what a `u32` holds.
    let _ = objects.retain(entry.object);
    Some(handle)
}
