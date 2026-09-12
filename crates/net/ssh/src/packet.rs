// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The binary packet of RFC 4253, section 6, and the sequence numbers of
//! section 6.4.
//!
//! A packet is a `uint32` length, a padding length byte, the payload, and
//! the padding — with the length of all four a multiple of the cipher
//! block size or eight, whichever is larger, and at least four bytes of
//! padding. No integrity tag is appended here: the one cipher this client
//! offers is an AEAD that carries its own, and that is step S3.
//!
//! One [`Encoder`] and one [`Decoder`] per direction, because each holds
//! the sequence number of its own.

use crypto_rng::Rng;

use crate::error::SshError;
use crate::wire::{Reader, Writer};

/// The largest payload every implementation must be able to receive
/// (RFC 4253, section 6.1).
pub const MAX_PAYLOAD: usize = 32768;

/// The largest whole packet every implementation must be able to receive
/// (RFC 4253, section 6.1). A receive buffer of this size is enough for
/// any peer that keeps to the document.
pub const MAX_PACKET: usize = 35000;

/// The largest packet that can be one: the largest payload, the header,
/// and the most padding a byte can count. Nothing between this and
/// [`MAX_PACKET`] can satisfy both bounds of section 6.1 at once, so
/// [`Decoder`] refuses it from the length field rather than buffering it
/// first.
pub const MAX_FRAME: usize = MAX_PAYLOAD + HEADER_BYTES + MAX_PADDING;

/// The smallest packet, from RFC 4253, section 6.
pub const MIN_PACKET: usize = 16;

/// The least padding RFC 4253, section 6, allows.
pub const MIN_PADDING: usize = 4;

/// The most padding that section allows, which is what the byte counting
/// it can express.
pub const MAX_PADDING: usize = 255;

/// The block size a packet is padded to before a cipher is negotiated,
/// which section 6 requires even of a stream cipher.
pub const MIN_BLOCK: usize = 8;

/// The largest block size this crate pads to. Above it a packet could
/// need more padding than the byte that counts it can express: the
/// padding is at most three bytes more than the block.
pub const MAX_BLOCK: usize = MAX_PADDING - 3;

/// The length field and the padding length byte.
const HEADER_BYTES: usize = 5;

/// The implicit packet sequence number of RFC 4253, section 6.4: a
/// `uint32` that never appears on the wire, starts at zero, is never
/// reset by a re-exchange, and wraps at 2^32.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SequenceNumber(u32);

impl SequenceNumber {
    /// The number of the first packet, which is zero.
    #[must_use]
    pub const fn new() -> SequenceNumber {
        SequenceNumber(0)
    }

    /// The number the next packet carries.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Counts one packet and answers with the number that packet had.
    pub const fn advance(&mut self) -> u32 {
        let used = self.0;
        self.0 = self.0.wrapping_add(1);
        used
    }
}

#[cfg(test)]
impl SequenceNumber {
    /// A number partway through the range, so that the wrap of section
    /// 6.4 can be reached without counting to it.
    pub(crate) const fn at(value: u32) -> SequenceNumber {
        SequenceNumber(value)
    }
}

/// The block size a packet is padded to, at least [`MIN_BLOCK`] and at
/// most [`MAX_BLOCK`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Block(usize);

impl Block {
    /// The block size for `size`.
    const fn new(size: usize) -> Result<Block, SshError> {
        if size < MIN_BLOCK || size > MAX_BLOCK {
            return Err(SshError::BlockSize(size));
        }
        Ok(Block(size))
    }

    /// The length a packet of `content` bytes is padded to: the next
    /// multiple of the block at or above `content` plus the four bytes of
    /// padding the document requires.
    const fn padded(self, content: usize) -> usize {
        content.saturating_add(MIN_PADDING).next_multiple_of(self.0)
    }

    /// Whether `length` is a whole number of blocks.
    const fn divides(self, length: usize) -> bool {
        length.next_multiple_of(self.0) == length
    }
}

/// Frames outgoing packets, and counts them.
#[derive(Clone, Debug)]
pub struct Encoder {
    /// What the packet length is padded to.
    block: Block,
    /// The number of the next packet.
    sequence: SequenceNumber,
}

impl Encoder {
    /// An encoder for the connection before a cipher is negotiated, where
    /// the block size is [`MIN_BLOCK`].
    #[must_use]
    pub const fn new() -> Encoder {
        Encoder {
            block: Block(MIN_BLOCK),
            sequence: SequenceNumber::new(),
        }
    }

