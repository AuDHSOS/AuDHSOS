// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::heap`, against the items of the edge-case catalog
//! 6.6.12.

use test_support::generators::{Generator, range, vec};
use test_support::property::check;

use crate::heap::{AllocError, Allocator, Extent};

/// The allocator the tests use: enough room for the cases and small enough
/// that a test can fill either table.
type Heap = Allocator<32, 8>;

/// An allocator over an arena of `len` bytes.
fn arena(len: u64) -> Heap {
    let mut heap = Heap::new();
    heap.grow(len).unwrap();
    heap
}

/// The free extents of `heap`, as a vector.
fn extents(heap: &Heap) -> Vec<Extent> {
    heap.extents().collect()
}

#[test]
fn a_new_allocator_has_no_arena_and_no_free_space() {
    let heap = Heap::new();
    assert_eq!(heap.arena_len(), 0);
    assert_eq!(heap.free_bytes(), 0);
    assert_eq!(heap.free_extents(), 0);
    assert_eq!(heap.live_blocks(), 0);
}

#[test]
fn growing_from_nothing_makes_one_extent_over_the_whole_arena() {
    let heap = arena(4096);
    assert_eq!(heap.arena_len(), 4096);
    assert_eq!(
        extents(&heap),
        vec![Extent {
            start: 0,
            end: 4096
        }]
    );
}

#[test]
fn growing_again_extends_the_extent_that_reaches_the_end() {
    let mut heap = arena(4096);
    heap.grow(8192).unwrap();
    assert_eq!(heap.arena_len(), 8192);
    assert_eq!(
        extents(&heap),
        vec![Extent {
            start: 0,
            end: 8192
        }]
    );
}

#[test]
fn growing_behind_a_live_block_at_the_end_adds_an_extent() {
    let mut heap = arena(64);
    let whole = heap.allocate(64, 1).unwrap();
    assert_eq!(whole, 0);
    assert_eq!(heap.free_extents(), 0);
    heap.grow(128).unwrap();
    assert_eq!(
        extents(&heap),
        vec![Extent {
            start: 64,
            end: 128
        }]
    );
}

#[test]
fn growing_to_the_same_length_changes_nothing() {
    let mut heap = arena(4096);
    heap.grow(4096).unwrap();
    assert_eq!(
        extents(&heap),
        vec![Extent {
            start: 0,
            end: 4096
        }]
    );
}

#[test]
fn an_arena_cannot_become_smaller() {
    let mut heap = arena(4096);
    assert_eq!(
        heap.grow(2048),
        Err(AllocError::Shrink {
            current: 4096,
            requested: 2048,
        })
    );
    assert_eq!(heap.arena_len(), 4096);
}

#[test]
fn growing_with_a_full_free_list_is_refused() {
    let (mut heap, _) = blocks(17, &EVERY_SECOND);
    assert_eq!(heap.free_extents(), 8);
    let arena = heap.arena_len();
    // The last block is live, so the new bytes need an extent of their own
    // and the list has no slot for it.
    assert_eq!(
        heap.grow(arena.wrapping_add(64)),
        Err(AllocError::TooFragmented)
    );
    assert_eq!(heap.arena_len(), arena);
}

/// An arena of `count` blocks of twelve bytes each, with the blocks named
/// in `given_back` released again.
///
/// Twelve, so that a hole starts at an address no alignment of these tests
/// lands on: a block asked for with an alignment then falls into the middle
/// of a hole and leaves an extent on either side, which is the case that
/// needs one more extent than there was.
fn blocks(count: usize, given_back: &[usize]) -> (Heap, Vec<u64>) {
    let len = u64::try_from(count).unwrap().wrapping_mul(12);
    let mut heap = arena(len);
    let mut taken = Vec::new();
    for _ in 0..count {
        taken.push(heap.allocate(12, 1).unwrap());
    }
    for index in given_back {
        heap.release(*taken.get(*index).unwrap()).unwrap();
    }
    (heap, taken)
}

