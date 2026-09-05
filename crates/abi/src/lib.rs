// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod boot_image;
pub mod boot_info;
pub mod error;
pub mod handle;
pub mod ipc_buffer;
pub mod layout;
pub mod object;
pub mod rights;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;
pub mod syscall;
pub mod thread;

pub use boot_image::{BootImageError, BootImageHeader};
pub use boot_info::{
    BootInfoError, BootInfoView, BootInfoWriter, BootRegion, BootRegionKind, Framebuffer,
    FramebufferFormat,
};
pub use error::Error;
pub use handle::Handle;
pub use ipc_buffer::{Buffer, BufferMut, Message, MessageError, Status};
pub use object::ObjectType;
pub use rights::Rights;
pub use syscall::{FirstArgument, Syscall};
pub use thread::{Fault, FaultKind, ThreadState};

#[cfg(test)]
mod tests;
