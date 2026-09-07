// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The numbers of RFC 1951, transcribed.
//!
//! Every table here is from `docs/rfc/rfc1951.txt`, section 3.2.5 for the
//! lengths and the distances and section 3.2.6 for the code lengths of the
//! fixed alphabet. None of them can be derived from anything else, which
//! is why they are written out rather than computed.

/// The first code of the literal alphabet that means a length.
pub(crate) const FIRST_LENGTH: u16 = 257;

/// The symbol that ends a block.
pub(crate) const END_OF_BLOCK: u16 = 256;

/// How many symbols the literal and length alphabet has, counting the two
/// that never occur but take part in building the code.
pub(crate) const LITERALS: usize = 288;

/// How many symbols the distance alphabet has, counting the two that
/// never occur.
pub(crate) const DISTANCES: usize = 32;

/// The shortest run this crate looks for.
pub(crate) const MIN_MATCH: usize = 3;

/// The longest run the format can name.
pub(crate) const MAX_MATCH: usize = 258;

/// How far back a run may reach.
pub const WINDOW: usize = 32768;

/// The lengths of section 3.2.5: the code, how many extra bits follow it,
/// and the shortest length that code stands for.
pub(crate) const LENGTHS: [(u16, u8, u16); 29] = [
    (257, 0, 3),
    (258, 0, 4),
    (259, 0, 5),
    (260, 0, 6),
    (261, 0, 7),
    (262, 0, 8),
    (263, 0, 9),
    (264, 0, 10),
    (265, 1, 11),
    (266, 1, 13),
    (267, 1, 15),
    (268, 1, 17),
    (269, 2, 19),
    (270, 2, 23),
    (271, 2, 27),
    (272, 2, 31),
    (273, 3, 35),
    (274, 3, 43),
    (275, 3, 51),
    (276, 3, 59),
    (277, 4, 67),
    (278, 4, 83),
    (279, 4, 99),
    (280, 4, 115),
    (281, 5, 131),
    (282, 5, 163),
    (283, 5, 195),
    (284, 5, 227),
    (285, 0, 258),
];

/// The distances of section 3.2.5: the code, how many extra bits follow
/// it, and the shortest distance that code stands for.
pub(crate) const DISTANCE_CODES: [(u16, u8, u16); 30] = [
    (0, 0, 1),
    (1, 0, 2),
    (2, 0, 3),
    (3, 0, 4),
    (4, 1, 5),
    (5, 1, 7),
    (6, 2, 9),
    (7, 2, 13),
    (8, 3, 17),
    (9, 3, 25),
    (10, 4, 33),
    (11, 4, 49),
    (12, 5, 65),
    (13, 5, 97),
    (14, 6, 129),
    (15, 6, 193),
    (16, 7, 257),
    (17, 7, 385),
    (18, 8, 513),
    (19, 8, 769),
    (20, 9, 1025),
    (21, 9, 1537),
    (22, 10, 2049),
    (23, 10, 3073),
    (24, 11, 4097),
    (25, 11, 6145),
    (26, 12, 8193),
    (27, 12, 12289),
    (28, 13, 16385),
    (29, 13, 24577),
];

/// The code lengths of the fixed literal alphabet, section 3.2.6: eight
/// bits up to 143, nine to 255, seven for the end of block and the short
/// lengths, eight again for the rest.
pub(crate) fn fixed_literal_lengths() -> [u8; LITERALS] {
    let mut lengths = [8u8; LITERALS];
    for (symbol, length) in lengths.iter_mut().enumerate() {
        // Eight bits up to 143 and again from 280, nine between, and
        // seven for the end of block and the shortest lengths.
        *length = match symbol {
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    lengths
}

/// The code lengths of the fixed distance alphabet, section 3.2.6: five
/// bits, every one of them.
pub(crate) const fn fixed_distance_lengths() -> [u8; DISTANCES] {
    [5u8; DISTANCES]
}

/// The code and the extra bits a length is written as.
pub(crate) fn length_code(length: usize) -> Option<(u16, u8, u16)> {
    let length = u16::try_from(length).ok()?;
    let mut found = None;
    for (code, extra, base) in LENGTHS {
        if base <= length {
            found = Some((code, extra, length.saturating_sub(base)));
        }
    }
    found
}

/// The code and the extra bits a distance is written as.
pub(crate) fn distance_code(distance: usize) -> Option<(u16, u8, u16)> {
    let distance = u16::try_from(distance).ok()?;
    let mut found = None;
    for (code, extra, base) in DISTANCE_CODES {
        if base <= distance {
            found = Some((code, extra, distance.saturating_sub(base)));
        }
    }
    found
}

/// What a length code stands for: how many extra bits it takes and the
/// shortest length it means.
pub(crate) fn length_of(code: u16) -> Option<(u8, u16)> {
    LENGTHS
        .iter()
        .find(|(known, _, _)| *known == code)
        .map(|(_, extra, base)| (*extra, *base))
}

/// What a distance code stands for.
pub(crate) fn distance_of(code: u16) -> Option<(u8, u16)> {
    DISTANCE_CODES
        .iter()
        .find(|(known, _, _)| *known == code)
        .map(|(_, extra, base)| (*extra, *base))
}