/// The eight blocks that leave eight holes with a live block between them.
const EVERY_SECOND: [usize; 8] = [1, 3, 5, 7, 9, 11, 13, 15];

#[test]
fn a_length_of_zero_is_refused() {
    let mut heap = arena(4096);
    assert_eq!(heap.allocate(0, 8), Err(AllocError::ZeroLength));
    assert_eq!(heap.live_blocks(), 0);
}

#[test]
fn a_length_larger_than_the_arena_is_refused() {
    let mut heap = arena(4096);
    assert_eq!(heap.allocate(4097, 1), Err(AllocError::OutOfMemory));
    assert_eq!(heap.free_bytes(), 4096);
}

#[test]
fn the_alignments_the_catalog_names_all_work() {
    for align in [1u64, 8, 4096] {
        let mut heap = arena(8192);
        let offset = heap.allocate(64, align).unwrap();
        assert!(
            offset.is_multiple_of(align),
            "{offset} is not {align}-aligned"
        );
        assert!(offset.checked_add(64).unwrap() <= 8192);
    }
}

#[test]
fn an_alignment_that_is_no_power_of_two_is_refused() {
    let mut heap = arena(4096);
    for align in [0u64, 3, 6, 12, 1000] {
        assert_eq!(heap.allocate(16, align), Err(AllocError::Alignment(align)));
    }
    assert_eq!(heap.live_blocks(), 0);
}

#[test]
fn an_alignment_larger_than_the_arena_finds_no_room() {
    let mut heap = arena(4096);
    // Offset zero satisfies every alignment, so the arena has to be busy
    // there before an alignment above it can fail to be met.
    assert_eq!(heap.allocate(16, 1).unwrap(), 0);
    assert_eq!(heap.allocate(16, 8192), Err(AllocError::OutOfMemory));
    assert_eq!(heap.free_bytes(), 4080);
}

#[test]
fn a_length_and_an_alignment_that_do_not_fit_a_word_are_refused() {
    let mut heap = arena(4096);
    // `len + align - 1` is exactly what overflows here.
    assert_eq!(heap.allocate(u64::MAX, 2), Err(AllocError::Overflow),);
    assert_eq!(heap.allocate(u64::MAX, 1), Err(AllocError::OutOfMemory));
}

#[test]
fn three_blocks_where_the_middle_and_the_first_are_freed_become_one() {
    // The item of the catalog: allocate a, b, c; free b; free a; the two
    // merge; an allocation of a + b fits.
    let mut heap = arena(4096);
    let a = heap.allocate(64, 1).unwrap();
    let b = heap.allocate(64, 1).unwrap();
    let c = heap.allocate(64, 1).unwrap();
    assert_eq!((a, b, c), (0, 64, 128));

    heap.release(b).unwrap();
    heap.release(a).unwrap();
    assert_eq!(
        extents(&heap),
        vec![
            Extent { start: 0, end: 128 },
            Extent {
                start: 192,
                end: 4096
            }
        ]
    );
    let merged = heap.allocate(128, 1).unwrap();
    assert_eq!(merged, 0);
}

#[test]
fn a_freed_block_between_two_free_neighbours_joins_both() {
    let mut heap = arena(4096);
    let a = heap.allocate(64, 1).unwrap();
    let b = heap.allocate(64, 1).unwrap();
    let c = heap.allocate(64, 1).unwrap();
    heap.release(a).unwrap();
    heap.release(c).unwrap();
    assert_eq!(heap.free_extents(), 2);
    heap.release(b).unwrap();
    assert_eq!(
        extents(&heap),
        vec![Extent {
            start: 0,
            end: 4096
        }]
    );
}

#[test]
fn a_freed_block_that_touches_the_extent_below_it_extends_it() {
    let mut heap = arena(4096);
    let a = heap.allocate(64, 1).unwrap();
    let b = heap.allocate(64, 1).unwrap();
    heap.release(a).unwrap();
    heap.release(b).unwrap();
    assert_eq!(
        extents(&heap),
        vec![Extent {
            start: 0,
            end: 4096
        }]
    );
}

