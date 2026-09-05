// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! The functions the instrumented target calls.
//!
//! The names are the compiler's, not this project's: `sancov`, the pass
//! that rustc runs over a fuzz target, emits calls to them and would
//! otherwise leave the link with nothing to resolve. Their shapes come
//! from LLVM's `SanitizerCoverage`, and what they do with what they are
//! given follows libFuzzer, which is where the idea of remembering
//! comparisons for the mutator comes from.
//!
//! Invariants: a call outside a run of the target is dropped before it
//! touches anything, which is what keeps the engine's own comparisons out
//! of the target's; nothing here allocates or takes a lock, because it
//! runs between two instructions of the target; the state is reached by
//! exactly one thread, the one running the target.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::counters;
use crate::dictionary::Compares;
use crate::feature::ValueMap;

/// What one run of the target left behind for the mutator to read.
#[derive(Debug)]
pub struct Trace {
    /// The last comparisons of four-byte values.
    pub compares4: Compares,
    /// The last comparisons of eight-byte values.
    pub compares8: Compares,
    /// The bits the comparisons of the run set, kept only when the engine
    /// asks for the value profile.
    pub values: ValueMap,
    /// Whether to keep the value profile at all. It costs a clear and a
    /// scan of eight kilobytes for every run, so it is off unless asked
    /// for, exactly as it is in libFuzzer.
    pub value_profile: bool,
}

impl Trace {
    /// An empty trace.
    const fn new() -> Self {
        Self {
            compares4: Compares::new(4),
            compares8: Compares::new(8),
            values: ValueMap::new(),
            value_profile: false,
        }
    }
}

/// A cell the instrumentation may write through from the one thread that
/// runs the target.
struct Shared(UnsafeCell<Trace>);

// SAFETY: the engine runs the target on the thread it was started on and
// on no other, and the callbacks below are the only writers. The type is
// `Sync` so that it can be a `static`; it is never actually shared.
unsafe impl Sync for Shared {}

/// The trace of the run in progress.
static TRACE: Shared = Shared(UnsafeCell::new(Trace::new()));

/// Whether the target is running. Outside a run the callbacks return
/// before they touch [`TRACE`], which keeps the mutator's own comparisons
/// out of the table and leaves the engine free to hold a reference to the
/// trace while no callback can reach it.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// The trace, for the engine, between runs.
///
/// The engine calls this only while the target is not running, which is
/// when the callbacks below return before they reach the trace. That is
/// what makes the borrow this hands out the only one alive.
pub fn with_trace<R>(body: impl FnOnce(&mut Trace) -> R) -> R {
    // SAFETY: the target is not running, so no callback can reach the
    // cell, and the engine is the only other holder of a reference to it
    // and holds none while it calls this.
    let trace = unsafe { &mut *TRACE.0.get() };
    body(trace)
}

/// Runs `body` with the instrumentation recording.
///
/// The counters are cleared first, so that what they hold afterwards
/// belongs to this run and to no other.
pub fn record<R>(body: impl FnOnce() -> R) -> R {
    counters::clear();
    RECORDING.store(true, Ordering::Relaxed);
    let outcome = body();
    RECORDING.store(false, Ordering::Relaxed);
    outcome
}

/// The trace of the run in progress, or `None` outside a run.
fn recording() -> Option<&'static mut Trace> {
    if !RECORDING.load(Ordering::Relaxed) {
        return None;
    }
    // SAFETY: `RECORDING` is true only between the two stores in `record`,
    // and the engine holds no reference to the cell across that call. The
    // target runs on one thread, so this is the only reference alive.
    Some(unsafe { &mut *TRACE.0.get() })
}

/// What one comparison of the target adds to the trace.
///
/// `constant` is the side the compiler knew, when it knew one. It stands
/// in for the address of the comparison, which libFuzzer reads off the
/// stack and Rust will not hand out: for the comparisons a parser is made
/// of, against a tag, a length limit, or a magic number, the constant
/// tells two comparison sites apart about as well as their addresses would.
fn handle_compare(trace: &mut Trace, width: usize, left: u64, right: u64, constant: Option<u64>) {
    match width {
        4 => trace.compares4.insert(left, right),
        8 => trace.compares8.insert(left, right),
        _ => {}
    }
    if !trace.value_profile {
        return;
    }
    let difference = left ^ right;
    let hamming = u64::from(difference.count_ones());
    let absolute = if left == right {
        0
    } else {
        u64::from(left.wrapping_sub(right).leading_zeros()).saturating_add(1)
    };
    let site = constant.unwrap_or(0).wrapping_mul(128);
    let _ = trace.values.add(site.wrapping_add(hamming));
    let _ = trace
        .values
        .add(site.wrapping_add(64).wrapping_add(absolute));
}

