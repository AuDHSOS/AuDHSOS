// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A configuration space in ordinary bytes, and the recording of a real
//! one.
//!
//! [`RecordedConfigSpace::q35`] is the configuration space of QEMU's `q35`
//! machine with `-device virtio-net-pci,disable-legacy=on,mq=off`, read out
//! of the ECAM window of a running machine through the monitor and kept as
//! the byte fixture `src/fixtures/q35.bin`. The five functions are the host
//! bridge, the network device, the ISA bridge, the SATA controller, and the
//! `SMBus` controller, in that order, two hundred and fifty-six bytes each.
//!
//! Two things a byte dump cannot carry. The first is what a register
//! answers after all ones have been written to it, so the size masks the
//! machine reported are recorded beside the bytes and a write of all ones
//! to a base address register makes the next read answer the mask. The
//! second is the status register that shares its word with the command
//! register: its bits are read-only or cleared by writing a one, so a write
//! of the command word leaves the upper half as it stood. Every other write
//! is stored as it came.

use crate::address::Address;
use crate::bar::MAX_BARS;
use crate::header::{BAR0, COMMAND};
use crate::space::ConfigSpace;

/// Number of functions a space of this double holds.
pub const MAX_FUNCTIONS: usize = 8;

/// Number of bytes of a function this double holds, which is the space the
/// capability list lies in. Above them the machine this was read from
/// answers zero, because it publishes no extended capability.
pub const RECORDED_LEN: usize = 256;

/// The recorded bytes of the `q35` machine, five functions of
/// [`RECORDED_LEN`] bytes.
const Q35_BYTES: &[u8] = include_bytes!("fixtures/q35.bin");

/// What a function of the `q35` machine is: where it sits, and what its
/// base address registers answer a probe with.
const Q35_FUNCTIONS: [(u8, u8, [u32; MAX_BARS]); 5] = [
    // The host bridge decodes nothing.
    (0, 0, [0; MAX_BARS]),
    // The network device: four kibibytes of memory in register one, and
    // sixteen kibibytes of prefetchable 64-bit memory in registers four
    // and five.
    (1, 0, [0, 0xFFFF_F000, 0, 0, 0xFFFF_C00C, 0xFFFF_FFFF]),
    // The ISA bridge decodes nothing.
    (31, 0, [0; MAX_BARS]),
    // The SATA controller: thirty-two I/O ports in register four and four
    // kibibytes of memory in register five.
    (31, 2, [0, 0, 0, 0, 0xFFFF_FFE1, 0xFFFF_F000]),
    // The SMBus controller: sixty-four I/O ports in register four.
    (31, 3, [0, 0, 0, 0, 0xFFFF_FFC1, 0]),
];

/// The word an absent function answers with.
const ABSENT_WORD: u32 = 0xFFFF_FFFF;

/// One function of the space.
#[derive(Clone, Copy, Debug)]
struct Held {
    device: u8,
    number: u8,
    bytes: [u8; RECORDED_LEN],
    masks: [u32; MAX_BARS],
}

/// A configuration space of one bus, in bytes a test owns.
#[derive(Clone, Copy, Debug)]
pub struct RecordedConfigSpace {
    segment: u16,
    bus: u8,
    functions: [Option<Held>; MAX_FUNCTIONS],
}

impl RecordedConfigSpace {
    /// A space of segment zero, bus zero, with no function on it.
    #[must_use]
    pub const fn blank() -> RecordedConfigSpace {
        RecordedConfigSpace {
            segment: 0,
            bus: 0,
            functions: [None; MAX_FUNCTIONS],
        }
    }

    /// The recorded space of the `q35` machine with a virtio-net device.
    #[must_use]
    pub fn q35() -> RecordedConfigSpace {
        let mut space = RecordedConfigSpace::blank();
        for (index, (device, function, masks)) in Q35_FUNCTIONS.iter().enumerate() {
            let at = index.saturating_mul(RECORDED_LEN);
            let bytes = Q35_BYTES
                .get(at..at.saturating_add(RECORDED_LEN))
                .and_then(|slice| slice.first_chunk::<RECORDED_LEN>().copied())
                .unwrap_or([0; RECORDED_LEN]);
            space.install(*device, *function, bytes, *masks);
        }
        space
    }

