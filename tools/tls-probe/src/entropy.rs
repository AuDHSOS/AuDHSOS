// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The host's entropy, standing in for the source the kernel will have.
//!
//! `crypto_rng::source` says one implementation of [`Entropy`] exists per
//! platform, and that on this system it will read `RDSEED` through a
//! system call. There is no such call yet, so the probe reads the device
//! the host offers. Nothing above this file can tell the difference: the
//! protocol sees a [`crypto_rng::Rng`], and the generator behind it is the
//! project's own `ChaCha20` construction either way.

use std::fs::File;
use std::io::Read;

use crypto_rng::{Entropy, EntropyError};

/// The host device, held open for the life of the probe.
pub struct OsEntropy {
    /// `/dev/urandom`.
    device: File,
}

impl OsEntropy {
    /// Opens the device.
    ///
    /// # Errors
    ///
    /// Whatever opening `/dev/urandom` reports.
    pub fn open() -> std::io::Result<OsEntropy> {
        Ok(OsEntropy {
            device: File::open("/dev/urandom")?,
        })
    }
}

impl Entropy for OsEntropy {
    fn fill(&mut self, out: &mut [u8]) -> Result<(), EntropyError> {
        self.device
            .read_exact(out)
            .map_err(|_| EntropyError::Unavailable)
    }
}
