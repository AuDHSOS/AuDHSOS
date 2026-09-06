// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The address selection of RFC 6724: which of this host's addresses a
//! packet leaves with, and which of a name's addresses it goes to.
//!
//! The policy table of section 2.1 is written over IPv6 prefixes, with
//! IPv4 standing in it as the mapped range `::ffff:0:0/96`. This system
//! has no mapped addresses at all — D-69 makes the two families distinct
//! values everywhere and refuses the mapped form — so the table is read
//! with that one row standing for the whole IPv4 family rather than by
//! forming an address the rest of the stack would refuse. The numbers are
//! the document's: precedence 35, label 4.
//!
//! Of the rules, the ones this stack can decide are here and the ones it
//! cannot are not, and each omission is about a thing this system does
//! not have. There are no deprecated addresses, because nothing here
//! tracks a preferred lifetime apart from a valid one (rule 3 of both
//! lists); no home addresses, because there is no Mobile IPv6 (rule 4);
//! no temporary addresses, because there are no privacy extensions
//! (source rule 7); one interface, so preferring the outgoing one decides
//! nothing (source rules 5 and 5.5); and no tunnels, so every transport
//! is native (destination rule 7). What is left is enough to put a
//! dual-stack name's addresses in the order a host should try them.

use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Interface-local scope.
const SCOPE_INTERFACE: u8 = 0x1;

/// Link-local scope.
const SCOPE_LINK: u8 = 0x2;

/// Site-local scope.
const SCOPE_SITE: u8 = 0x5;

/// Global scope.
const SCOPE_GLOBAL: u8 = 0xE;

/// One row of the policy table: a prefix, its precedence, and its label.
struct Row {
    /// The first bytes of the prefix.
    prefix: [u8; 16],
    /// How many bits of it count.
    bits: u32,
    /// Higher is preferred as a destination.
    precedence: u8,
    /// A source and a destination of one label belong together.
    label: u8,
}

