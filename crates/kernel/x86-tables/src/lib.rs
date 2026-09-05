// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod gdt;
pub mod idt;
pub mod tss;

pub use gdt::{GDT_ENTRIES, Selector, build_gdt};
pub use idt::{IDT_ENTRIES, gate};
pub use tss::{TSS_LEN, TaskStateSegment};

#[cfg(test)]
mod tests;
