// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What every test file here needs: a device that has reached
//! `DRIVER_OK`, and the buffers of a chain.

use crate::descriptor::Buffer;
use crate::device::{Device, F_VERSION_1};
use crate::doubles::{RamQueue, ScriptedRegisters};

/// A device driven through the whole initialization, and the registers it
/// was driven through.
pub(crate) fn live_device() -> (Device, ScriptedRegisters) {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    device.driver(&mut registers).expect("driver");
    device.negotiate(&mut registers, 0).expect("negotiate");
    device.driver_ok(&mut registers).expect("driver ok");
    (device, registers)
}

/// `count` buffers the device reads, at recognisable addresses.
pub(crate) fn to_device(count: usize) -> Vec<Buffer> {
    (0..count)
        .map(|index| {
            let step = u64::try_from(index).unwrap_or(0).saturating_mul(0x100);
            Buffer::to_device(0x1000u64.saturating_add(step), 64)
        })
        .collect()
}

/// The descriptor indices of the chain at `head`, in order.
pub(crate) fn chain_of(memory: &RamQueue, head: u16) -> Vec<u16> {
    let mut indices = vec![head];
    let mut current = head;
    while let Some(descriptor) = memory.descriptor(current) {
        if !descriptor.has_next() {
            break;
        }
        current = descriptor.next;
        indices.push(current);
    }
    indices
}