    /// The space with `bytes` as the function `device.function`, and
    /// `masks` as what its base address registers answer a probe with.
    pub fn install(
        &mut self,
        device: u8,
        function: u8,
        bytes: [u8; RECORDED_LEN],
        masks: [u32; MAX_BARS],
    ) -> bool {
        let Some(slot) = self.functions.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(Held {
            device,
            number: function,
            bytes,
            masks,
        });
        true
    }

    /// Writes `value` into the bytes of `device.function` without the
    /// probing a [`ConfigSpace::write_u32`] does, which is how a test lays
    /// out a function of its own.
    pub fn poke(&mut self, device: u8, function: u8, offset: u16, value: u32) -> bool {
        let Some(found) = self.find_mut(device, function) else {
            return false;
        };
        store(&mut found.bytes, offset, value)
    }

    /// The bytes of `device.function`, for a test that reads back what a
    /// write left.
    #[must_use]
    pub fn peek(&self, device: u8, function: u8, offset: u16) -> Option<u32> {
        load(&self.find(device, function)?.bytes, offset)
    }

    /// The address of `device.function` in this space.
    #[must_use]
    pub fn address(&self, device: u8, function: u8) -> Option<Address> {
        Address::new(self.segment, self.bus, device, function).ok()
    }

    /// The function at `address`, or `None` when the space has none there.
    fn at(&self, address: Address) -> Option<&Held> {
        if address.segment() != self.segment || address.bus() != self.bus {
            return None;
        }
        self.find(address.device(), address.function())
    }

    /// The function at `address`, for modification.
    fn at_mut(&mut self, address: Address) -> Option<&mut Held> {
        if address.segment() != self.segment || address.bus() != self.bus {
            return None;
        }
        self.find_mut(address.device(), address.function())
    }

    /// The function `device.function`.
    fn find(&self, device: u8, function: u8) -> Option<&Held> {
        self.functions
            .iter()
            .flatten()
            .find(|held| held.device == device && held.number == function)
    }

    /// The function `device.function`, for modification.
    fn find_mut(&mut self, device: u8, function: u8) -> Option<&mut Held> {
        self.functions
            .iter_mut()
            .flatten()
            .find(|held| held.device == device && held.number == function)
    }
}

impl ConfigSpace for RecordedConfigSpace {
    fn read_u32(&self, address: Address, offset: u16) -> Option<u32> {
        crate::address::check_offset(offset).ok()?;
        let Some(found) = self.at(address) else {
            return Some(ABSENT_WORD);
        };
        Some(load(&found.bytes, offset).unwrap_or(0))
    }

    fn write_u32(&mut self, address: Address, offset: u16, value: u32) {
        if crate::address::check_offset(offset).is_err() {
            return;
        }
        let Some(found) = self.at_mut(address) else {
            return;
        };
        let written = match probed(found, offset, value) {
            Some(mask) => mask,
            None => value,
        };
        let kept = match offset {
            COMMAND => keep_status(found, written),
            _ => written,
        };
        store(&mut found.bytes, offset, kept);
    }
}

/// The command word as it lands: the command half of `written` and the
/// status half the function already held, because no write of this crate
/// sets a status bit and a write of a zero clears none.
fn keep_status(function: &Held, written: u32) -> u32 {
    let held = load(&function.bytes, COMMAND).unwrap_or(0);
    (written & 0xFFFF) | (held & 0xFFFF_0000)
}

/// The mask a probe of the base address register at `offset` answers with,
/// or `None` when this write is not a probe.
fn probed(function: &Held, offset: u16, value: u32) -> Option<u32> {
    if value != ABSENT_WORD {
        return None;
    }
    let from_first = offset.checked_sub(BAR0)?;
    if from_first & 0b11 != 0 {
        return None;
    }
    function
        .masks
        .get(usize::from(from_first.wrapping_shr(2)))
        .copied()
}

/// The little-endian word at `offset`, or `None` beyond the bytes.
fn load(bytes: &[u8; RECORDED_LEN], offset: u16) -> Option<u32> {
    let at = usize::from(offset);
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(slice.first_chunk::<4>().copied()?))
}

/// Writes the little-endian word at `offset`; `false` beyond the bytes.
fn store(bytes: &mut [u8; RECORDED_LEN], offset: u16, value: u32) -> bool {
    let at = usize::from(offset);
    let Some(end) = at.checked_add(4) else {
        return false;
    };
    let Some(slice) = bytes.get_mut(at..end) else {
        return false;
    };
    slice.copy_from_slice(&value.to_le_bytes());
    true
}
