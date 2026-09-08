// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::store`, against the items of the edge-case catalog
//! 6.6.23.

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Handle};
use test_support::generators::{Generator, range, vec};
use test_support::property::check;

use crate::doubles::{Call, RecordingPages};
use crate::store::{Object, Store};

/// The store the tests work with: small enough that either table can be
/// filled, wide enough for the cases.
type Memory = Store<8, 8>;

/// Two mebibytes, which is the alignment the catalog names beside a page.
const TWO_MIB: u64 = 2 * 1024 * 1024;

/// A handle every table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// An object of `pages` pages at `start`.
fn object(index: u32, start: u64, pages: u64) -> Object {
    Object {
        handle: handle(index),
        start,
        len: pages.wrapping_mul(PAGE_SIZE),
    }
}

/// A store that has taken over one region of `pages` pages at zero.
fn adopted(pages: u64) -> (Memory, RecordingPages) {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    store.adopt(&mut kernel, object(1, 0, pages)).unwrap();
    kernel.forget();
    (store, kernel)
}

/// The map, zero, and unmap of one pass, as a triple.
fn pass(calls: &[Call]) -> Option<(Handle, u64, u64)> {
    let mut walk = calls.iter();
    let (object, len, address) = match walk.next()? {
        Call::Map {
            object,
            offset: 0,
            len,
            address,
        } => (*object, *len, *address),
        _ => return None,
    };
    match (walk.next()?, walk.next()?) {
        (
            Call::Zero {
                address: zero_at,
                len: zero_len,
            },
            Call::Unmap {
                address: unmap_at,
                len: unmap_len,
            },
        ) if *zero_at == address
            && *zero_len == len
            && *unmap_at == address
            && *unmap_len == len =>
        {
            Some((object, address, len))
        }
        _ => None,
    }
}

#[test]
fn a_new_store_holds_nothing() {
    let store = Memory::new();
    assert_eq!(store.free_bytes(), 0);
    assert_eq!(store.live_bytes(), 0);
    assert_eq!(store.free_objects(), 0);
    assert_eq!(store.live_objects(), 0);
    assert_eq!(Memory::default().free_bytes(), 0);
}

#[test]
fn returned_shared_pages_are_neither_zeroed_nor_reallocated_until_exclusive() {
    let (mut store, mut pages) = adopted(1);
    let ring = store.allocate(&mut pages, 5, PAGE_SIZE, PAGE_SIZE).unwrap();
    pages.shared.push(ring.handle);
    pages.forget();
    store.release(&mut pages, 5, ring).unwrap();
    assert!(
        pages.zeroed().is_empty(),
        "the previous client can still be reading its page"
    );
    assert_eq!(store.free_bytes(), 0);
    assert_eq!(
        store.allocate(&mut pages, 6, PAGE_SIZE, PAGE_SIZE),
        Err(Error::OutOfMemory)
    );
    pages.shared.clear();
    let next = store.allocate(&mut pages, 6, PAGE_SIZE, PAGE_SIZE).unwrap();
    assert_eq!(next.start, ring.start);
    assert_eq!(
        pages.zeroed().len(),
        2,
        "zero on reclaim and again on allocation"
    );
}

#[test]
fn client_end_does_not_recycle_memory_transferred_to_another_process() {
    let (mut store, mut pages) = adopted(1);
    let ring = store.allocate(&mut pages, 5, PAGE_SIZE, PAGE_SIZE).unwrap();
    pages.shared.push(ring.handle);
    pages.forget();
    assert_eq!(store.forget_client(&mut pages, 5), Ok(1));
    assert!(pages.zeroed().is_empty());
    assert_eq!(
        store.allocate(&mut pages, 6, PAGE_SIZE, PAGE_SIZE),
        Err(Error::OutOfMemory)
    );
    pages.shared.clear();
    store.reclaim(&mut pages).unwrap();
    assert_eq!(store.free_bytes(), PAGE_SIZE);
}

#[test]
fn memory_taken_over_is_zeroed_before_it_is_free() {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    let region = object(1, 0, 4);
    store.adopt(&mut kernel, region).unwrap();
    let (zeroed, _, len) = pass(kernel.calls()).expect("map, zero, unmap");
    assert_eq!(zeroed, region.handle);
    assert_eq!(len, region.len, "the pass covers the whole object");
    assert_eq!(kernel.calls().len(), 3, "one pass and nothing else");
    assert_eq!(store.free_bytes(), region.len);
}

