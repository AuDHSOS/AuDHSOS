// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod align;
pub mod cache;
pub mod error;
pub mod phys;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;
pub mod virt;

pub use align::Alignment;
pub use cache::CachePolicy;
pub use error::Error;
pub use phys::{PhysAddr, PhysFrame, PhysFrameRange};
pub use virt::{Page, PageRange, VirtAddr};

#[cfg(test)]
mod tests;