#[test]
fn releasing_everything_leaves_one_extent_whatever_the_order() {
    for order in [[0usize, 1, 2], [2, 1, 0], [1, 0, 2], [1, 2, 0]] {
        let mut heap = arena(4096);
        let offsets = [
            heap.allocate(64, 1).unwrap(),
            heap.allocate(128, 8).unwrap(),
            heap.allocate(32, 16).unwrap(),
        ];
        for index in order {
            heap.release(*offsets.get(index).unwrap()).unwrap();
        }
        assert_eq!(
            extents(&heap),
            vec![Extent {
                start: 0,
                end: 4096
            }],
            "order {order:?}"
        );
        assert_eq!(heap.live_blocks(), 0);
        assert_eq!(heap.free_bytes(), 4096);
    }
}

#[test]
fn exhaustion_returns_an_error_and_the_arena_comes_back() {
    let mut heap = arena(256);
    let mut offsets = Vec::new();
    while let Ok(offset) = heap.allocate(32, 1) {
        offsets.push(offset);
    }
    assert_eq!(offsets.len(), 8);
    assert_eq!(heap.allocate(32, 1), Err(AllocError::OutOfMemory));
    for offset in offsets {
        heap.release(offset).unwrap();
    }
    assert_eq!(extents(&heap), vec![Extent { start: 0, end: 256 }]);
}

#[test]
fn the_block_table_runs_out_before_the_memory_does() {
    let mut heap = arena(4096);
    for _ in 0..32 {
        heap.allocate(8, 1).unwrap();
    }
    assert_eq!(heap.live_blocks(), 32);
    assert_eq!(heap.allocate(8, 1), Err(AllocError::TooManyBlocks));
    assert!(heap.free_bytes() > 0);
}

#[test]
fn a_split_that_needs_a_ninth_extent_is_refused() {
    let (mut heap, _) = blocks(17, &EVERY_SECOND);
    assert_eq!(heap.free_extents(), 8);
    // The first hole runs from twelve to twenty-four, so a block of four
    // bytes aligned to eight lands at sixteen and leaves an extent on
    // either side of itself. The list has no slot for the second.
    assert_eq!(heap.allocate(4, 8), Err(AllocError::TooFragmented));
    assert_eq!(heap.free_extents(), 8);
}

#[test]
fn a_release_that_needs_a_ninth_extent_is_refused() {
    // Eight holes with a live block between them, and behind them four
    // live blocks in a row. Releasing the second of those four touches no
    // hole at all, so its bytes need an extent of their own.
    let (mut heap, taken) = blocks(20, &EVERY_SECOND);
    assert_eq!(heap.free_extents(), 8);
    let isolated = *taken.get(17).unwrap();
    assert_eq!(heap.release(isolated), Err(AllocError::TooFragmented));
    assert_eq!(heap.live_blocks(), 12);
    // Its neighbour below touches the last hole, so that one still works.
    let joins = *taken.get(16).unwrap();
    assert_eq!(heap.release(joins).unwrap(), 12);
    assert_eq!(heap.free_extents(), 8);
}

#[test]
fn releasing_a_block_that_was_never_handed_out_is_refused() {
    let mut heap = arena(4096);
    let offset = heap.allocate(64, 1).unwrap();
    assert_eq!(heap.release(1), Err(AllocError::NotAllocated(1)));
    heap.release(offset).unwrap();
    assert_eq!(
        heap.release(offset),
        Err(AllocError::NotAllocated(offset)),
        "a second release names no block"
    );
}

#[test]
fn an_alignment_leaves_the_bytes_before_the_block_free() {
    let mut heap = arena(4096);
    let first = heap.allocate(8, 1).unwrap();
    assert_eq!(first, 0);
    let aligned = heap.allocate(8, 64).unwrap();
    assert_eq!(aligned, 64);
    assert_eq!(
        extents(&heap),
        vec![
            Extent { start: 8, end: 64 },
            Extent {
                start: 72,
                end: 4096
            }
        ]
    );
}

