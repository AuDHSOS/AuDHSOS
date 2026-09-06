// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the policy needs of the kernel, and nothing else.
//!
//! Five operations: map an object into the server's own address space, fill
//! it with zeros, take it out again, split an object, and join two that lie
//! side by side. Everything the memory server does with memory is one of
//! those, so a double that records them is a complete account of what a run
//! of the policy did — which is what
//! [6.6.23](../../../../docs/06-testing-strategy.md#6623-memory-server-logic-server-memory-host-tested-with-a-recording-double-for-map-zero-and-unmap)
//! asks for.
//!
//! Map and zero are apart, and the object is mapped whole rather than in
//! pieces, so that one zeroing pass is one recorded call whose range is the
//! range of the object. A test can then say what the catalog says: exactly
//! one pass, over exactly these bytes, before the handle went out.

use audhsos_abi::{Error, Handle};

/// The operations the policy makes on memory.
pub trait Pages {
    /// Maps `object`, whose length is `len`, into the server's own address
    /// space, and answers with the address it went to.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    fn map(&mut self, object: Handle, len: u64) -> Result<u64, Error>;

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
    /// Whatever the kernel answered.
    fn merge(&mut self, lower: Handle, upper: Handle) -> Result<(), Error>;
}
