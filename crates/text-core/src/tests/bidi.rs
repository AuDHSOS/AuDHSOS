// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    bidi::{self, Direction, Unit},
    unicode::BidiClass as B,
};
fn source(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/unicode/ucd")
            .join(name),
    )
    .expect("bidi conformance file")
}
fn levels(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|n| {
            if n == "x" {
                255
            } else {
                n.parse().expect("level")
            }
        })
        .collect()
}
fn order(text: &str) -> Vec<usize> {
    text.split_whitespace()
        .map(|n| n.parse().expect("index"))
        .collect()
}
fn class(name: &str) -> B {
    match name {
        "L" => B::L,
        "R" => B::R,
        "AL" => B::Al,
        "EN" => B::En,
        "ES" => B::Es,
        "ET" => B::Et,
        "AN" => B::An,
        "CS" => B::Cs,
        "NSM" => B::Nsm,
        "BN" => B::Bn,
        "B" => B::B,
        "S" => B::S,
        "WS" => B::Ws,
        "ON" => B::On,
        "LRE" => B::Lre,
        "LRO" => B::Lro,
        "RLE" => B::Rle,
        "RLO" => B::Rlo,
        "PDF" => B::Pdf,
        "LRI" => B::Lri,
        "RLI" => B::Rli,
        "FSI" => B::Fsi,
        "PDI" => B::Pdi,
        _ => panic!("unknown bidi class {name}"),
    }
}
#[test]
fn complete_bidi_class_conformance() {
    let mut expected_levels = Vec::new();
    let mut expected_order = Vec::new();
    let mut cases = 0_usize;
    let mut directions = 0_usize;
    let mut units = Vec::new();
    let mut seq = Vec::new();
    let mut actual_levels = Vec::new();
    let mut actual_order = Vec::new();
    for (line_number, line) in source("BidiTest.txt").lines().enumerate() {
        let line = line.split('#').next().expect("line").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(value) = line.strip_prefix("@Levels:") {
            expected_levels = levels(value);
            continue;
        }
        if let Some(value) = line.strip_prefix("@Reorder:") {
            expected_order = order(value);
            continue;
        }
        let (classes, mask) = line.split_once(';').expect("data row");
        let classes: Vec<_> = classes.split_whitespace().map(class).collect();
        let mask = u8::from_str_radix(mask.trim(), 16).expect("direction bitset");
        units.resize(classes.len(), Unit::default());
        seq.resize(classes.len(), 0);
        actual_levels.resize(classes.len(), 0);
        actual_order.resize(classes.len(), 0);
        for (bit, direction) in [
            (1, Direction::Auto),
            (2, Direction::LeftToRight),
            (4, Direction::RightToLeft),
        ] {
            if mask & bit == 0 {
                continue;
            }
            let info =
                bidi::resolve_classes(&classes, direction, &mut units, &mut seq).expect("resolve");
            let visual =
                bidi::reorder_line(&units, 0..info.count, &mut actual_levels, &mut actual_order)
                    .expect("reorder");
            assert_eq!(
                actual_levels,
                expected_levels,
                "BidiTest line {} {direction:?}: {classes:?}",
                line_number.checked_add(1).expect("line")
            );
            assert_eq!(
                &actual_order[..visual.count],
                expected_order,
                "BidiTest line {} {direction:?}: {classes:?}",
                line_number.checked_add(1).expect("line")
            );
            directions = directions.checked_add(1).expect("count");
        }
        cases = cases.checked_add(1).expect("count");
    }
    assert_eq!(cases, 490_846);
    assert_eq!(directions, 770_241);
    eprintln!("BidiTest: {cases} rows, {directions} direction cases");
}
#[test]
fn complete_bidi_character_conformance() {
    let mut count = 0_usize;
    for (line_number, line) in source("BidiCharacterTest.txt").lines().enumerate() {
        let line = line.split('#').next().expect("line").trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split(';').map(str::trim).collect();
        assert_eq!(fields.len(), 5);
        let text: String = fields[0]
            .split_whitespace()
            .map(|n| {
                char::from_u32(u32::from_str_radix(n, 16).expect("code point")).expect("scalar")
            })
            .collect();
        let direction = match fields[1] {
            "0" => Direction::LeftToRight,
            "1" => Direction::RightToLeft,
            "2" => Direction::Auto,
            _ => panic!("direction"),
        };
        let expected_base: u8 = fields[2].parse().expect("paragraph level");
        let expected_levels = levels(fields[3]);
        let expected_order = order(fields[4]);
        let mut units = vec![Unit::default(); expected_levels.len()];
        let mut seq = vec![0; units.len()];
        let mut actual_levels = vec![0; units.len()];
        let mut actual_order = vec![0; units.len()];
        let info = bidi::resolve(&text, direction, &mut units, &mut seq).expect("resolve");
        assert_eq!(
            units[0].paragraph_level(),
            expected_base,
            "line {line_number}"
        );
        let visual =
            bidi::reorder_line(&units, 0..info.count, &mut actual_levels, &mut actual_order)
                .expect("reorder");
        assert_eq!(
            actual_levels,
            expected_levels,
            "BidiCharacterTest line {}: {text:?}",
            line_number.checked_add(1).expect("line")
        );
        assert_eq!(
            &actual_order[..visual.count],
            expected_order,
            "BidiCharacterTest line {}: {text:?}",
            line_number.checked_add(1).expect("line")
        );
        count = count.checked_add(1).expect("count");
    }
    assert_eq!(count, 91707);
    eprintln!("BidiCharacterTest: {count} cases");
}

