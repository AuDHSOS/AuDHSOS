// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R7: the glyph cache of D-184, its key, its ring and its bound.

use text_core::Fixed;

use crate::{CacheKey, Entry, Glyph, GlyphCache, RasterError};

fn key(glyph: u16) -> CacheKey {
    CacheKey {
        generation: 7,
        face: 1,
        glyph,
        size: Fixed::from_i32(16),
        subpixel: 0,
    }
}

fn image(coverage: &[u8]) -> Glyph<'_> {
    Glyph {
        width: u32::try_from(coverage.len()).unwrap(),
        height: 1,
        left: -2,
        top: 3,
        coverage,
    }
}

#[test]
fn a_hit_returns_the_bytes_it_was_given() {
    let mut ring = [0_u8; 256];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    assert!(cache.is_empty());
    for subpixel in 0..4_u8 {
        let coverage = [subpixel, 10, 20, 30];
        let mut at = key(5);
        at.subpixel = subpixel;
        cache.insert(at, &[], image(&coverage)).unwrap();
    }
    assert_eq!(cache.len(), 4);
    for subpixel in 0..4_u8 {
        let mut at = key(5);
        at.subpixel = subpixel;
        let stored = cache.get(at, &[]).unwrap();
        assert_eq!(stored.coverage, &[subpixel, 10, 20, 30]);
        assert_eq!((stored.width, stored.height), (4, 1));
        assert_eq!((stored.left, stored.top), (-2, 3));
    }
}

#[test]
fn a_change_to_any_key_field_misses() {
    let mut ring = [0_u8; 256];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    let coordinates = [Fixed::ONE, Fixed::ZERO];
    cache
        .insert(key(5), &coordinates, image(&[1, 2, 3]))
        .unwrap();
    assert!(cache.get(key(5), &coordinates).is_some());
    let changes: [CacheKey; 5] = [
        CacheKey {
            generation: 8,
            ..key(5)
        },
        CacheKey { face: 2, ..key(5) },
        key(6),
        CacheKey {
            size: Fixed::from_i32(17),
            ..key(5)
        },
        CacheKey {
            subpixel: 1,
            ..key(5)
        },
    ];
    for changed in changes {
        assert!(
            cache.get(changed, &coordinates).is_none(),
            "{changed:?} still hit"
        );
    }
    assert!(cache.get(key(5), &[Fixed::ZERO, Fixed::ZERO]).is_none());
    assert!(cache.get(key(5), &[Fixed::ONE]).is_none());
}

#[test]
fn stored_coordinates_rule_out_a_digest_collision() {
    // Two instances whose stored bytes differ must not answer each other even
    // if a digest were to agree, which the stored coordinates decide.
    let mut ring = [0_u8; 256];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    let first = [Fixed::from_bits(1), Fixed::from_bits(2)];
    let second = [Fixed::from_bits(2), Fixed::from_bits(1)];
    cache.insert(key(5), &first, image(&[9, 9])).unwrap();
    cache.insert(key(5), &second, image(&[7, 7])).unwrap();
    assert_eq!(cache.get(key(5), &first).unwrap().coverage, &[9, 9]);
    assert_eq!(cache.get(key(5), &second).unwrap().coverage, &[7, 7]);
}

#[test]
fn the_ring_evicts_in_allocation_order() {
    // A ring of sixteen bytes holds four blocks of four.
    let mut ring = [0_u8; 16];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    for glyph in 0..4_u16 {
        let coverage = [u8::try_from(glyph).unwrap(); 4];
        cache.insert(key(glyph), &[], image(&coverage)).unwrap();
    }
    assert_eq!(cache.len(), 4);
    cache.insert(key(4), &[], image(&[4; 4])).unwrap();
    // The oldest is gone and every later one is still there.
    assert!(cache.get(key(0), &[]).is_none());
    for glyph in 1..5_u16 {
        assert!(cache.get(key(glyph), &[]).is_some(), "glyph {glyph}");
    }
}

#[test]
fn a_full_entry_table_evicts_the_oldest() {
    let mut ring = [0_u8; 4096];
    let mut entries = [Entry::default(); 3];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    for glyph in 0..5_u16 {
        cache.insert(key(glyph), &[], image(&[1, 2])).unwrap();
    }
    assert_eq!(cache.len(), 3);
    assert!(cache.get(key(0), &[]).is_none());
    assert!(cache.get(key(1), &[]).is_none());
    for glyph in 2..5_u16 {
        assert!(cache.get(key(glyph), &[]).is_some(), "glyph {glyph}");
    }
}

#[test]
fn a_block_larger_than_the_ring_is_never_stored() {
    let mut ring = [0_u8; 8];
    let mut entries = [Entry::default(); 4];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    cache.insert(key(1), &[], image(&[0; 9])).unwrap();
    assert!(cache.is_empty());
    assert!(cache.get(key(1), &[]).is_none());
    // The ring still works for a block that fits.
    cache.insert(key(2), &[], image(&[5; 8])).unwrap();
    assert_eq!(cache.get(key(2), &[]).unwrap().coverage, &[5; 8]);
}

