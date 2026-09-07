// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the input server decides: who is subscribed, which decoder a byte
//! goes to, and what is appended to whose ring.
//!
//! One byte from the controller belongs to the keyboard or to the mouse,
//! and the `AUX` bit that came with it says which. It goes to that decoder,
//! and what the decoder makes of it goes to every subscriber: a ring is not
//! addressed to anybody, everyone who listens hears everything.
//!
//! A subscriber that cannot be woken is gone. That is how a client that has
//! ended is noticed, and it is why this server needs no watch of its own:
//! the first event after the end of a client fails to reach it and the
//! subscription goes with it.
//!
//! Invariants: one subscription per badge, and no badge is [`NOBODY`]; a
//! slot belongs to exactly one subscriber while that subscriber is
//! subscribed; a ring that cannot be written to costs the event and never
//! the subscription — only a wake-up that fails does that.

use audhsos_abi::Error;
use audhsos_collections::ArrayVec;
use driver_i8042::keyboard::Decoder as KeyDecoder;
use driver_i8042::mouse::{Decoder as MouseDecoder, PointerEvent};
use user_proto::input::{Event, RingWriter};

/// The badge of a capability that carries none.
///
/// A client reaches this server through a capability its parent badged, and
/// a subscription belongs to a badge. A request that arrives without one
/// names nobody — that is what a capability found under a name looks like —
/// and two of nobody are one client to a server that keeps a ring per
/// client.
pub const NOBODY: u64 = 0;

/// How many events one byte can produce: a mouse packet that carries both a
/// movement and a turn of the wheel is two.
pub const MAX_PRODUCED: usize = 2;

/// One client that listens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subscriber {
    /// The badge of the capability its messages arrive through.
    pub badge: u64,
    /// Which of the server's ring slots it holds.
    pub slot: usize,
}

/// What the process around this crate holds for each subscriber.
pub trait Clients {
    /// The bytes of the ring of the subscriber in `slot`, or `None` when
    /// the process holds none.
    fn ring(&mut self, slot: usize) -> Option<&mut [u8]>;

    /// Wakes the subscriber in `slot`, and answers whether it could be
    /// woken. A client that has ended cannot, which is how its end is
    /// noticed.
    fn wake(&mut self, slot: usize) -> bool;
}

/// What one byte produced.
pub type Produced = ArrayVec<Event, MAX_PRODUCED>;

/// Who listens, and what the two devices are saying.
#[derive(Debug)]
pub struct Input<const N: usize> {
    /// The clients that subscribed, in the order they did.
    subscribers: ArrayVec<Subscriber, N>,
    /// The state machine of scancode set 2.
    keyboard: KeyDecoder,
    /// The state machine of the mouse packet.
    mouse: MouseDecoder,
}

impl<const N: usize> Default for Input<N> {
    fn default() -> Self {
        Input::new(0)
    }
}

impl<const N: usize> Input<N> {
    /// A server nobody listens to, for a mouse that answered `mouse_id` to
    /// `GET_ID`: the identifier decides how long a packet is.
    #[must_use]
    pub const fn new(mouse_id: u8) -> Self {
        Input {
            subscribers: ArrayVec::new(),
            keyboard: KeyDecoder::new(),
            mouse: MouseDecoder::for_id(mouse_id),
        }
    }

    /// How many clients listen.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.subscribers.len()
    }

    /// `true` when nobody listens.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.subscribers.is_empty()
    }

    /// The clients that listen, in the order they subscribed.
    pub fn iter(&self) -> impl Iterator<Item = &Subscriber> {
        self.subscribers.iter()
    }

    /// The subscription `badge` holds, if it holds one.
    #[must_use]
    pub fn of(&self, badge: u64) -> Option<&Subscriber> {
        self.subscribers.iter().find(|held| held.badge == badge)
    }

    /// Takes `badge` on and answers with the ring slot it now holds.
    ///
    /// # Errors
    ///
    /// [`Error::AccessDenied`] for a request that carries no badge;
    /// [`Error::AlreadyExists`] when the client is subscribed already;
    /// [`Error::QuotaExceeded`] when the server holds as many rings as it
    /// can.
    pub fn subscribe(&mut self, badge: u64) -> Result<usize, Error> {
        if badge == NOBODY {
            return Err(Error::AccessDenied);
        }
        if self.of(badge).is_some() {
            return Err(Error::AlreadyExists);
        }
        let slot = self.free_slot().ok_or(Error::QuotaExceeded)?;
        self.subscribers
            .push(Subscriber { badge, slot })
            .map_err(|_| Error::QuotaExceeded)?;
        Ok(slot)
    }

    /// Lets `badge` go and answers with the ring slot that is free again.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a client that is not subscribed, which a
    /// request without a badge also is.
    pub fn unsubscribe(&mut self, badge: u64) -> Result<usize, Error> {
        let slot = self.of(badge).ok_or(Error::NotFound)?.slot;
        self.forget(badge);
        Ok(slot)
    }

    /// Forgets what `badge` held, which is what happens when it goes away.
    pub fn forget(&mut self, badge: u64) -> Option<Subscriber> {
        let index = self
            .subscribers
            .iter()
            .position(|held| held.badge == badge)?;
        self.subscribers.remove(index)
    }

    /// Takes one byte of the controller and answers with what it completed.
    ///
    /// `aux` is the bit the status register carried with the byte: it says
    /// the byte came from the mouse, and it is the only thing that does.
    /// A packet that carries both a movement and a turn of the wheel makes
    /// two events, so that a client that only reads the wheel does not have
    /// to look at the movement to find it.
    pub fn feed(&mut self, byte: u8, aux: bool) -> Produced {
        let mut produced = Produced::new();
        if aux {
            if let Some(pointer) = self.mouse.feed(byte) {
                if pointer.wheel != 0 {
                    let _fitted = produced.push(Event::Pointer(PointerEvent {
                        wheel: 0,
                        ..pointer
                    }));
                    let _fitted = produced.push(Event::Pointer(PointerEvent {
                        dx: 0,
                        dy: 0,
                        ..pointer
                    }));
                } else {
                    let _fitted = produced.push(Event::Pointer(pointer));
                }
            }
            return produced;
        }
        if let Some(key) = self.keyboard.feed(byte) {
            let _fitted = produced.push(Event::Key(key));
        }
        produced
    }

    /// Appends `event` to every subscriber's ring and wakes each of them,
    /// and answers with those that could not be woken and are therefore
    /// gone.
    ///
    /// The caller unmaps the ring of everyone in that list and closes what
    /// it held of them; here they are simply no longer subscribed.
    pub fn deliver<C: Clients>(
        &mut self,
        event: Event,
        clients: &mut C,
    ) -> ArrayVec<Subscriber, N> {
        let mut gone: ArrayVec<Subscriber, N> = ArrayVec::new();
        for subscriber in &self.subscribers {
            if let Some(bytes) = clients.ring(subscriber.slot)
                && let Some(mut ring) = RingWriter::adopt(bytes)
            {
                // A ring that is full drops the event and counts it; that
                // is the client's business and not the server's.
                let _fitted = ring.push(event);
            }
            if !clients.wake(subscriber.slot) {
                let _fitted = gone.push(*subscriber);
            }
        }
        for subscriber in &gone {
            self.forget(subscriber.badge);
        }
        gone
    }

    /// The lowest ring slot nobody holds.
    fn free_slot(&self) -> Option<usize> {
        (0..N).find(|slot| {
            !self
                .subscribers
                .iter()
                .any(|subscriber| subscriber.slot == *slot)
        })
    }
}