#[test]
fn bidi_paragraphs_line_resets_mirroring_and_buffers() {
    use crate::TextError;
    let mut units = vec![Unit::default(); 64];
    let mut seq = vec![0; 64];
    let mut levels = vec![0; 64];
    let mut order = vec![0; 64];
    let info =
        bidi::resolve("א \nA \tB", Direction::Auto, &mut units, &mut seq).expect("paragraphs");
    assert_eq!(
        info,
        bidi::Info {
            count: 7,
            paragraphs: 2
        }
    );
    assert_eq!(
        units[..7]
            .iter()
            .map(Unit::paragraph_level)
            .collect::<Vec<_>>(),
        [1, 1, 1, 0, 0, 0, 0]
    );
    assert_eq!(
        units[..7].iter().map(Unit::byte).collect::<Vec<_>>(),
        [0, 2, 3, 4, 5, 6, 7]
    );
    assert!(bidi::reorder_line(&units[..7], 0..7, &mut levels, &mut order).is_err());
    bidi::resolve("אa  \tb", Direction::Auto, &mut units, &mut seq).expect("paragraph");
    let saved: Vec<_> = units[..6].iter().map(Unit::level).collect();
    bidi::reorder_line(&units[..6], 0..4, &mut levels, &mut order).expect("first line");
    assert_eq!(&levels[..4], [1, 2, 1, 1]);
    bidi::reorder_line(&units[..6], 4..6, &mut levels, &mut order).expect("second line");
    assert_eq!(&levels[..2], [1, 2]);
    assert_eq!(&order[..2], [5, 4]);
    assert_eq!(
        units[..6].iter().map(Unit::level).collect::<Vec<_>>(),
        saved
    );
    bidi::resolve("(a)", Direction::RightToLeft, &mut units, &mut seq).expect("mirroring");
    assert_eq!(
        units[..3].iter().map(Unit::mirrored).collect::<Vec<_>>(),
        [0x29, 0x61, 0x28]
    );
    assert_eq!(
        bidi::resolve("A", Direction::Auto, &mut [], &mut seq),
        Err(TextError::BufferTooSmall)
    );
    assert_eq!(
        bidi::resolve("A", Direction::Auto, &mut units, &mut []),
        Err(TextError::BufferTooSmall)
    );
    assert_eq!(
        bidi::resolve_classes(&[B::L], Direction::Auto, &mut [], &mut seq),
        Err(TextError::BufferTooSmall)
    );
    assert!(bidi::reorder_line(&units[..3], 0..4, &mut levels, &mut order).is_err());
    assert!(bidi::reorder_line(&units[..3], 0..3, &mut [], &mut order).is_err());
    assert!(bidi::reorder_line(&units[..3], 0..3, &mut levels, &mut []).is_err());
    assert_eq!(
        bidi::resolve("", Direction::Auto, &mut [], &mut []),
        Ok(bidi::Info::default())
    );
    assert_eq!(
        bidi::reorder_line(&[], 0..0, &mut [], &mut []),
        Ok(bidi::LineInfo { count: 0 })
    );
    bidi::resolve_classes(&[B::Bn], Direction::Auto, &mut units, &mut seq).expect("X9");
    assert_eq!(units[0].level(), None);
    assert_eq!(TextError::InvalidInput.to_string(), "invalid text input");
}
#[test]
fn bidi_embedding_and_bracket_limit_boundaries() {
    for depth in [62, 63, 64, 1000] {
        let mut classes = vec![B::Rle; depth];
        classes.push(B::L);
        classes.extend(vec![B::Pdf; depth]);
        classes.push(B::L);
        let mut units = vec![Unit::default(); classes.len()];
        let mut seq = vec![0; classes.len()];
        bidi::resolve_classes(&classes, Direction::Auto, &mut units, &mut seq)
            .expect("overflow accounting");
        assert_eq!(
            units[depth].level(),
            Some(if depth == 62 { 124 } else { 126 })
        );
        assert_eq!(units.last().expect("last").level(), Some(0));
    }
    for (depth, last_level) in [(63, 1), (64, 0)] {
        let text = format!("א{}ב{}a", "(".repeat(depth), ")".repeat(depth));
        let n = text.chars().count();
        let mut units = vec![Unit::default(); n];
        let mut seq = vec![0; n];
        bidi::resolve(&text, Direction::LeftToRight, &mut units, &mut seq).expect("bracket stack");
        assert_eq!(units[n - 2].level(), Some(last_level));
    }
}
