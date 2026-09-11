// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Builders that lay out a function's configuration space, so that every
//! test starts from bytes a device could answer with.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::bar::MAX_BARS;
use crate::capability::ID_VENDOR;
use crate::doubles::{RECORDED_LEN, RecordedConfigSpace};
use crate::header::{BAR0, CAPABILITY_POINTER, STATUS_CAPABILITIES};

/// The vendor and device of the virtio network device of the reference
/// machine.
pub(crate) const VIRTIO_NET: (u16, u16) = (0x1AF4, 0x1041);

/// One function's configuration space, under construction.
pub(crate) struct Builder {
    bytes: [u8; RECORDED_LEN],
    masks: [u32; MAX_BARS],
}

impl Builder {
    /// A function of `vendor` and `device`, type-0, decoding memory and
    /// I/O, with nothing else in it.
    pub(crate) fn new(vendor: u16, device: u16) -> Builder {
        let mut builder = Builder {
            bytes: [0; RECORDED_LEN],
            masks: [0; MAX_BARS],
        };
        builder.word(0x00, u32::from(vendor) | (u32::from(device) << 16));
        builder.word(0x04, 0x0000_0003);
        builder
    }

    /// Writes a word at `offset`.
    pub(crate) fn word(&mut self, offset: u16, value: u32) -> &mut Builder {
        let at = offset as usize;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        self
    }

    /// Writes a byte at `offset`.
    pub(crate) fn byte(&mut self, offset: u16, value: u8) -> &mut Builder {
        self.bytes[offset as usize] = value;
        self
    }

    /// Sets the header type, which carries the multi-function bit.
    pub(crate) fn header_type(&mut self, value: u8) -> &mut Builder {
        self.byte(0x0E, value)
    }

    /// Sets the class, subclass, and programming interface.
    pub(crate) fn class(&mut self, class: u8, subclass: u8, prog_if: u8) -> &mut Builder {
        self.word(
            0x08,
            (u32::from(class) << 24) | (u32::from(subclass) << 16) | (u32::from(prog_if) << 8),
        )
    }

    /// Puts a base address register at `index`, with the mask a probe of it
    /// answers.
    pub(crate) fn bar(&mut self, index: u8, value: u32, mask: u32) -> &mut Builder {
        self.masks[index as usize] = mask;
        self.word(BAR0 + u16::from(index) * 4, value)
    }

    /// Names the first capability of the list and sets the status bit that
    /// says there is one.
    pub(crate) fn capabilities(&mut self, first: u8) -> &mut Builder {
        self.word(0x04, 0x0000_0003 | (u32::from(STATUS_CAPABILITIES) << 16));
        self.byte(CAPABILITY_POINTER, first)
    }

    /// Puts a capability of `id` at `offset`, pointing at `next`, with
    /// `body` after the two header bytes.
    pub(crate) fn capability(&mut self, offset: u8, id: u8, next: u8, body: &[u8]) -> &mut Builder {
        let at = offset as usize;
        self.bytes[at] = id;
        self.bytes[at + 1] = next;
        self.bytes[at + 2..at + 2 + body.len()].copy_from_slice(body);
        self
    }

    /// Puts a virtio capability at `offset`.
    pub(crate) fn virtio(
        &mut self,
        at: (u8, u8),
        cfg_type: u8,
        bar: u8,
        structure: u32,
        len: u32,
        multiplier: Option<u32>,
    ) -> &mut Builder {
        let (offset, next) = at;
        let announced = if multiplier.is_some() { 20u8 } else { 16u8 };
        let mut body = vec![announced, cfg_type, bar, 0, 0, 0];
        body.extend_from_slice(&structure.to_le_bytes());
        body.extend_from_slice(&len.to_le_bytes());
        if let Some(multiplier) = multiplier {
            body.extend_from_slice(&multiplier.to_le_bytes());
        }
        self.capability(offset, ID_VENDOR, next, &body)
    }

    /// Puts an MSI-X capability at `offset`.
    pub(crate) fn msix(
        &mut self,
        offset: u8,
        next: u8,
        vectors: u16,
        table: (u8, u32),
        pending: (u8, u32),
    ) -> &mut Builder {
        let control = vectors - 1;
        let mut body = Vec::new();
        body.extend_from_slice(&control.to_le_bytes());
        body.extend_from_slice(&(table.1 | u32::from(table.0)).to_le_bytes());
        body.extend_from_slice(&(pending.1 | u32::from(pending.0)).to_le_bytes());
        self.capability(offset, 0x11, next, &body)
    }

    /// The bytes and the masks, as the double takes them.
    pub(crate) fn build(&self) -> ([u8; RECORDED_LEN], [u32; MAX_BARS]) {
        (self.bytes, self.masks)
    }

    /// Installs the function as `device.function` of `space`.
    pub(crate) fn install(&self, space: &mut RecordedConfigSpace, device: u8, function: u8) {
        let (bytes, masks) = self.build();
        assert!(
            space.install(device, function, bytes, masks),
            "the space holds no further function"
        );
    }
}

/// A space with one function at `0.0`.
pub(crate) fn one(builder: &Builder) -> RecordedConfigSpace {
    let mut space = RecordedConfigSpace::blank();
    builder.install(&mut space, 0, 0);
    space
}