/// The default policy table of RFC 6724, section 2.1, less the row for
/// the IPv4 range, which [`policy`] answers directly.
const TABLE: [Row; 8] = [
    Row {
        prefix: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        bits: 128,
        precedence: 50,
        label: 0,
    },
    Row {
        prefix: [0x20, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 16,
        precedence: 30,
        label: 2,
    },
    Row {
        prefix: [0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 32,
        precedence: 5,
        label: 5,
    },
    Row {
        prefix: [0xFC, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 7,
        precedence: 3,
        label: 13,
    },
    Row {
        prefix: [0xFE, 0xC0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 10,
        precedence: 1,
        label: 11,
    },
    Row {
        prefix: [0x3F, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 16,
        precedence: 1,
        label: 12,
    },
    Row {
        prefix: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 96,
        precedence: 1,
        label: 3,
    },
    Row {
        prefix: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        bits: 0,
        precedence: 40,
        label: 1,
    },
];

/// The precedence and the label of `address`.
#[must_use]
pub fn policy(address: IpAddr) -> (u8, u8) {
    let IpAddr::V6(address) = address else {
        // The `::ffff:0:0/96` row of the table, read as the row of the
        // IPv4 family (D-69).
        return (35, 4);
    };
    let octets = address.octets();
    let mut best: Option<&Row> = None;
    for row in &TABLE {
        if !matches(&octets, row) {
            continue;
        }
        if best.is_none_or(|held| row.bits > held.bits) {
            best = Some(row);
        }
    }
    best.map_or((40, 1), |row| (row.precedence, row.label))
}

/// Whether `octets` begins with the prefix of `row`.
fn matches(octets: &[u8; 16], row: &Row) -> bool {
    common_bits(octets, &row.prefix) >= row.bits
}

/// How many leading bits `left` and `right` share.
fn common_bits(left: &[u8; 16], right: &[u8; 16]) -> u32 {
    let mut shared = 0u32;
    for (a, b) in left.iter().zip(right.iter()) {
        let same = (a ^ b).leading_zeros();
        shared = shared.saturating_add(same);
        if same != 8 {
            break;
        }
    }
    shared
}

/// The scope of `address`, as RFC 6724, section 3.1 has it.
#[must_use]
pub const fn scope(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(address) => scope_v4(address),
        IpAddr::V6(address) => scope_v6(address),
    }
}

/// The scope of an IPv4 address. RFC 6724, section 3.1 gives the loopback
/// and link-local ranges link-local scope and everything else, private
/// ranges included, global scope.
const fn scope_v4(address: Ipv4Addr) -> u8 {
    let [first, second, third, _] = address.octets();
    if first == 127 || (first == 169 && second == 254) {
        return SCOPE_LINK;
    }
    if first == 224 && second == 0 && third == 0 {
        return SCOPE_LINK;
    }
    if first == 239 {
        return SCOPE_SITE;
    }
    SCOPE_GLOBAL
}

/// The scope of an IPv6 address. A multicast address carries its own in
/// the low four bits of its second byte (RFC 4291, section 2.7).
const fn scope_v6(address: Ipv6Addr) -> u8 {
    let octets = address.octets();
    let [first, second, ..] = octets;
    if first == 0xFF {
        return second & 0x0F;
    }
    if address.is_loopback() {
        return SCOPE_INTERFACE;
    }
    if first == 0xFE && second & 0xC0 == 0x80 {
        return SCOPE_LINK;
    }
    if first == 0xFE && second & 0xC0 == 0xC0 {
        return SCOPE_SITE;
    }
    SCOPE_GLOBAL
}

/// How many leading bits two addresses of one family share, which is
/// destination rule 9 and source rule 8.
#[must_use]
pub fn common_prefix(left: IpAddr, right: IpAddr) -> u32 {
    match (left, right) {
        (IpAddr::V6(left), IpAddr::V6(right)) => common_bits(&left.octets(), &right.octets()),
        (IpAddr::V4(left), IpAddr::V4(right)) => {
            // Four bytes and no more: the twelve zeros behind them are
            // padding for the comparison and not bits two addresses share.
            common_bits(&widened(left), &widened(right)).min(32)
        }
        _ => 0,
    }
}

/// An IPv4 address in the sixteen bytes the bit comparison works over.
/// This is not the mapped form D-69 refuses; it is four bytes and twelve
/// zeros, used to count shared leading bits and never as an address.
const fn widened(address: Ipv4Addr) -> [u8; 16] {
    let [first, second, third, fourth] = address.octets();
    [
        first, second, third, fourth, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]
}

/// Which of `candidates` a packet to `destination` should leave from.
///
/// The rules applied are 1 (prefer the destination itself), 2 (prefer an
/// appropriate scope), 6 (prefer a matching label) and 8 (use the longest
/// matching prefix), in that order. A candidate of the other family is
/// never one.
#[must_use]
pub fn source_for(destination: IpAddr, candidates: &[IpAddr]) -> Option<IpAddr> {
    let target = scope(destination);
    let (_, wanted) = policy(destination);
    let mut best: Option<IpAddr> = None;
    for candidate in candidates {
        let candidate = *candidate;
        if candidate.version() != destination.version() {
            continue;
        }
        let Some(held) = best else {
            best = Some(candidate);
            continue;
        };
        if prefer_source(candidate, held, destination, target, wanted) {
            best = Some(candidate);
        }
    }
    best
}

/// Whether `candidate` is a better source than `held`.
fn prefer_source(
    candidate: IpAddr,
    held: IpAddr,
    destination: IpAddr,
    target: u8,
    wanted: u8,
) -> bool {
    // Rule 1: prefer the same address.
    if candidate == destination {
        return true;
    }
    if held == destination {
        return false;
    }
    // Rule 2: prefer an appropriate scope, which is the smallest that is
    // not smaller than the destination's.
    let (mine, theirs) = (scope(candidate), scope(held));
    if mine != theirs {
        return match (mine >= target, theirs >= target) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => mine < theirs,
            (false, false) => mine > theirs,
        };
    }
    // Rule 6: prefer a matching label.
    let (_, mine_label) = policy(candidate);
    let (_, theirs_label) = policy(held);
    if (mine_label == wanted) != (theirs_label == wanted) {
        return mine_label == wanted;
    }
    // Rule 8: use the longest matching prefix.
    common_prefix(candidate, destination) > common_prefix(held, destination)
}

/// Puts `addresses` in the order a host should try them, leaving the
/// order of two it cannot tell apart as it found it.
///
/// `source_of` says which of this host's addresses would carry a packet
/// to a given destination, or `None` when none would. The rules applied
/// are 1 (avoid unusable), 2 (prefer a matching scope), 5 (prefer a
/// matching label), 6 (prefer a higher precedence), 8 (prefer a smaller
/// scope), 9 (use the longest matching prefix) and 10 (leave the rest as
/// it was).
pub fn order_destinations(addresses: &mut [IpAddr], source_of: &dyn Fn(IpAddr) -> Option<IpAddr>) {
    // An insertion sort, because it is stable and because a name has a
    // handful of addresses.
    for at in 1..addresses.len() {
        let mut here = at;
        while here > 0 {
            let before = here.saturating_sub(1);
            let (Some(left), Some(right)) =
                (addresses.get(before).copied(), addresses.get(here).copied())
            else {
                break;
            };
            if !prefer_destination(right, left, source_of) {
                break;
            }
            addresses.swap(before, here);
            here = before;
        }
    }
}

/// Whether `candidate` should be tried before `held`.
fn prefer_destination(
    candidate: IpAddr,
    held: IpAddr,
    source_of: &dyn Fn(IpAddr) -> Option<IpAddr>,
) -> bool {
    let (mine, theirs) = (source_of(candidate), source_of(held));
    // Rule 1: avoid destinations nothing can leave for.
    let (Some(mine), Some(theirs)) = (mine, theirs) else {
        return mine.is_some();
    };
    // Rule 2: prefer a destination whose scope is its source's.
    let matching = |source: IpAddr, destination: IpAddr| scope(source) == scope(destination);
    let (a, b) = (matching(mine, candidate), matching(theirs, held));
    if a != b {
        return a;
    }
    // Rule 5: prefer a destination whose label is its source's.
    let (mine_prec, mine_label) = policy(candidate);
    let (theirs_prec, theirs_label) = policy(held);
    let (_, mine_source_label) = policy(mine);
    let (_, theirs_source_label) = policy(theirs);
    let (a, b) = (
        mine_label == mine_source_label,
        theirs_label == theirs_source_label,
    );
    if a != b {
        return a;
    }
    // Rule 6: prefer the higher precedence.
    if mine_prec != theirs_prec {
        return mine_prec > theirs_prec;
    }
    // Rule 8: prefer the smaller scope.
    let (a, b) = (scope(candidate), scope(held));
    if a != b {
        return a < b;
    }
    // Rule 9: use the longest matching prefix, which is a comparison
    // between two addresses of one family and says nothing across them.
    common_prefix(mine, candidate) > common_prefix(theirs, held)
}
