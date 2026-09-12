// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What holds for every request, not only for the ones written by hand.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use test_support::generators::{one_of, pair, range};
use test_support::property::check;

use crate::blk::{Chain, Segment};
use crate::config::SECTOR_LEN;
use crate::request::{HEADER_LEN, Kind, Request};

use super::support::{CAPACITY, device, live, queue};

#[test]
fn a_header_written_for_any_sector_carries_that_sector_and_that_type() {
    let kinds = one_of(vec![Kind::In, Kind::Out, Kind::Flush]);
    let cases = pair(kinds, range(0..=u64::MAX));
    check("blk header", &cases, |&(kind, sector)| {
        let sector = if kind == Kind::Flush { 0 } else { sector };
        let request = Request { kind, sector };
        let mut bytes = [0xFFu8; 32];
        request
            .write_header(&mut bytes)
            .map_err(|error| format!("{error}"))?;
        let written = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([0; 4]));
        if written != kind.code() {
            return Err(format!("{written} is not the code of {kind:?}"));
        }
        let reserved = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
        if reserved != 0 {
            return Err(format!("the reserved word is {reserved}"));
        }
        let carried = u64::from_le_bytes(bytes[8..16].try_into().unwrap_or([0; 8]));
        if carried != sector {
            return Err(format!("{carried} is not {sector}"));
        }
        let len = usize::try_from(HEADER_LEN).unwrap_or(0);
        if bytes[len] != 0xFF {
            return Err("the header reached past its length".to_owned());
        }
        Ok(())
    });
}

#[test]
fn a_request_is_taken_exactly_when_every_rule_of_the_framing_holds() {
    let kinds = one_of(vec![Kind::In, Kind::Out, Kind::Flush]);
    let cases = pair(pair(kinds, range(0..=CAPACITY + 4)), range(0..=4u32));
    check("blk framing", &cases, |&((kind, sector), sectors)| {
        let mut registers = device();
        let blk = live(&mut registers);
        let (mut memory, mut queue) = queue();
        let length = sectors * SECTOR_LEN;
        let data = (kind != Kind::Flush).then_some(Segment {
            address: 0x2_1000,
            length,
        });
        let chain = Chain {
            header: 0x2_0000,
            data,
            status: 0x2_2000,
        };
        let request = Request {
            kind,
            sector: if kind == Kind::Flush { 0 } else { sector },
        };
        // What the specification allows, said again here and not taken
        // from the driver: whole sectors, at least one, and inside the
        // device.
        let allowed = match kind {
            Kind::Flush => true,
            Kind::In | Kind::Out => sectors > 0 && sector + u64::from(sectors) <= CAPACITY,
        };
        match (
            allowed,
            blk.submit(&mut queue, &mut memory, &request, &chain),
        ) {
            (true, Ok(_)) | (false, Err(_)) => Ok(()),
            (true, Err(error)) => Err(format!("{kind:?} of {sectors} at {sector}: {error}")),
            (false, Ok(head)) => Err(format!(
                "{kind:?} of {sectors} at {sector} was taken as {head}"
            )),
        }
    });
}
