// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The walk over the bus: which functions are there and what their headers
//! say.
//!
//! The rule for what to read is the *PCI Express Base Specification* 6.0,
//! section 7.5.1.1.9: function zero of a device says whether the device has
//! others, and a device whose function zero is absent has none.
//!
//! Invariants: the walk is bounded by the bus range of the window and ends
//! after at most thirty-two devices of eight functions of each bus; a
//! function that is not there ends the walk of its device and not of the
//! bus; [`walk`] reports a bridge and does not descend into it.
//!
//! [`Buses`] names the buses a caller that maps one bus at a time walks:
//! the first bus of the window and every bus behind a bridge found on a
//! bus walked before (D-196). Each bus comes out at most once, so a walk
//! over B reachable buses costs O(B) bus mappings, not one per bus of the
//! window. A bus-numbering loop of bridges ends there.

use crate::address::{Address, MAX_DEVICE, MAX_FUNCTION, Window};
use crate::error::PciError;
use crate::header::{self, Header};
use crate::space::ConfigSpace;

/// One function the walk found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Function {
    /// Where it is.
    pub address: Address,
    /// What its header says.
    pub header: Header,
}

/// Reports every function of the window to `visit` and answers how many
/// there were.
///
/// # Errors
///
/// The errors of [`crate::header::read`].
pub fn walk<S, F>(space: &S, window: Window, mut visit: F) -> Result<usize, PciError>
where
    S: ConfigSpace,
    F: FnMut(&Function),
{
    let mut count = 0usize;
    for bus in window.first_bus()..=window.last_bus() {
        for device in 0..=MAX_DEVICE {
            let address = Address::new(window.segment(), bus, device, 0)?;
            let Some(header) = header::read(space, address)? else {
                continue;
            };
            let multi = header.is_multi_function();
            visit(&Function { address, header });
            count = count.saturating_add(1);
            if !multi {
                continue;
            }
            for function in 1..=MAX_FUNCTION {
                let Some(address) = address.with_function(function) else {
                    continue;
                };
                let Some(header) = header::read(space, address)? else {
                    continue;
                };
                visit(&Function { address, header });
                count = count.saturating_add(1);
            }
        }
    }
    Ok(count)
}

/// The first function of the window whose vendor and device identifiers
/// are the ones asked for.
///
/// # Errors
///
/// The errors of [`crate::header::read`].
pub fn find(
    space: &impl ConfigSpace,
    window: Window,
    vendor: u16,
    device: u16,
) -> Result<Option<Function>, PciError> {
    let mut found = None;
    walk(space, window, |function| {
        if found.is_none() && function.header.vendor == vendor && function.header.device == device {
            found = Some(*function);
        }
    })?;
    Ok(found)
}

/// The buses a walk one bus at a time reaches, lowest first.
///
/// The first bus of the window is reached from the start; [`Buses::reach`]
/// adds a bus inside the window. A bus [`Buses::next_bus`] gave out is not
/// given out again.
#[derive(Clone, Copy, Debug)]
pub struct Buses {
    /// The range a bus must lie in.
    window: Window,
    /// One bit per bus reached.
    reached: [u64; 4],
    /// One bit per bus given out.
    taken: [u64; 4],
}

impl Buses {
    /// The walk of `window`, holding its first bus.
    #[must_use]
    pub fn new(window: Window) -> Buses {
        let mut buses = Buses {
            window,
            reached: [0; 4],
            taken: [0; 4],
        };
        buses.reach(window.first_bus());
        buses
    }

    /// The lowest bus reached and not given out, or `None` when there is
    /// none.
    pub fn next_bus(&mut self) -> Option<u8> {
        for (index, (reached, taken)) in self.reached.iter().zip(self.taken.iter_mut()).enumerate()
        {
            let open = *reached & !*taken;
            if open != 0 {
                let bit = open.trailing_zeros();
                *taken |= 1u64.wrapping_shl(bit);
                let base = u32::try_from(index).unwrap_or(0).wrapping_mul(64);
                return u8::try_from(base.wrapping_add(bit)).ok();
            }
        }
        None
    }

    /// Adds `bus` when it lies in the window.
    pub fn reach(&mut self, bus: u8) {
        if bus < self.window.first_bus() || bus > self.window.last_bus() {
            return;
        }
        let word = usize::from(bus.wrapping_shr(6));
        let bit = 1u64.wrapping_shl(u32::from(bus & 63));
        for (index, reached) in self.reached.iter_mut().enumerate() {
            if index == word {
                *reached |= bit;
            }
        }
    }

    /// Adds the secondary bus of `function` when it is a bridge.
    pub fn reach_behind(&mut self, function: &Function) {
        if let Some(bus) = function.header.secondary_bus {
            self.reach(bus);
        }
    }
}