#[test]
fn a_zero_length_ring_and_an_empty_table_store_nothing() {
    let mut ring: [u8; 0] = [];
    let mut entries = [Entry::default(); 4];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    cache.insert(key(1), &[], image(&[1])).unwrap();
    assert!(cache.is_empty());

    let mut ring = [0_u8; 64];
    let mut entries: [Entry; 0] = [];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    cache.insert(key(1), &[], image(&[1])).unwrap();
    assert!(cache.is_empty());
    assert!(cache.get(key(1), &[]).is_none());
}

#[test]
fn a_changed_generation_resets_the_whole_ring() {
    let mut ring = [0_u8; 64];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    cache.insert(key(1), &[], image(&[1, 2])).unwrap();
    cache.insert(key(2), &[], image(&[3, 4])).unwrap();
    assert_eq!(cache.len(), 2);
    let later = CacheKey {
        generation: 8,
        ..key(3)
    };
    cache.insert(later, &[], image(&[5, 6])).unwrap();
    assert_eq!(cache.len(), 1);
    assert!(cache.get(key(1), &[]).is_none());
    assert_eq!(cache.get(later, &[]).unwrap().coverage, &[5, 6]);
}

#[test]
fn clearing_drops_every_entry() {
    let mut ring = [0_u8; 64];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    cache.insert(key(1), &[], image(&[1, 2])).unwrap();
    cache.clear();
    assert!(cache.is_empty());
    assert!(cache.get(key(1), &[]).is_none());
}

#[test]
fn coverage_shorter_than_its_dimensions_is_refused() {
    let mut ring = [0_u8; 64];
    let mut entries = [Entry::default(); 8];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    let coverage = [0_u8; 3];
    let short = Glyph {
        width: 2,
        height: 2,
        left: 0,
        top: 0,
        coverage: &coverage,
    };
    assert_eq!(
        cache.insert(key(1), &[], short).unwrap_err(),
        RasterError::BufferTooSmall
    );
}

#[test]
fn reuse_of_the_ring_keeps_the_stored_bytes_apart() {
    // Fill the ring several times over and check every surviving entry still
    // answers with its own bytes.
    let mut ring = [0_u8; 64];
    let mut entries = [Entry::default(); 16];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    for glyph in 0..64_u16 {
        let byte = u8::try_from(glyph % 251).unwrap();
        cache.insert(key(glyph), &[], image(&[byte; 7])).unwrap();
        let stored = cache.get(key(glyph), &[]).unwrap();
        assert_eq!(stored.coverage, &[byte; 7], "glyph {glyph}");
    }
    for glyph in 0..64_u16 {
        if let Some(stored) = cache.get(key(glyph), &[]) {
            let byte = u8::try_from(glyph % 251).unwrap();
            assert_eq!(stored.coverage, &[byte; 7], "survivor {glyph}");
        }
    }
}

#[test]
fn a_zero_sized_glyph_is_stored_and_answers_with_no_bytes() {
    let mut ring = [0_u8; 64];
    let mut entries = [Entry::default(); 4];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    let empty: [u8; 0] = [];
    let blank = Glyph {
        width: 0,
        height: 0,
        left: 0,
        top: 0,
        coverage: &empty,
    };
    cache.insert(key(1), &[], blank).unwrap();
    let stored = cache.get(key(1), &[]).unwrap();
    assert_eq!(stored.coverage.len(), 0);
    assert_eq!((stored.width, stored.height), (0, 0));
}

#[test]
fn a_block_of_exactly_the_ring_replaces_everything() {
    let mut ring = [0_u8; 8];
    let mut entries = [Entry::default(); 4];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    cache.insert(key(1), &[], image(&[1, 2, 3, 4])).unwrap();
    cache.insert(key(2), &[], image(&[5, 6, 7, 8])).unwrap();
    assert_eq!(cache.len(), 2);
    cache.insert(key(3), &[], image(&[9; 8])).unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(key(3), &[]).unwrap().coverage, &[9; 8]);
    assert!(cache.get(key(1), &[]).is_none());
    assert!(cache.get(key(2), &[]).is_none());
}

#[test]
fn coordinates_are_stored_beside_the_coverage() {
    let mut ring = [0_u8; 128];
    let mut entries = [Entry::default(); 4];
    let mut cache = GlyphCache::new(&mut ring, &mut entries);
    let coordinates = [Fixed::from_bits(7), Fixed::from_bits(-9), Fixed::ONE];
    cache
        .insert(key(1), &coordinates, image(&[4, 5, 6]))
        .unwrap();
    assert_eq!(
        cache.get(key(1), &coordinates).unwrap().coverage,
        &[4, 5, 6]
    );
    // One axis differing by one bit is a different instance.
    let moved = [
        Fixed::from_bits(7),
        Fixed::from_bits(-9),
        Fixed::from_bits(1),
    ];
    assert!(cache.get(key(1), &moved).is_none());
}
