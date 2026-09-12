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
//! bus; a bridge is reported and not descended into (D-112).

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
