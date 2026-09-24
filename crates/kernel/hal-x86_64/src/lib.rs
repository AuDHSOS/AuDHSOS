// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(abi_x86_interrupt)]
#![doc = include_str!("../README.md")]

pub mod acpi;
pub mod apic;
pub mod bootinfo;
#[cfg(feature = "debug-uart")]
pub mod console;
pub mod context;
pub mod descriptors;
pub mod entry;
#[cfg(feature = "test-exit")]
pub mod exit;
pub mod instructions;
pub mod interrupts;
pub mod memory;
pub mod paging;
pub mod pic;
pub mod ports;
pub mod processor;
pub mod random;
pub mod remote;
pub mod startup;
#[cfg(all(feature = "debug-uart", feature = "test-exit"))]
pub mod testing;
pub mod timer;
pub mod traps;
pub mod window;

pub use kernel_x86_tables::vectors;

// Only the feature-gated modules use these crates.
#[cfg(not(feature = "debug-uart"))]
use driver_uart16550 as _;
#[cfg(not(all(feature = "debug-uart", feature = "test-exit")))]
use kernel_test_harness as _;
