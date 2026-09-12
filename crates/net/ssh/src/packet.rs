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

use crate::cipher::{self, ChaChaPoly};
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

/// The length field on its own.
const LENGTH_BYTES: usize = 4;

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
impl Encoder {
    /// An encoder whose next packet carries `sequence`.
    pub(crate) const fn at(sequence: u32) -> Encoder {
        Encoder {
            block: Block(MIN_BLOCK),
            sequence: SequenceNumber::at(sequence),
            cipher: None,
        }
    }
}

#[cfg(test)]
impl Decoder {
    /// A decoder whose next packet carries `sequence`.
    pub(crate) const fn at(sequence: u32) -> Decoder {
        Decoder {
            block: Block(MIN_BLOCK),
            sequence: SequenceNumber::at(sequence),
            cipher: None,
        }
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

    /// The block itself.
    const fn size(self) -> usize {
        self.0
    }
}

/// Frames outgoing packets, and counts them.
pub struct Encoder {
    /// What the packet length is padded to.
    block: Block,
    /// The number of the next packet.
    sequence: SequenceNumber,
    /// The cipher, once one has been negotiated and taken into use.
    cipher: Option<ChaChaPoly>,
}

impl Encoder {
    /// An encoder for the connection before a cipher is negotiated, where
    /// the block size is [`MIN_BLOCK`].
    #[must_use]
    pub const fn new() -> Encoder {
        Encoder {
            block: Block(MIN_BLOCK),
            sequence: SequenceNumber::new(),
            cipher: None,
        }
    }