/// Notes a comparison of `width` bytes whose sides the compiler did not
/// know.
fn compare(width: usize, left: u64, right: u64) {
    if let Some(trace) = recording() {
        handle_compare(trace, width, left, right, None);
    }
}

/// Notes a comparison of `width` bytes whose left side the compiler knew.
fn constant_compare(width: usize, left: u64, right: u64) {
    if let Some(trace) = recording() {
        handle_compare(trace, width, left, right, Some(left));
    }
}

/// Registers a range of counters. Called once per object file, before
/// `main`.
///
/// # Safety
///
/// The compiler passes the bounds of one array of bytes that the linker
/// placed and that lives as long as the program.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_8bit_counters_init(start: *mut u8, stop: *mut u8) {
    // SAFETY: the caller is the compiler's constructor, which passes the
    // bounds of the counter array of its own object file.
    unsafe { counters::register(start, stop) }
}

/// Registers a range of the program table, which describes one block of
/// the target per counter. Called once per object file, before `main`.
///
/// The engine reads nothing out of the table; it counts the blocks so that
/// it can say how much of the target it has reached.
///
/// # Safety
///
/// The compiler passes the bounds of one array of pairs of words.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_pcs_init(beg: *const usize, end: *const usize) {
    let (first, last) = (beg.expose_provenance(), end.expose_provenance());
    let bytes_per_entry = core::mem::size_of::<usize>().saturating_mul(2);
    let entries = last
        .saturating_sub(first)
        .checked_div(bytes_per_entry)
        .unwrap_or(0);
    counters::add_blocks(entries);
}

/// Notes a comparison of two bytes.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_cmp1(left: u8, right: u8) {
    compare(1, u64::from(left), u64::from(right));
}

/// Notes a comparison of two two-byte values.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_cmp2(left: u16, right: u16) {
    compare(2, u64::from(left), u64::from(right));
}

/// Notes a comparison of two four-byte values.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_cmp4(left: u32, right: u32) {
    compare(4, u64::from(left), u64::from(right));
}

/// Notes a comparison of two eight-byte values.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_cmp8(left: u64, right: u64) {
    compare(8, left, right);
}

/// Notes a comparison of a byte with one the compiler knew.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_const_cmp1(left: u8, right: u8) {
    constant_compare(1, u64::from(left), u64::from(right));
}

/// Notes a comparison of a two-byte value with one the compiler knew.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_const_cmp2(left: u16, right: u16) {
    constant_compare(2, u64::from(left), u64::from(right));
}

/// Notes a comparison of a four-byte value with one the compiler knew.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_const_cmp4(left: u32, right: u32) {
    constant_compare(4, u64::from(left), u64::from(right));
}

/// Notes a comparison of an eight-byte value with one the compiler knew.
#[unsafe(no_mangle)]
pub extern "C" fn __sanitizer_cov_trace_const_cmp8(left: u64, right: u64) {
    constant_compare(8, left, right);
}

/// Notes a `match` over a wide value.
///
/// The table is the compiler's: its first word is the number of arms, its
/// second the width of the value in bits, and the rest are the values of
/// the arms in ascending order. The two arms the value falls between are
/// what the mutator can use; arms that are all small carry no signal worth
/// the room in the table, which is libFuzzer's judgement and is kept here.
///
/// # Safety
///
/// `cases` must point at such a table, which the compiler emitted into
/// read-only memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_trace_switch(value: u64, cases: *const u64) {
    if !RECORDING.load(Ordering::Relaxed) || cases.is_null() {
        return;
    }
    // SAFETY: the compiler passes a table of at least two words, followed
    // by as many words as the first of them says.
    let header = unsafe { core::slice::from_raw_parts(cases, 2) };
    let (Some(count), Some(bits)) = (header.first().copied(), header.get(1).copied()) else {
        return;
    };
    let Ok(count) = usize::try_from(count) else {
        return;
    };
    if count == 0 {
        return;
    }
    // SAFETY: as above, the table holds `count` values after its header.
    let arms = unsafe { core::slice::from_raw_parts(cases.wrapping_add(2), count) };
    if arms.last().copied().unwrap_or(0) < 256 || value < 256 {
        return;
    }
    let mut smaller = 0u64;
    let mut larger = u64::MAX;
    for arm in arms {
        if value < *arm {
            larger = *arm;
            break;
        }
        if value > *arm {
            smaller = *arm;
        }
    }
    let width = if bits == 32 { 4 } else { 8 };
    constant_compare(width, smaller, value);
    constant_compare(width, larger, value);
}

/// Notes an indirect call. The engine keeps nothing from it: the counters
/// already say which blocks a run reached, and the pairs of caller and
/// callee that libFuzzer builds out of this have not been worth their cost
/// for the targets of this project.
#[unsafe(no_mangle)]
pub const extern "C" fn __sanitizer_cov_trace_pc_indir(_callee: usize) {}