    /// Pads to `block` from the next packet on, which is what a
    /// `SSH_MSG_NEWKEYS` decides. The sequence number is untouched: RFC
    /// 4253, section 6.4, does not reset it for a re-exchange, and there
    /// is no second constructor that would.
    ///
    /// # Errors
    ///
    /// [`SshError::BlockSize`] when `block` is below [`MIN_BLOCK`] or
    /// above [`MAX_BLOCK`]. The block in use does not change.
    pub const fn set_block(&mut self, block: usize) -> Result<(), SshError> {
        match Block::new(block) {
            Ok(block) => {
                self.block = block;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// The number the next packet will carry.
    #[must_use]
    pub const fn sequence(&self) -> u32 {
        self.sequence.get()
    }

    /// Frames `payload` into `out` and answers with the length of the
    /// packet. The padding is one call on `rng`, which is what D-121
    /// requires: nothing here draws randomness per byte.
    ///
    /// The bytes of a packet that did not fit are in `out` and are not a
    /// packet; a caller sends what this answers with and nothing else.
    ///
    /// # Errors
    ///
    /// [`SshError::PayloadLength`] above [`MAX_PAYLOAD`],
    /// [`SshError::OutOfBounds`] when `out` is too small, and
    /// [`SshError::Rng`] when the generator has nothing.
    pub fn encode(
        &mut self,
        payload: &[u8],
        rng: &mut impl Rng,
        out: &mut [u8],
    ) -> Result<usize, SshError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(SshError::PayloadLength(payload.len()));
        }
        let content = HEADER_BYTES.saturating_add(payload.len());
        let total = self.block.padded(content);
        let padding = total.saturating_sub(content);
        let length = u32::try_from(total.saturating_sub(4)).unwrap_or(u32::MAX);
        let count = u8::try_from(padding).unwrap_or(u8::MAX);
        let mut writer = Writer::new(out);
        writer.write_u32(length)?;
        writer.write_byte(count)?;
        writer.write_bytes(payload)?;
        rng.fill(writer.take(padding)?).map_err(SshError::Rng)?;
        self.sequence.advance();
        Ok(total)
    }
}

impl Default for Encoder {
    fn default() -> Encoder {
        Encoder::new()
    }
}

/// What a [`Decoder`] found at the front of the bytes it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decoded<'a> {
    /// Not a whole packet yet. `needed` is how many bytes the packet is
    /// altogether, which is four until the length field has arrived.
    Incomplete {
        /// Bytes the packet takes, counted from the first.
        needed: usize,
    },
    /// One packet, and nothing of what may follow it.
    Packet {
        /// The payload, without the header and the padding.
        payload: &'a [u8],
        /// Bytes the packet took, which is where the next one starts.
        length: usize,
    },
}

/// Reads incoming packets, and counts them.
#[derive(Clone, Debug)]
pub struct Decoder {
    /// What the packet length must be a multiple of.
    block: Block,
    /// The number of the next packet.
    sequence: SequenceNumber,
}

impl Decoder {
    /// A decoder for the connection before a cipher is negotiated, where
    /// the block size is [`MIN_BLOCK`].
    #[must_use]
    pub const fn new() -> Decoder {
        Decoder {
            block: Block(MIN_BLOCK),
            sequence: SequenceNumber::new(),
        }
    }

    /// Expects packets a multiple of `block` long from the next one on.
    /// As with [`Encoder::set_block`], the sequence number stands.
    ///
    /// # Errors
    ///
    /// [`SshError::BlockSize`] when `block` is below [`MIN_BLOCK`] or
    /// above [`MAX_BLOCK`]. The block in use does not change.
    pub const fn set_block(&mut self, block: usize) -> Result<(), SshError> {
        match Block::new(block) {
            Ok(block) => {
                self.block = block;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// The number the next packet will carry.
    #[must_use]
    pub const fn sequence(&self) -> u32 {
        self.sequence.get()
    }

    /// The packet at the front of `bytes`, or what is still missing of it.
    ///
    /// The length is judged as soon as the four bytes that hold it have
    /// arrived, and before the rest is waited for, so a length no packet
    /// has costs nothing to refuse.
    ///
    /// # Errors
    ///
    /// [`SshError::PacketLength`] for a length below [`MIN_PACKET`], above
    /// [`MAX_FRAME`], or not a whole number of blocks;
    /// [`SshError::PaddingLength`] for padding that is not inside the
    /// packet or is under [`MIN_PADDING`]; [`SshError::PayloadLength`] for
    /// a payload above [`MAX_PAYLOAD`].
    pub fn decode<'a>(&mut self, bytes: &'a [u8]) -> Result<Decoded<'a>, SshError> {
        let mut reader = Reader::new(bytes);
        let Ok(length) = reader.read_u32() else {
            return Ok(Decoded::Incomplete { needed: 4 });
        };
        let total = usize::try_from(length)
            .unwrap_or(usize::MAX)
            .saturating_add(4);
        if !(MIN_PACKET..=MAX_FRAME).contains(&total) || !self.block.divides(total) {
            return Err(SshError::PacketLength(length));
        }
        if bytes.len() < total {
            return Ok(Decoded::Incomplete { needed: total });
        }
        let padding = reader.read_byte()?;
        let content = total.saturating_sub(HEADER_BYTES);
        let payload_len = content
            .checked_sub(usize::from(padding))
            .ok_or(SshError::PaddingLength(padding))?;
        if usize::from(padding) < MIN_PADDING {
            return Err(SshError::PaddingLength(padding));
        }
        if payload_len > MAX_PAYLOAD {
            return Err(SshError::PayloadLength(payload_len));
        }
        let payload = reader.read_bytes(payload_len)?;
        self.sequence.advance();
        Ok(Decoded::Packet {
            payload,
            length: total,
        })
    }
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new()
    }
}