#[test]
fn an_allocation_is_zeroed_over_its_whole_range_before_the_handle_goes_out() {
    let (mut store, mut kernel) = adopted(8);
    let given = store
        .allocate(&mut kernel, 7, 2 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let zeroing: Vec<(u64, u64)> = kernel.zeroed();
    assert_eq!(zeroing.len(), 1, "exactly one pass");
    let (_, len) = *zeroing.first().unwrap();
    assert_eq!(len, given.len, "the recorded range is the object range");
    assert_eq!(given.len, 2 * PAGE_SIZE);
    assert_eq!(store.live_bytes(), 2 * PAGE_SIZE);
    assert_eq!(store.free_bytes(), 6 * PAGE_SIZE);
}

#[test]
fn a_release_is_zeroed_at_once_and_the_memory_goes_back() {
    let (mut store, mut kernel) = adopted(8);
    let given = store
        .allocate(&mut kernel, 7, 2 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    kernel.forget();
    let back = store.release(&mut kernel, 7, given).unwrap();
    assert_eq!(back, given);
    let zeroing = kernel.zeroed();
    assert_eq!(zeroing.len(), 1, "exactly one pass");
    assert_eq!(zeroing.first().unwrap().1, given.len);
    assert_eq!(store.live_objects(), 0);
    assert_eq!(store.free_bytes(), 8 * PAGE_SIZE);
    assert_eq!(store.free_objects(), 1, "and the region is whole again");
}

#[test]
fn an_object_that_comes_back_under_another_name_is_recognized_and_the_name_given_up() {
    let (mut store, mut kernel) = adopted(8);
    let given = store
        .allocate(&mut kernel, 7, 2 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    kernel.forget();
    // A handle that travels through a message arrives under a number of the
    // receiver's own, so what comes back names the same memory and nothing
    // else about it agrees.
    let returned = Object {
        handle: handle(9999),
        ..given
    };
    assert_eq!(store.release(&mut kernel, 7, returned).unwrap(), given);
    assert!(
        kernel.calls().contains(&Call::Close {
            handle: handle(9999)
        }),
        "the second name for the object was not given up"
    );
    assert_eq!(store.live_objects(), 0);
    assert_eq!(store.free_objects(), 1, "and the region is whole again");
}

#[test]
fn memory_returned_at_a_length_the_store_never_handed_out_is_refused() {
    let (mut store, mut kernel) = adopted(8);
    let given = store
        .allocate(&mut kernel, 7, 2 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    kernel.forget();
    let returned = Object {
        len: PAGE_SIZE,
        ..given
    };
    assert_eq!(
        store.release(&mut kernel, 7, returned).unwrap_err(),
        Error::InvalidArgument
    );
    assert!(kernel.calls().is_empty());
    assert_eq!(store.live_objects(), 1, "and it still holds it");
}

#[test]
fn a_join_the_kernel_refuses_leaves_the_two_objects_where_they_are() {
    let (mut store, mut kernel) = adopted(8);
    let low = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let high = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    store.release(&mut kernel, 7, low).unwrap();
    let before = store.free_objects();
    // A client that has not yet given its capability up is the usual reason
    // the kernel refuses; the store keeps two free objects rather than
    // failing the release.
    kernel.refuse_merges = true;
    store.release(&mut kernel, 7, high).unwrap();
    assert_eq!(store.live_objects(), 0);
    assert_eq!(
        store.free_objects(),
        before.wrapping_add(1),
        "the object went back and stayed a piece of its own"
    );
    assert_eq!(store.free_bytes(), 8 * PAGE_SIZE, "and no memory was lost");
}

#[test]
fn a_second_release_of_the_same_object_is_refused_and_zeroes_nothing() {
    let (mut store, mut kernel) = adopted(8);
    let given = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    store.release(&mut kernel, 7, given).unwrap();
    kernel.forget();
    assert_eq!(
        store.release(&mut kernel, 7, given).unwrap_err(),
        Error::NotFound
    );
    assert!(
        kernel.calls().is_empty(),
        "a refused release zeroes nothing"
    );
}

#[test]
fn a_release_of_an_object_this_store_never_handed_out_is_refused() {
    let (mut store, mut kernel) = adopted(8);
    kernel.forget();
    assert_eq!(
        store
            .release(
                &mut kernel,
                7,
                Object {
                    handle: handle(4242),
                    start: 0xDEAD_0000,
                    len: PAGE_SIZE,
                },
            )
            .unwrap_err(),
        Error::NotFound
    );
    assert!(kernel.calls().is_empty());
}

#[test]
fn a_client_may_not_release_what_another_holds() {
    let (mut store, mut kernel) = adopted(8);
    let given = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    kernel.forget();
    assert_eq!(
        store.release(&mut kernel, 9, given).unwrap_err(),
        Error::AccessDenied
    );
    assert!(kernel.calls().is_empty());
    assert_eq!(store.live_objects(), 1, "and it still holds it");
}

#[test]
fn the_memory_of_a_dead_client_is_treated_as_returned() {
    let (mut store, mut kernel) = adopted(8);
    let first = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let second = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let other = store
        .allocate(&mut kernel, 9, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    kernel.forget();
    assert_eq!(store.forget_client(&mut kernel, 7).unwrap(), 2);
    let zeroing = kernel.zeroed();
    assert_eq!(zeroing.len(), 2, "one pass per object");
    assert!(
        zeroing.iter().all(|(_, len)| *len == PAGE_SIZE),
        "each over the whole object"
    );
    assert_eq!(store.live_objects(), 1);
    assert_eq!(store.held().next().unwrap().object, other);
    let _ = (first, second);
}

#[test]
fn a_client_that_holds_nothing_leaves_nothing_behind() {
    let (mut store, mut kernel) = adopted(8);
    let _given = store.allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE);
    kernel.forget();
    assert_eq!(store.forget_client(&mut kernel, 9).unwrap(), 0);
    assert!(kernel.calls().is_empty());
}

#[test]
fn the_alignments_the_catalog_names_are_met() {
    for align in [PAGE_SIZE, TWO_MIB] {
        let mut store = Memory::new();
        let mut kernel = RecordingPages::new();
        // A region that begins one page below a two-mebibyte boundary, so
        // that the wider alignment costs a piece at the front.
        let start = TWO_MIB.wrapping_sub(PAGE_SIZE);
        store.adopt(&mut kernel, object(1, start, 4 * 512)).unwrap();
        let given = store.allocate(&mut kernel, 7, PAGE_SIZE, align).unwrap();
        assert!(
            given.start.is_multiple_of(align),
            "{:#x} is not {align:#x}-aligned",
            given.start
        );
    }
}

#[test]
fn an_alignment_no_free_object_can_meet_finds_nothing() {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    // Four pages that begin one page above a two-mebibyte boundary: the
    // next boundary lies far beyond the end of them.
    let start = TWO_MIB.wrapping_add(PAGE_SIZE);
    store.adopt(&mut kernel, object(1, start, 4)).unwrap();
    assert_eq!(
        store
            .allocate(&mut kernel, 7, PAGE_SIZE, TWO_MIB)
            .unwrap_err(),
        Error::OutOfMemory
    );
}

#[test]
fn a_length_of_zero_is_refused() {
    let (mut store, mut kernel) = adopted(4);
    assert_eq!(
        store.allocate(&mut kernel, 7, 0, PAGE_SIZE).unwrap_err(),
        Error::InvalidArgument
    );
}

#[test]
fn an_alignment_that_is_no_power_of_two_is_refused() {
    let (mut store, mut kernel) = adopted(4);
    for align in [0u64, 3, 6, 4095] {
        assert_eq!(
            store
                .allocate(&mut kernel, 7, PAGE_SIZE, align)
                .unwrap_err(),
            Error::InvalidArgument
        );
    }
}

#[test]
fn a_request_without_a_badge_is_refused() {
    let (mut store, mut kernel) = adopted(4);
    assert_eq!(
        store
            .allocate(&mut kernel, 0, PAGE_SIZE, PAGE_SIZE)
            .unwrap_err(),
        Error::InvalidArgument
    );
}

#[test]
fn a_length_that_is_no_whole_page_is_rounded_up() {
    let (mut store, mut kernel) = adopted(8);
    let given = store.allocate(&mut kernel, 7, 1, PAGE_SIZE).unwrap();
    assert_eq!(given.len, PAGE_SIZE);
    let given = store
        .allocate(&mut kernel, 7, PAGE_SIZE.wrapping_add(1), PAGE_SIZE)
        .unwrap();
    assert_eq!(given.len, 2 * PAGE_SIZE);
}

#[test]
fn exhaustion_is_an_error_and_a_release_makes_the_request_work_again() {
    let (mut store, mut kernel) = adopted(4);
    let mut given = Vec::new();
    for _ in 0..4 {
        given.push(
            store
                .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
                .unwrap(),
        );
    }
    assert_eq!(
        store
            .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
            .unwrap_err(),
        Error::OutOfMemory
    );
    store
        .release(&mut kernel, 7, *given.first().unwrap())
        .unwrap();
    let again = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    assert_eq!(again.len, PAGE_SIZE);
}

#[test]
fn two_released_neighbours_are_handed_out_as_one() {
    // The item of the catalog that `memory_merge` was added for (D-90).
    let (mut store, mut kernel) = adopted(4);
    let first = store
        .allocate(&mut kernel, 7, 2 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let second = store
        .allocate(&mut kernel, 7, 2 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    assert_eq!(second.start, first.end());
    assert_eq!(store.free_objects(), 0);

    store.release(&mut kernel, 7, first).unwrap();
    store.release(&mut kernel, 7, second).unwrap();
    assert_eq!(store.free_objects(), 1, "the two became one");
    assert_eq!(store.free_bytes(), 4 * PAGE_SIZE);

    let whole = store
        .allocate(&mut kernel, 7, 4 * PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    assert_eq!(whole.len, 4 * PAGE_SIZE);
}

#[test]
fn a_release_joins_the_neighbour_below_and_the_one_above() {
    let (mut store, mut kernel) = adopted(6);
    let low = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let middle = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    let high = store
        .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
        .unwrap();
    store.release(&mut kernel, 7, low).unwrap();
    store.release(&mut kernel, 7, high).unwrap();
    assert_eq!(store.free_objects(), 2, "a hole where the middle one is");
    kernel.forget();
    store.release(&mut kernel, 7, middle).unwrap();
    assert_eq!(store.free_objects(), 1);
    assert_eq!(store.free_bytes(), 6 * PAGE_SIZE);
    let merges = kernel
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Merge { .. }))
        .count();
    assert_eq!(merges, 2, "one join upwards and one downwards");
}

#[test]
fn a_piece_taken_out_of_the_middle_leaves_both_ends_free() {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    let start = TWO_MIB.wrapping_sub(PAGE_SIZE);
    store.adopt(&mut kernel, object(1, start, 3)).unwrap();
    let given = store.allocate(&mut kernel, 7, PAGE_SIZE, TWO_MIB).unwrap();
    assert_eq!(given.start, TWO_MIB);
    assert_eq!(store.free_objects(), 2, "one page below, one above");
    let free: Vec<Object> = store.free().collect();
    assert_eq!(free.first().unwrap().start, start);
    assert_eq!(free.get(1).unwrap().start, TWO_MIB.wrapping_add(PAGE_SIZE));
}

#[test]
fn a_full_live_table_takes_no_further_object() {
    let (mut store, mut kernel) = adopted(16);
    for _ in 0..8 {
        store
            .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
            .unwrap();
    }
    assert_eq!(
        store
            .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
            .unwrap_err(),
        Error::PoolExhausted
    );
}

#[test]
fn a_free_list_with_no_room_for_the_pieces_refuses_the_request() {
    // Eight regions that touch nothing, which fills the free list. Each
    // begins one page below a two-mebibyte boundary, so a request aligned
    // to that boundary lands in the middle of one and would leave a piece
    // on either side — two where one was, and there is no slot for it.
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    for index in 0..8u32 {
        let start = u64::from(index)
            .wrapping_mul(4)
            .wrapping_add(1)
            .wrapping_mul(TWO_MIB)
            .wrapping_sub(PAGE_SIZE);
        store
            .adopt(&mut kernel, object(index.wrapping_add(1), start, 3))
            .unwrap();
    }
    assert_eq!(store.free_objects(), 8);
    kernel.forget();
    assert_eq!(
        store
            .allocate(&mut kernel, 7, PAGE_SIZE, TWO_MIB)
            .unwrap_err(),
        Error::PoolExhausted
    );
    assert!(kernel.calls().is_empty(), "and nothing was cut");
}

#[test]
fn taking_over_more_regions_than_the_free_list_holds_is_refused() {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    for index in 0..8u32 {
        let start = u64::from(index).wrapping_mul(TWO_MIB);
        store
            .adopt(&mut kernel, object(index.wrapping_add(1), start, 4))
            .unwrap();
    }
    assert_eq!(
        store
            .adopt(&mut kernel, object(99, 100 * TWO_MIB, 4))
            .unwrap_err(),
        Error::PoolExhausted
    );
}

#[test]
fn an_error_from_the_kernel_comes_back_out() {
    let (mut store, mut kernel) = adopted(8);
    kernel.fail_after = Some((0, Error::OutOfKernelMemory));
    assert_eq!(
        store
            .allocate(&mut kernel, 7, PAGE_SIZE, PAGE_SIZE)
            .unwrap_err(),
        Error::OutOfKernelMemory
    );
}

#[test]
fn a_failed_unmap_is_reported_even_when_the_fill_worked() {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    // Map and zero go through; the unmap behind them does not.
    kernel.fail_after = Some((2, Error::NotMapped));
    assert_eq!(
        store.adopt(&mut kernel, object(1, 0, 2)).unwrap_err(),
        Error::NotMapped
    );
    assert_eq!(kernel.calls().len(), 3, "the object came out all the same");
}

#[test]
fn the_free_objects_come_out_lowest_first() {
    let mut store = Memory::new();
    let mut kernel = RecordingPages::new();
    store.adopt(&mut kernel, object(1, 8 * TWO_MIB, 2)).unwrap();
    store.adopt(&mut kernel, object(2, 2 * TWO_MIB, 2)).unwrap();
    store.adopt(&mut kernel, object(3, 5 * TWO_MIB, 2)).unwrap();
    let starts: Vec<u64> = store.free().map(|free| free.start).collect();
    assert_eq!(starts, vec![2 * TWO_MIB, 5 * TWO_MIB, 8 * TWO_MIB]);
}

#[test]
fn an_object_says_where_it_ends() {
    let region = object(1, 0x2000, 3);
    assert_eq!(region.end(), 0x2000 + 3 * PAGE_SIZE);
}

/// One step of the model: a request of a number of pages, or the release of
/// the object at a place in the list of what is out.
#[derive(Clone, Copy, Debug)]
enum Step {
    Allocate { pages: u64 },
    Release { which: usize },
}

/// How many pages the property runs over.
const ARENA_PAGES: u64 = 64;

#[test]
fn what_is_out_never_overlaps_and_every_byte_of_it_was_zeroed() {
    let steps = vec(
        range(0u64..=60).map(|value| {
            if value.is_multiple_of(3) {
                Step::Release {
                    which: usize::try_from(value).unwrap_or(0),
                }
            } else {
                Step::Allocate {
                    pages: value.wrapping_rem(5).wrapping_add(1),
                }
            }
        }),
        0..=30,
    );
    check("memory store keeps its ranges apart", &steps, |steps| {
        let mut store: Store<32, 32> = Store::new();
        let mut kernel = RecordingPages::new();
        store
            .adopt(&mut kernel, object(1, 0, ARENA_PAGES))
            .map_err(|error| format!("{error:?}"))?;
        let mut out: Vec<Object> = Vec::new();
        for step in steps {
            match *step {
                Step::Allocate { pages } => {
                    kernel.forget();
                    if let Ok(given) = store.allocate(&mut kernel, 7, pages * PAGE_SIZE, PAGE_SIZE)
                    {
                        check_given(given, &out, &kernel)?;
                        out.push(given);
                    }
                }
                Step::Release { which } => {
                    if !out.is_empty() {
                        let given = out.remove(which % out.len());
                        store
                            .release(&mut kernel, 7, given)
                            .map_err(|error| format!("{error:?}"))?;
                    }
                }
            }
        }
        for given in out {
            store
                .release(&mut kernel, 7, given)
                .map_err(|error| format!("{error:?}"))?;
        }
        if store.free_bytes() != ARENA_PAGES * PAGE_SIZE {
            return Err(format!("{} bytes came back", store.free_bytes()));
        }
        if store.free_objects() != 1 {
            return Err(format!("in {} pieces", store.free_objects()));
        }
        Ok(())
    });
}

/// What every handed-out object has to satisfy: it lies inside the region,
/// it touches nothing that is already out, and the calls that led to it
/// hold exactly one zeroing pass over the whole of it.
fn check_given(given: Object, out: &[Object], kernel: &RecordingPages) -> Result<(), String> {
    if given.end() > ARENA_PAGES * PAGE_SIZE {
        return Err(format!("{given:?} leaves the region"));
    }
    for other in out {
        if given.start < other.end() && other.start < given.end() {
            return Err(format!("{given:?} overlaps {other:?}"));
        }
    }
    let zeroing = kernel.zeroed();
    match zeroing.as_slice() {
        [(_, len)] if *len == given.len => Ok(()),
        other => Err(format!("{other:?} is no single pass over {}", given.len)),
    }
}
