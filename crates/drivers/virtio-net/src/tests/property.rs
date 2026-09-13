// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What holds for every frame, not only for the ones written by hand.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use test_support::generators::{bytes, pair, range};
use test_support::property::check;

use crate::net::HEADER_LEN;
use crate::queues::Side;

use super::support::{STRIDE, device, frames, header, live, queue};

#[test]
fn a_frame_of_any_length_that_fits_goes_out_whole_and_behind_a_zeroed_header() {
    let capacity = STRIDE as usize - HEADER_LEN;
    let cases = bytes(0..=64);
    check("net transmit", &cases, |frame| {
        let mut registers = device();
        let mut net = live(&mut registers);
        let (mut memory, mut queue) = queue();
        let mut area = frames(Side::Transmit);
        net.send(&mut queue, &mut memory, &mut area, frame)
            .map_err(|error| std::format!("{error}"))?;
        let head = memory.available_entry(0);
        let descriptor = memory
            .descriptor(head)
            .ok_or_else(|| "no descriptor".to_owned())?;
        if descriptor.length as usize != HEADER_LEN + frame.len() {
            return Err(std::format!("{} bytes went out", descriptor.length));
        }
        let bytes = area.buffer(0);
        if bytes[..HEADER_LEN] != [0u8; HEADER_LEN] {
            return Err("the header is not zero".to_owned());
        }
        if &bytes[HEADER_LEN..HEADER_LEN + frame.len()] != frame.as_slice() {
            return Err("the frame is not what was sent".to_owned());
        }
        if frame.len() > capacity {
            return Err("a frame past the buffer was taken".to_owned());
        }
        Ok(())
    });
}

#[test]
fn a_used_element_of_any_length_yields_that_frame_or_refuses_and_never_reads_further() {
    let cases = pair(bytes(0..=48), range(0..=64u32));
    check("net receive", &cases, |(frame, extra)| {
        let mut registers = device();
        let mut net = live(&mut registers);
        let (mut memory, mut queue) = queue();
        let mut area = frames(Side::Receive);
        net.fill(&mut queue, &mut memory, &area)
            .map_err(|error| std::format!("{error}"))?;
        let head = memory.available_entry(0);
        let written = area.deliver(0, &header(), frame);
        // The device may report fewer bytes than it wrote, and may report
        // more; what it may not do is make this driver read past the
        // buffer, so both are handed in.
        let reported = written.saturating_add(*extra).min(STRIDE);
        memory.complete(u32::from(head), reported);
        let mut into = [0u8; 64];
        match net.receive(&mut queue, &mut memory, &area, &mut into) {
            Ok(Some(taken)) => {
                let len = reported as usize - HEADER_LEN;
                if taken.len() != len {
                    return Err(std::format!("{} bytes where {len} were said", taken.len()));
                }
                let common = taken.len().min(frame.len());
                if taken[..common] != frame[..common] {
                    return Err("the bytes are not the ones delivered".to_owned());
                }
                Ok(())
            }
            Ok(None) => Err("the used element was not seen".to_owned()),
            // A refusal is right for a length the driver cannot hand on;
            // what matters is that the buffer went back either way.
            Err(_) if queue.free_count() == 0 => Ok(()),
            Err(error) => Err(std::format!("{error} left a descriptor free")),
        }
    });
}
