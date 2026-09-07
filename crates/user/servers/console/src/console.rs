// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bytes out on the line, and the bytes that came in on it.
//!
//! Sending is direct: the controller is asked to take each byte, and it
//! either takes it or the transmitter never becomes ready, which is the one
//! failure a write has. Receiving is a ring, because the bytes arrive when
//! they arrive and a client asks for them when it asks.
//!
//! The ring drops the oldest byte when it is full and counts what it
//! dropped. The other choice is to drop the newest, and it is worse for a
//! console: what a person is typing now is what they will look for on the
//! screen, and a count of what was lost is something a client can act on
//! while a silently missing byte is not.
//!
//! Invariants: what `read` hands out is what `receive` was given, in that
//! order and once each; a write reports how many bytes reached the
//! controller, which is every byte or the ones before the first refusal.

use audhsos_collections::RingBuffer;
use driver_uart16550::{Registers, Uart16550, UartError};

/// How many bytes the driver keeps for a client that has not asked yet.
///
/// A line of terminal input is far shorter, and a client that has fallen
/// this far behind has stopped reading rather than fallen behind.
pub const RECEIVE_CAPACITY: usize = 256;

/// The console driver over one controller.
#[derive(Debug)]
pub struct Console<R: Registers> {
    uart: Uart16550<R>,
    received: RingBuffer<u8, RECEIVE_CAPACITY>,
    dropped: u64,
}

impl<R: Registers> Console<R> {
    /// Takes the controller over: initializes it and turns the receive
    /// interrupt on, which is what makes the interrupt thread wake.
    pub fn new(registers: R) -> Self {
        let mut uart = Uart16550::init(registers);
        uart.enable_receive_interrupt();
        Console {
            uart,
            received: RingBuffer::new(),
            dropped: 0,
        }
    }

    /// How many bytes are waiting for a client.
    #[must_use]
    pub const fn waiting(&self) -> usize {
        self.received.len()
    }

    /// How many bytes were dropped because nobody read them in time.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Puts `bytes` on the line and answers with how many went out.
    ///
    /// A byte the controller would not take ends the write; the count says
    /// how far it came, and the client decides whether to try the rest.
    ///
    /// # Errors
    ///
    /// [`UartError::Timeout`] when the very first byte found the
    /// transmitter busy, so that a write which moved nothing is an error
    /// and not a count of zero.
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, UartError> {
        let mut written = 0usize;
        for byte in bytes {
            if self.uart.write_byte(*byte).is_err() {
                break;
            }
            written = written.wrapping_add(1);
        }
        if written == 0 && !bytes.is_empty() {
            return Err(UartError::Timeout);
        }
        Ok(written)
    }

    /// Takes the byte the controller has, for the thread the interrupt
    /// woke, and acknowledges nothing: the interrupt object is the
    /// binary's business.
    ///
    /// Answers `None` when the interrupt was not a byte arriving, which
    /// happens: the controller raises one interrupt for several reasons.
    pub fn take_from_controller(&mut self) -> Option<u8> {
        self.uart.read_byte().ok()
    }

    /// Puts a byte that arrived into the ring.
    ///
    /// Answers `true` when it fitted and `false` when the oldest byte had
    /// to go to make room.
    pub fn receive(&mut self, byte: u8) -> bool {
        if self.received.is_full() {
            let _oldest = self.received.pop();
            self.dropped = self.dropped.wrapping_add(1);
            let _pushed = self.received.push(byte);
            return false;
        }
        let _pushed = self.received.push(byte);
        true
    }

    /// Takes at most `max` bytes out of the ring into `into` and answers
    /// with how many. Zero when nothing has arrived, which is an answer and
    /// not a failure: a client that wants to wait asks again.
    pub fn read(&mut self, max: usize, into: &mut [u8]) -> usize {
        let bound = max.min(into.len()).min(self.received.len());
        let ring = &mut self.received;
        let mut taken = 0usize;
        for (slot, byte) in into
            .iter_mut()
            .zip(core::iter::from_fn(|| ring.pop()))
            .take(bound)
        {
            *slot = byte;
            taken = taken.wrapping_add(1);
        }
        taken
    }

    /// The controller, for a caller that has to reach it directly.
    pub const fn uart(&mut self) -> &mut Uart16550<R> {
        &mut self.uart
    }
}
