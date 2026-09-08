// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the policy needs of the kernel, and nothing else.
//!
//! Seven operations: map an object into the server's own address space, fill
//! it with zeros, take it out again, split an object, join two that lie
//! side by side, give a capability up, and count its references. Everything the memory server
//! does with memory is one of those, so a double that records them is a
//! complete account of what a run of the policy did — which is what
//! [6.6.23](../../../../docs/06-testing-strategy.md#6623-memory-server-logic-server-memory-host-tested-with-a-recording-double-for-map-zero-and-unmap)
//! asks for.
//!
//! Map and zero are apart, so that a test can see the fill happen and say
//! over which bytes. An object is mapped and zeroed in windows of
//! [`WINDOW`](crate::store::WINDOW) bytes and not whole: a region of memory
//! this machine hands the server is hundreds of mebibytes, and an address
//! space region that wide would need more page tables than the kernel
//! reserve holds. The zeroing of an object is therefore a run of calls
//! whose ranges follow one another and cover it exactly once, which for
//! every object smaller than the window is one call.

use audhsos_abi::{Error, Handle};

/// The operations the policy makes on memory.
pub trait Pages {
    /// Counts handles and mappings, including this server's handle.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered; an unknown count is never exclusive.
    fn references(&mut self, object: Handle) -> Result<u64, Error>;

    /// Maps `len` bytes of `object`, from `offset`, into the server's own
    /// address space, and answers with the address they went to.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    fn map(&mut self, object: Handle, offset: u64, len: u64) -> Result<u64, Error>;

    /// Overwrites `len` bytes at `address` with zeros.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    fn zero(&mut self, address: u64, len: u64) -> Result<(), Error>;

    /// Takes `len` bytes at `address` out of the server's address space.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    fn unmap(&mut self, address: u64, len: u64) -> Result<(), Error>;

    /// Splits `object` at `offset` and answers with the upper part. The
    /// object keeps the lower part.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    fn split(&mut self, object: Handle, offset: u64) -> Result<Handle, Error>;

    /// Joins `upper` into `lower`, which then covers both. `upper` ceases
    /// to exist.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered. [`Error::InvalidArgument`] for two
    /// objects the kernel will not join, which includes two that something
    /// other than this server still holds.
    fn merge(&mut self, lower: Handle, upper: Handle) -> Result<(), Error>;

    /// Gives up `handle`. What it named lives on while anything else names
    /// it.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    fn close(&mut self, handle: Handle) -> Result<(), Error>;
}
