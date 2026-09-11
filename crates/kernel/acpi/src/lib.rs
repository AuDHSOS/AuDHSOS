// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod madt;
pub mod mcfg;
pub mod raw;
pub mod rsdp;
pub mod sdt;

pub use error::AcpiError;
pub use madt::{IoApic, MAX_IO_APICS, MAX_OVERRIDES, Madt, Override, Polarity, Routing, Trigger};
pub use mcfg::{BYTES_PER_BUS, Ecam, MAX_ECAM_ALLOCATIONS, Mcfg};
pub use rsdp::{RSDP_LEN, Rsdp, parse_rsdp};
pub use sdt::{RootTable, SDT_HEADER_LEN, SdtHeader, announced_length};

#[cfg(test)]
mod tests;