    /// Encrypts from the next packet on, which is what
    /// `SSH_MSG_NEWKEYS` decides. The sequence number is untouched here
    /// too, and the block size the cipher pads to is its own.
    pub fn set_cipher(&mut self, key: &[u8; cipher::KEY_BYTES]) {
        self.block = Block(cipher::BLOCK);
        self.cipher = Some(ChaChaPoly::new(key));
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
        // What the padding aligns differs with the cipher: this one
        // encrypts the length field on its own and leaves it outside the
        // aligned region, which is what appendix A of its draft shows —
        // a packet of 76 bytes whose length field names 72.
        let sealed = self.cipher.is_some();
        let content = Encoder::header(sealed).saturating_add(payload.len());
        let aligned = self.block.padded(content);
        let padding = aligned.saturating_sub(content);
        let length = if sealed {
            aligned
        } else {
            aligned.saturating_sub(LENGTH_BYTES)
        };
        let frame = length.saturating_add(LENGTH_BYTES);
        let count = u8::try_from(padding).unwrap_or(u8::MAX);
        let available = out.len();
        {
            let mut writer = Writer::new(out);
            writer.write_u32(u32::try_from(length).unwrap_or(u32::MAX))?;
            writer.write_byte(count)?;
            writer.write_bytes(payload)?;
            rng.fill(writer.take(padding)?).map_err(SshError::Rng)?;
        }
        let Some(cipher) = self.cipher.as_ref() else {
            self.sequence.advance();
            return Ok(frame);
        };
        let mut tag = [0u8; cipher::TAG_BYTES];
        let total = frame.saturating_add(cipher::TAG_BYTES);
        let whole = out.get_mut(..total).ok_or(SshError::OutOfBounds {
            needed: total,
            available,
        })?;
        let (body, slot) = whole.split_at_mut(frame);
        cipher.seal(self.sequence.get(), body, &mut tag)?;
        slot.copy_from_slice(&tag);
        self.sequence.advance();
        Ok(total)
    }
}

impl Encoder {
    /// How much of the header the aligned region holds.
    const fn header(sealed: bool) -> usize {
        if sealed {
            HEADER_BYTES.saturating_sub(LENGTH_BYTES)
        } else {
            HEADER_BYTES
        }
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
pub struct Decoder {
    /// What the packet length must be a multiple of.
    block: Block,
    /// The number of the next packet.
    sequence: SequenceNumber,
    /// The cipher, once one has been negotiated and taken into use.
    cipher: Option<ChaChaPoly>,
}

impl Decoder {
    /// A decoder for the connection before a cipher is negotiated, where
    /// the block size is [`MIN_BLOCK`].
    #[must_use]
    pub const fn new() -> Decoder {
        Decoder {
            block: Block(MIN_BLOCK),
            sequence: SequenceNumber::new(),
            cipher: None,
        }
    }

    /// Decrypts from the next packet on. As with [`Encoder::set_cipher`],
    /// the sequence number stands.
    pub fn set_cipher(&mut self, key: &[u8; cipher::KEY_BYTES]) {
        self.block = Block(cipher::BLOCK);
        self.cipher = Some(ChaChaPoly::new(key));
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
    /// Once a cipher is in use this decrypts `bytes` in place and the
    /// payload it answers with borrows them, so what the caller holds
    /// after a packet is read is that packet in the clear. A second call
    /// over the same bytes reads what has already been decrypted and is
    /// not the same packet again.
    ///
    /// # Errors
    ///
    /// [`SshError::PacketLength`] for a length below [`MIN_PACKET`], above
    /// [`MAX_FRAME`], or not a whole number of blocks;
    /// [`SshError::PaddingLength`] for padding that is not inside the
    /// packet or is under [`MIN_PADDING`]; [`SshError::PayloadLength`] for
    /// a payload above [`MAX_PAYLOAD`].
    pub fn decode<'a>(&mut self, bytes: &'a mut [u8]) -> Result<Decoded<'a>, SshError> {
        let Some(head) = bytes.first_chunk::<LENGTH_BYTES>() else {
            return Ok(Decoded::Incomplete {
                needed: LENGTH_BYTES,
            });
        };
        let length = match self.cipher.as_ref() {
            Some(cipher) => cipher.length(self.sequence.get(), head),
            None => u32::from_be_bytes(*head),
        };
        let frame = usize::try_from(length)
            .unwrap_or(usize::MAX)
            .saturating_add(LENGTH_BYTES);
        self.check(length, frame)?;
        let tag_len = self.cipher.as_ref().map_or(0, |_| cipher::TAG_BYTES);
        let total = frame.saturating_add(tag_len);
        if bytes.len() < total {
            return Ok(Decoded::Incomplete { needed: total });
        }
        if let Some(cipher) = self.cipher.as_ref() {
            // The tag is over the ciphertext, so it is checked before a
            // byte is decrypted, which is what the draft requires of a
            // receiver.
            let (body, rest) = bytes.split_at_mut(frame);
            let tag = rest.get(..tag_len).unwrap_or(&[]);
            cipher.open(self.sequence.get(), body, tag)?;
        }
        let payload = Decoder::payload(bytes.get(..frame).unwrap_or(&[]))?;
        self.sequence.advance();
        Ok(Decoded::Packet {
            payload,
            length: total,
        })
    }

    /// What a length field may name: a frame the block divides, inside
    /// the bounds of section 6.1, and no shorter than a packet is.
    ///
    /// The floor differs with the cipher. Without one the frame holds the
    /// length field and section 6 asks for sixteen bytes altogether; with
    /// this cipher the length field is outside the aligned region, so
    /// what the document's floor can ask for is one block, and a packet
    /// of one block is what an OpenSSH sends for a one-byte payload.
    const fn check(&self, length: u32, frame: usize) -> Result<(), SshError> {
        let floor = if self.cipher.is_some() {
            self.block.size().saturating_add(LENGTH_BYTES)
        } else {
            MIN_PACKET
        };
        let aligned = if self.cipher.is_some() {
            frame.saturating_sub(LENGTH_BYTES)
        } else {
            frame
        };
        if frame < floor || frame > MAX_FRAME || !self.block.divides(aligned) {
            return Err(SshError::PacketLength(length));
        }
        Ok(())
    }

    /// The payload of a frame whose length is already judged.
    fn payload(frame: &[u8]) -> Result<&[u8], SshError> {
        let mut reader = Reader::new(frame);
        reader.read_bytes(LENGTH_BYTES)?;
        let padding = reader.read_byte()?;
        let content = frame.len().saturating_sub(HEADER_BYTES);
        let payload_len = content
            .checked_sub(usize::from(padding))
            .ok_or(SshError::PaddingLength(padding))?;
        if usize::from(padding) < MIN_PADDING {
            return Err(SshError::PaddingLength(padding));
        }
        if payload_len > MAX_PAYLOAD {
            return Err(SshError::PayloadLength(payload_len));
        }
        reader.read_bytes(payload_len)
    }
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new()
    }
}
