// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One request: the header the device reads, the status byte it writes,
//! and the rules about what may stand between them (virtio 5.2.6).

use crate::error::BlkError;

/// Bytes of the header: the type, the reserved word, and the sector.
pub const HEADER_LEN: u32 = 16;

/// Bytes of the status the device writes.
pub const STATUS_LEN: u32 = 1;

/// `VIRTIO_BLK_S_OK`.
pub const S_OK: u8 = 0;

/// `VIRTIO_BLK_S_IOERR`.
pub const S_IOERR: u8 = 1;

/// `VIRTIO_BLK_S_UNSUPP`.
pub const S_UNSUPP: u8 = 2;

/// What a request asks the device to do.
///
/// Three of the eight types virtio 5.2.6 has. The five that are missing —
/// get id, get lifetime, discard, write zeroes, secure erase — each need
/// a feature this driver does not take or a framing rule of their own,
/// and a file system server needs none of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `VIRTIO_BLK_T_IN`: the device writes sectors into the data.
    In,
    /// `VIRTIO_BLK_T_OUT`: the device reads sectors out of the data.
    Out,
    /// `VIRTIO_BLK_T_FLUSH`: what was written reaches the storage.
    Flush,
}

impl Kind {
    /// The number the header carries.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Kind::In => 0,
            Kind::Out => 1,
            Kind::Flush => 4,
        }
    }

    /// Whether the device writes the data of such a request.
    #[must_use]
    pub const fn writes_data(self) -> bool {
        matches!(self, Kind::In)
    }

    /// Whether such a request changes what the device holds.
    #[must_use]
    pub const fn changes_device(self) -> bool {
        matches!(self, Kind::Out | Kind::Flush)
    }
}

/// What a request asks for, without saying where the bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Request {
    /// What to do.
    pub kind: Kind,
    /// The first sector, counted in 512 bytes whatever the device says
    /// its block size is (virtio 5.2.5, point 2).
    pub sector: u64,
}

impl Request {
    /// A read of `sector`.
    #[must_use]
    pub const fn read(sector: u64) -> Request {
        Request {
            kind: Kind::In,
            sector,
        }
    }

    /// A write of `sector`.
    #[must_use]
    pub const fn write(sector: u64) -> Request {
        Request {
            kind: Kind::Out,
            sector,
        }
    }

    /// A flush, whose sector virtio 5.2.6.1 requires to be zero.
    #[must_use]
    pub const fn flush() -> Request {
        Request {
            kind: Kind::Flush,
            sector: 0,
        }
    }

    /// Writes the header into `into`, which the device reads.
    ///
    /// # Errors
    ///
    /// [`BlkError::Buffer`] for fewer than [`HEADER_LEN`] bytes, and
    /// [`BlkError::FlushSector`] for a flush of a sector other than zero.
    pub fn write_header(&self, into: &mut [u8]) -> Result<(), BlkError> {
        if self.kind == Kind::Flush && self.sector != 0 {
            return Err(BlkError::FlushSector(self.sector));
        }
        let len = usize::try_from(HEADER_LEN).unwrap_or(usize::MAX);
        let given = into.len();
        let header = into.get_mut(..len).ok_or(BlkError::Buffer(given))?;
        let mut bytes = [0u8; 16];
        if let Some(slot) = bytes.get_mut(0..4) {
            slot.copy_from_slice(&self.kind.code().to_le_bytes());
        }
        if let Some(slot) = bytes.get_mut(8..16) {
            slot.copy_from_slice(&self.sector.to_le_bytes());
        }
        header.copy_from_slice(&bytes);
        Ok(())
    }
}

/// What the device made of a request.
///
/// # Errors
///
/// [`BlkError::Status`] for anything but [`S_OK`], the byte included so
/// that a caller can tell an error of the device from a request it would
/// not take.
pub const fn status(byte: u8) -> Result<(), BlkError> {
    if byte == S_OK {
        Ok(())
    } else {
        Err(BlkError::Status(byte))
    }
}