#[test]
fn the_lengths_and_the_counts_are_what_was_asked_for() {
    let mut heap = arena(4096);
    heap.allocate(10, 1).unwrap();
    heap.allocate(20, 8).unwrap();
    assert_eq!(heap.live_bytes(), 30);
    assert_eq!(heap.live_blocks(), 2);
    let blocks: Vec<u64> = heap.blocks().map(|block| block.len).collect();
    assert_eq!(blocks, vec![10, 20]);
}

#[test]
fn an_extent_of_no_bytes_is_empty() {
    let extent = Extent { start: 8, end: 8 };
    assert!(extent.is_empty());
    assert_eq!(extent.len(), 0);
    let extent = Extent { start: 8, end: 24 };
    assert!(!extent.is_empty());
    assert_eq!(extent.len(), 16);
}

#[test]
fn every_error_renders_a_message() {
    let cases = [
        AllocError::ZeroLength,
        AllocError::Alignment(3),
        AllocError::Overflow,
        AllocError::OutOfMemory,
        AllocError::TooManyBlocks,
        AllocError::TooFragmented,
        AllocError::NotAllocated(64),
        AllocError::Shrink {
            current: 8,
            requested: 4,
        },
    ];
    for case in cases {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
    }
}

/// One step of the model: an allocation of a length and an alignment, or
/// the release of the block at a place in the live list.
#[derive(Clone, Copy, Debug)]
enum Step {
    Allocate { len: u64, align: u64 },
    Release { which: usize },
}

#[test]
fn live_blocks_never_overlap_and_the_arena_always_comes_back() {
    let steps = vec(
        range(0u64..=100).map(|value| {
            if value.is_multiple_of(3) {
                Step::Release {
                    which: usize::try_from(value).unwrap_or(0),
                }
            } else {
                Step::Allocate {
                    len: value.wrapping_mul(3).wrapping_rem(200).wrapping_add(1),
                    align: 1u64 << value.wrapping_rem(5),
                }
            }
        }),
        0..=40,
    );
    check("heap blocks stay disjoint", &steps, |steps| {
        let mut heap: Allocator<64, 32> = Allocator::new();
        heap.grow(ARENA).map_err(|error| format!("{error}"))?;
        let mut live: Vec<(u64, u64)> = Vec::new();
        for step in steps {
            match *step {
                Step::Allocate { len, align } => {
                    if let Ok(offset) = heap.allocate(len, align) {
                        check_block(offset, len, align, &live)?;
                        live.push((offset, len));
                    }
                }
                Step::Release { which } => {
                    if !live.is_empty() {
                        let (offset, _) = live.remove(which % live.len());
                        heap.release(offset).map_err(|error| format!("{error}"))?;
                    }
                }
            }
        }
        for (offset, _) in live {
            heap.release(offset).map_err(|error| format!("{error}"))?;
        }
        if heap.free_bytes() != ARENA || heap.free_extents() != 1 {
            return Err(format!(
                "the arena came back as {} bytes in {} extents",
                heap.free_bytes(),
                heap.free_extents()
            ));
        }
        Ok(())
    });
}

/// The arena the property runs over.
const ARENA: u64 = 4096;

/// What every handed-out block has to satisfy: it is aligned, it lies
/// inside the arena, and it touches no block that is already live.
fn check_block(offset: u64, len: u64, align: u64, live: &[(u64, u64)]) -> Result<(), String> {
    if !offset.is_multiple_of(align) {
        return Err(format!("{offset} is not aligned to {align}"));
    }
    if offset.checked_add(len).is_none_or(|end| end > ARENA) {
        return Err(format!("{offset}+{len} leaves the arena"));
    }
    for (other, other_len) in live {
        let ends_below = offset.wrapping_add(len) <= *other;
        let starts_above = other.wrapping_add(*other_len) <= offset;
        if !ends_below && !starts_above {
            return Err(format!("{offset}+{len} overlaps {other}+{other_len}"));
        }
    }
    Ok(())
}
