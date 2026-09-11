// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod address;
pub mod bar;
pub mod capability;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod enumerate;
pub mod error;
pub mod header;
pub mod msix;
pub mod space;
pub mod virtio;

pub use address::{Address, CONFIG_SPACE_LEN, MAX_DEVICE, MAX_FUNCTION, Window};
pub use bar::{Bar, MAX_BARS, Space as BarSpace, Width};
pub use capability::{Capability, ID_MSIX, ID_VENDOR, MAX_CAPABILITIES};
pub use enumerate::Function;
pub use error::PciError;
pub use header::{Header, Kind, Subsystem};
pub use msix::{Entry, MsiX};
pub use space::ConfigSpace;
pub use virtio::{Kind as VirtioKind, Structure};

#[cfg(test)]
mod tests;

#[cfg(test)]
use test_support as _;
