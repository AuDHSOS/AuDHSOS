// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A recording implementation of [`Pages`], which is what
//! [6.6.23](../../../../docs/06-testing-strategy.md#6623-memory-server-logic-server-memory-host-tested-with-a-recording-double-for-map-zero-and-unmap)
//! asks the policy to be tested against.
//!
//! It keeps every call in the order it was made, so a test says what the
//! catalog says: one zeroing pass over exactly these bytes, before the
//! handle went out.
//!
//! It also plays the kernel: a split makes a second handle, a merge takes
//! one away, and a map hands out an address of its own. What it does not do
//! is hold bytes — nothing of the policy reads what it wrote.

extern crate alloc;

use alloc::vec::Vec;

use audhsos_abi::{Error, Handle};

use crate::pages::Pages;

/// One call the policy made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Call {
    /// A window of an object was mapped into the server's address space.
    Map {
        /// The object.
        object: Handle,
        /// Where in the object the window begins.
        offset: u64,
        /// How many bytes.
        len: u64,
        /// Where it went.
        address: u64,
    },
    /// A range was filled with zeros.
    Zero {
        /// Where the range begins.
        address: u64,
        /// How many bytes.
        len: u64,
    },
    /// A range was taken out of the address space.
    Unmap {
        /// Where the range begins.
        address: u64,
        /// How many bytes.
        len: u64,
    },
    /// An object was split.
    Split {
        /// The object that was split.
        object: Handle,
        /// Where.
        offset: u64,
        /// The handle to the upper part.
        upper: Handle,
    },
    /// Two objects were joined.
    Merge {
        /// The object that grew.
        lower: Handle,
        /// The object that ceased to exist.
        upper: Handle,
    },
    /// A capability was given up.
    Close {
        /// The handle.
        handle: Handle,
    },
}

/// The kernel as the tests play it.
#[derive(Debug, Default)]
pub struct RecordingPages {
    /// Objects still reachable outside the memory server.
    pub shared: Vec<Handle>,
    calls: Vec<Call>,
    next_handle: u32,
    next_address: u64,
    /// The call number after which every operation answers with this error.
    /// `None` lets everything succeed.
    pub fail_after: Option<(usize, Error)>,
    /// Whether a join is refused, which is what the kernel does when
    /// something other than the server still holds one of the two.
    pub refuse_merges: bool,
}

/// Where the double puts the first mapping.
const FIRST_ADDRESS: u64 = 0x1000_0000;

/// The handle the double falls back on, which it never needs: a generation
/// of one makes a handle out of every index.
const FALLBACK: Handle = match Handle::new(1, 1) {
    Some(handle) => handle,
    // A generation of one always makes a handle, so this arm is what the
    // language asks for and not a case the double can meet.
    None => Handle::MAX,
};

impl RecordingPages {
    /// A double that has recorded nothing.
    #[must_use]
    pub const fn new() -> Self {
        RecordingPages {
            shared: Vec::new(),
            calls: Vec::new(),
            next_handle: 1000,
            next_address: FIRST_ADDRESS,
            fail_after: None,
            refuse_merges: false,
        }
    }

    /// Every call, in the order it was made.
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Forgets what was recorded, so that a test can watch one operation
    /// after a set-up that made calls of its own.
    pub fn forget(&mut self) {
        self.calls.clear();
    }

    /// The zeroing passes that were made, as ranges.
    #[must_use]
    pub fn zeroed(&self) -> Vec<(u64, u64)> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                Call::Zero { address, len } => Some((*address, *len)),
                _ => None,
            })
            .collect()
    }

    /// A handle nothing has handed out yet.
    #[must_use]
    pub fn fresh_handle(&mut self) -> Handle {
        self.next_handle = self.next_handle.wrapping_add(1);
        Handle::new(self.next_handle, 1).unwrap_or(FALLBACK)
    }

    /// Records `call` and says whether the operation may go on.
    fn record(&mut self, call: Call) -> Result<(), Error> {
        self.calls.push(call);
        match self.fail_after {
            Some((after, error)) if self.calls.len() > after => Err(error),
            _ => Ok(()),
        }
    }
}

impl Pages for RecordingPages {
    fn references(&mut self, object: Handle) -> Result<u64, Error> {
        Ok(if self.shared.contains(&object) { 2 } else { 1 })
    }
    fn map(&mut self, object: Handle, offset: u64, len: u64) -> Result<u64, Error> {
        let address = self.next_address;
        self.next_address = self.next_address.saturating_add(len).next_multiple_of(4096);
        self.record(Call::Map {
            object,
            offset,
            len,
            address,
        })?;
        Ok(address)
    }

    fn zero(&mut self, address: u64, len: u64) -> Result<(), Error> {
        self.record(Call::Zero { address, len })
    }

    fn unmap(&mut self, address: u64, len: u64) -> Result<(), Error> {
        self.record(Call::Unmap { address, len })
    }

    fn split(&mut self, object: Handle, offset: u64) -> Result<Handle, Error> {
        let upper = self.fresh_handle();
        self.record(Call::Split {
            object,
            offset,
            upper,
        })?;
        Ok(upper)
    }

    fn merge(&mut self, lower: Handle, upper: Handle) -> Result<(), Error> {
        self.record(Call::Merge { lower, upper })?;
        if self.refuse_merges {
            return Err(Error::InvalidArgument);
        }
        Ok(())
    }

    fn close(&mut self, handle: Handle) -> Result<(), Error> {
        self.record(Call::Close { handle })
    }
}
