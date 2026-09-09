// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
extern crate alloc;
use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};

/// Registration options after host-language conversion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Receive the capturing phase instead of the bubbling phase.
    pub capture: bool,
    /// Remove before the first invocation.
    pub once: bool,
    /// Prevent cancellation during this invocation.
    pub passive: bool,
}
/// A live listener registration. Its ID is unique within the registry.
#[derive(Clone, Debug)]
pub struct Listener<T> {
    /// Registration identity, never reused.
    pub id: u64,
    /// UTF-16 event type, compared exactly.
    pub event_type: Rc<[u16]>,
    /// Embedding-defined callback identity/value.
    pub callback: T,
    /// Converted options.
    pub options: Options,
}
/// Logical resource exhaustion, including ID overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit;
/// Listener storage. The embedding must trace callbacks returned by `iter`.
pub struct Listeners<T> {
    entries: BTreeMap<u64, Listener<T>>,
    next: u64,
}
impl<T> Default for Listeners<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> Listeners<T> {
    /// Creates an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next: 0,
        }
    }
    /// Live callback roots in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Listener<T>> {
        self.entries.values()
    }
    /// Number of live registrations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// Whether no listeners remain.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Next registration identity; unchanged by duplicate additions.
    #[must_use]
    pub const fn next_id(&self) -> u64 {
        self.next
    }
    /// Removes a specific registration without matching a later replacement.
    pub fn remove_id(&mut self, id: u64) -> bool {
        self.entries.remove(&id).is_some()
    }
    /// Snapshots matching IDs for a phase, never callback copies.
    #[must_use]
    pub fn snapshot(&self, event_type: &[u16], capture: bool) -> Vec<u64> {
        self.entries
            .values()
            .filter(|l| l.event_type.as_ref() == event_type && l.options.capture == capture)
            .map(|l| l.id)
            .collect()
    }
}
impl<T: PartialEq + Clone> Listeners<T> {
    /// Adds a listener, leaving earlier options intact on duplicate registration.
    ///
    /// # Errors
    /// Returns `Limit` before mutation if capacity/type length or IDs run out.
    pub fn add(
        &mut self,
        event_type: Rc<[u16]>,
        callback: T,
        options: Options,
        capacity: usize,
        type_units: usize,
    ) -> Result<bool, Limit> {
        if event_type.len() > type_units {
            return Err(Limit);
        }
        if self.entries.values().any(|l| {
            l.event_type == event_type
                && l.callback == callback
                && l.options.capture == options.capture
        }) {
            return Ok(false);
        }
        if self.entries.len() >= capacity {
            return Err(Limit);
        }
        let next = self.next.checked_add(1).ok_or(Limit)?;
        self.entries.insert(
            self.next,
            Listener {
                id: self.next,
                event_type,
                callback,
                options,
            },
        );
        self.next = next;
        Ok(true)
    }
    /// Removes the exact type/callback/capture registration.
    pub fn remove(&mut self, event_type: &[u16], callback: &T, capture: bool) -> bool {
        let id = self
            .entries
            .values()
            .find(|l| {
                l.event_type.as_ref() == event_type
                    && l.callback == *callback
                    && l.options.capture == capture
            })
            .map(|l| l.id);
        id.is_some_and(|id| self.entries.remove(&id).is_some())
    }
    /// Returns a still-live snapshot entry; removes a once entry before returning.
    pub fn take_for_invoke(&mut self, id: u64) -> Option<Listener<T>> {
        let listener = self.entries.get(&id)?.clone();
        if listener.options.once {
            self.entries.remove(&id);
        }
        Some(listener)
    }
}

/// Event state independent of targets and host-language properties.
#[derive(Clone, Debug)]
pub struct Event {
    event_type: Rc<[u16]>,
    flags: u8,
}
const BUBBLES: u8 = 1;
const CANCELABLE: u8 = 2;
const COMPOSED: u8 = 4;
const CANCELED: u8 = 8;
const STOP: u8 = 16;
const IMMEDIATE: u8 = 32;
const PASSIVE: u8 = 64;
const DISPATCH: u8 = 128;
impl Event {
    /// Creates an initialized, untrusted event.
    #[must_use]
    pub const fn new(
        event_type: Rc<[u16]>,
        bubbles: bool,
        cancelable: bool,
        composed: bool,
    ) -> Self {
        Self {
            event_type,
            flags: (if bubbles { BUBBLES } else { 0 })
                | (if cancelable { CANCELABLE } else { 0 })
                | (if composed { COMPOSED } else { 0 }),
        }
    }
    /// Event type, preserving every UTF-16 code unit.
    #[must_use]
    pub fn event_type(&self) -> Rc<[u16]> {
        self.event_type.clone()
    }
    /// Whether bubbling is requested.
    #[must_use]
    pub const fn bubbles(&self) -> bool {
        self.flags & BUBBLES != 0
    }
    /// Whether cancellation is allowed.
    #[must_use]
    pub const fn cancelable(&self) -> bool {
        self.flags & CANCELABLE != 0
    }
    /// Whether the event crosses shadow boundaries when supported by the host.
    #[must_use]
    pub const fn composed(&self) -> bool {
        self.flags & COMPOSED != 0
    }
    /// Whether cancellation has occurred.
    #[must_use]
    pub const fn canceled(&self) -> bool {
        self.flags & CANCELED != 0
    }
    /// Whether propagation to a subsequent phase/target is stopped.
    #[must_use]
    pub const fn stopped(&self) -> bool {
        self.flags & STOP != 0
    }
    /// Whether invocation in the current phase is stopped.
    #[must_use]
    pub const fn immediate(&self) -> bool {
        self.flags & IMMEDIATE != 0
    }
    /// Whether the event is already being dispatched.
    #[must_use]
    pub const fn dispatching(&self) -> bool {
        self.flags & DISPATCH != 0
    }
    /// Begins dispatch; false rejects reentrant use of the same event.
    pub const fn begin(&mut self) -> bool {
        if self.dispatching() {
            false
        } else {
            self.flags |= DISPATCH;
            true
        }
    }
    /// Ends dispatch, clearing transient flags but preserving cancellation.
    pub const fn finish(&mut self) {
        self.flags &= !(DISPATCH | STOP | IMMEDIATE | PASSIVE);
    }
    /// Sets the passive state for the currently invoked listener.
    pub const fn passive(&mut self, value: bool) {
        if value {
            self.flags |= PASSIVE;
        } else {
            self.flags &= !PASSIVE;
        }
    }
    /// Cancels only if cancelable and outside a passive listener.
    pub const fn prevent_default(&mut self) {
        if self.cancelable() && self.flags & PASSIVE == 0 {
            self.flags |= CANCELED;
        }
    }
    /// Stops propagation; immediate also stops other listeners in this phase.
    pub const fn stop(&mut self, immediate: bool) {
        self.flags |= STOP;
        if immediate {
            self.flags |= IMMEDIATE;
        }
    }
    /// Legacy reinitialization, ignored during dispatch; composed is unchanged.
    pub fn initialize(&mut self, event_type: Rc<[u16]>, bubbles: bool, cancelable: bool) {
        if !self.dispatching() {
            let composed = self.composed();
            *self = Self::new(event_type, bubbles, cancelable, composed);
        }
    }
}

#[cfg(test)]
mod tests;
