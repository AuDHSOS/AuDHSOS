// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(abi_x86_interrupt)]
#![doc = include_str!("../README.md")]

pub mod bootinfo;
#[cfg(feature = "debug-uart")]
pub mod console;
pub mod descriptors;
pub mod entry;
#[cfg(feature = "test-exit")]
pub mod exit;
pub mod instructions;
pub mod paging;
pub mod traps;
