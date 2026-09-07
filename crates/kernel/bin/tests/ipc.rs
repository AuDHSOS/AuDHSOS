// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two user processes that meet on an endpoint, and a driver at ring three
//! that waits for an interrupt.
//!
//! The host tests of `kernel-ipc` and `kernel-syscall` reach every path of
//! the rendezvous with a recording double. What they cannot show is that two
//! user threads reach each other at all: that the message a thread wrote
//! into its own buffer arrives in the buffer of the other one, that the
//! handle it carried names the same memory in both processes, that the badge
//! of the capability the sender used is what the receiver reads, and that a
//! hardware interrupt reaches a thread that only ever asked for a bit of a
//! notification. This image shows that.
//!
//! The observations of a program go into a page the kernel shares with it,
//! not into its IPC buffer: a message of four hundred and eighty words fills
//! the message area, and a log kept there would be overwritten by the very
//! message it is about.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::ipc_buffer::{Status, fault_kind_of};
use audhsos_abi::layout::MAX_MESSAGE_WORDS;
use audhsos_abi::{Error, FaultKind, Rights, ThreadState};
use kernel_hal_x86_64::testing;
use kernel_objects::object::AnyObjectId;
use kernel_types::PhysFrame;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The client half of the pair.
static IPC_CLIENT: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/ipc_client.bin"));

/// The server half.
static IPC_SERVER: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/ipc_server.bin"));

/// The driver that waits for an interrupt.
static NOTIFICATION_WAITER: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/notification_waiter.bin"
));

/// The badge the client's capability of the endpoint carries.
const BADGE: u64 = 0x0BAD_6E00;

/// The word the server writes into the memory object both processes map.
const WRITTEN: u64 = 0x1234_5678_9ABC_DEF0;

/// Where both processes map that object.
const MAPPING: u64 = 0x0100_0000;

/// The words of the client's shared page.
const CLIENT_FIRST_STATUS: usize = 0;
const CLIENT_FIRST_REPLY: usize = 1;
const CLIENT_SEEN: usize = 2;
const CLIENT_EMPTY_STATUS: usize = 3;
const CLIENT_WIDEST_STATUS: usize = 4;
const CLIENT_TOO_WIDE_STATUS: usize = 5;
const CLIENT_DONE: usize = 6;

/// The value the client writes into [`CLIENT_DONE`] when it has finished.
const CLIENT_FINISHED: u64 = 0x0C_1DEA;

/// The words of the server's shared page.
const SERVER_HANDLED: usize = 0;
const SERVER_BADGE: usize = 1;
const SERVER_FIRST_COUNT: usize = 2;
const SERVER_FIRST_WORD: usize = 3;
const SERVER_LAST_WORD: usize = 4;
const SERVER_HANDLES: usize = 5;
const SERVER_MAP_STATUS: usize = 6;
const SERVER_SECOND_COUNT: usize = 7;
const SERVER_THIRD_COUNT: usize = 8;
const SERVER_THIRD_LAST: usize = 9;
const SERVER_TRY_STATUS: usize = 10;
const SERVER_DONE: usize = 11;

/// The value the server writes into [`SERVER_DONE`] when it has finished.
const SERVER_FINISHED: u64 = 0x0D0E_5E00;

/// The words of the driver's shared page.
const DRIVER_CREATE: usize = 0;
const DRIVER_BIND: usize = 1;
const DRIVER_PORTS: usize = 2;
const DRIVER_FIRST_WORD: usize = 3;
const DRIVER_SECOND_WORD: usize = 4;
const DRIVER_DONE: usize = 5;

/// The value the driver writes into [`DRIVER_DONE`] when it has finished.
const DRIVER_FINISHED: u64 = 0x0D_21FE;

/// The bit of the notification the interrupt is bound to.
const BOUND_BIT: u64 = 3;

/// The rights of a handle to a process that maps its own memory.
const OWN_PROCESS: Rights = Rights::MANAGE
    .union(Rights::MAP)
    .union(Rights::INSTALL)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// The rights of a handle to a memory object that travels.
const TRAVELLING_MEMORY: Rights = Rights::READ
    .union(Rights::WRITE)
    .union(Rights::MAP)
    .union(Rights::INFO)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// What the pair of programs left behind, once they have run.
struct Meeting {
    client: PhysFrame,
    server: PhysFrame,
}

/// Whether the pair has run; every test of it reads the same two pages.
static MET: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The two pages, once the pair has run.
static PAGES: audhsos_sync::Global<Meeting> = audhsos_sync::Global::new();

/// Builds the two processes, gives each what it needs, and runs them until
/// both have ended. Runs once; every test reads what they left.
fn meet_once() {
    if MET.swap(1, core::sync::atomic::Ordering::SeqCst) == 1 {
        return;
    }
    support::bring_up();
    let endpoint = support::endpoint();
    let memory = support::memory_object(1);

    // The server: a thread waiting on the endpoint, with a handle to its own
    // process so that it can map what the client sends it.
    let mut server = support::create_process(IPC_SERVER);
    let server_page = support::share_page(&server);
    let server_thread = support::add_thread(&mut server, support::DEFAULT_PRIORITY);
    let server_endpoint = support::install(
        server.id,
        AnyObjectId::of(endpoint),
        Rights::RECV | Rights::DUPLICATE | Rights::TRANSFER,
    );
    let server_own = support::install(server.id, AnyObjectId::of(server.id), OWN_PROCESS);
    support::set_buffer_word(server_thread.buffer, 0, server_endpoint.raw());
    support::set_buffer_word(server_thread.buffer, 1, server_own.raw());

    // The client: a badged capability of the same endpoint, and a memory
    // object mapped into its own space so that it can read what the server
    // writes into the copy it sends over.
    let mut client = support::create_process(IPC_CLIENT);
    let client_page = support::share_page(&client);
    let client_thread = support::add_thread(&mut client, support::DEFAULT_PRIORITY);
    let client_endpoint = support::install_badged(
        client.id,
        AnyObjectId::of(endpoint),
        Rights::SEND | Rights::DUPLICATE | Rights::TRANSFER,
        BADGE,
    );
    let client_memory = support::install(client.id, AnyObjectId::of(memory), TRAVELLING_MEMORY);
    support::map_memory(&client, memory, MAPPING);
    support::set_buffer_word(client_thread.buffer, 0, client_endpoint.raw());
    support::set_buffer_word(client_thread.buffer, 1, client_memory.raw());

    support::start(server_thread.thread);
    support::start(client_thread.thread);
    say!("two processes meet on one endpoint");
    support::run_until(|| {
        support::page_word(client_page, CLIENT_DONE) == CLIENT_FINISHED
            && support::page_word(server_page, SERVER_DONE) == SERVER_FINISHED
    });
    say!("both halves finished");

    if PAGES
        .init(Meeting {
            client: client_page,
            server: server_page,
        })
        .is_err()
    {
        testing::fail(format_args!("the pages are already in place"));
    }
}

/// Runs `body` with the two pages the pair left.
fn with_pages(body: impl FnOnce(&Meeting)) {
    meet_once();
    let Ok(pages) = PAGES.borrow(&audhsos_sync::UncontendedToken) else {
        testing::fail(format_args!("the pages are not reachable"));
    };
    body(&pages);
}

/// A call and a reply between two user threads: the server sees the badge and
/// the words, and the client gets the answer back.
#[test_case]
fn a_call_carries_the_badge_and_the_words_and_the_reply_comes_back() {
    with_pages(|pages| {
        let status = support::page_word(pages.client, CLIENT_FIRST_STATUS);
        if status != Status::OK.raw() {
            testing::fail(format_args!("the call answered {status:#x}"));
        }
        let badge = support::page_word(pages.server, SERVER_BADGE);
        if badge != BADGE {
            testing::fail(format_args!(
                "the server saw badge {badge:#x}, not {BADGE:#x}"
            ));
        }
        let count = support::page_word(pages.server, SERVER_FIRST_COUNT);
        let first = support::page_word(pages.server, SERVER_FIRST_WORD);
        let last = support::page_word(pages.server, SERVER_LAST_WORD);
        if (count, first, last) != (4, 1, 4) {
            testing::fail(format_args!(
                "the server saw {count} words, first {first}, last {last}"
            ));
        }
        let reply = support::page_word(pages.client, CLIENT_FIRST_REPLY);
        if reply != 0 {
            testing::fail(format_args!("the client got {reply} back, not 0"));
        }
        say!("the badge, four words, and the answer all arrived");
    });
}

/// The handle travels: the client sends one to a memory object, the server
/// maps it and writes, and the client reads what the server wrote.
#[test_case]
fn a_handle_travels_and_names_the_same_memory_in_both_processes() {
    with_pages(|pages| {
        let handles = support::page_word(pages.server, SERVER_HANDLES);
        if handles != 1 {
            testing::fail(format_args!("the server saw {handles} handles, not one"));
        }
        let status = support::page_word(pages.server, SERVER_MAP_STATUS);
        if status != Status::OK.raw() {
            testing::fail(format_args!(
                "the server could not map what it was sent: {status:#x}"
            ));
        }
        let seen = support::page_word(pages.client, CLIENT_SEEN);
        if seen != WRITTEN {
            testing::fail(format_args!(
                "the client read {seen:#x} through its own mapping, not {WRITTEN:#x}"
            ));
        }
        say!("the handle named the same memory in both processes");
    });
}

/// The three widths the catalog asks for: no words, the widest payload, and
/// one word more, which is refused before anything is copied.
#[test_case]
fn a_message_of_no_words_of_the_widest_payload_and_of_one_word_too_many() {
    with_pages(|pages| {
        let empty = support::page_word(pages.client, CLIENT_EMPTY_STATUS);
        let widest = support::page_word(pages.client, CLIENT_WIDEST_STATUS);
        let refused = support::page_word(pages.client, CLIENT_TOO_WIDE_STATUS);
        if empty != Status::OK.raw() || widest != Status::OK.raw() {
            testing::fail(format_args!(
                "the empty message answered {empty:#x} and the widest {widest:#x}"
            ));
        }
        if refused != Status::failed(Error::InvalidArgument).raw() {
            testing::fail(format_args!(
                "one word too many answered {refused:#x}, not an invalid argument"
            ));
        }
        let second = support::page_word(pages.server, SERVER_SECOND_COUNT);
        let third = support::page_word(pages.server, SERVER_THIRD_COUNT);
        let last = support::page_word(pages.server, SERVER_THIRD_LAST);
        let widest_count = u64::try_from(MAX_MESSAGE_WORDS).unwrap_or(0);
        if second != 0 || third != widest_count || last != widest_count {
            testing::fail(format_args!(
                "the server saw {second} words, then {third}, whose last is {last}"
            ));
        }
        say!("no words, {widest_count} words, and one word too many");
    });
}

/// A receive that refuses to wait answers `WouldBlock` when nobody is
/// sending.
#[test_case]
fn a_receive_that_refuses_to_wait_says_so_on_an_empty_endpoint() {
    with_pages(|pages| {
        let status = support::page_word(pages.server, SERVER_TRY_STATUS);
        if status != Status::failed(Error::WouldBlock).raw() {
            testing::fail(format_args!(
                "the receive that refuses to wait answered {status:#x}"
            ));
        }
        let handled = support::page_word(pages.server, SERVER_HANDLED);
        if handled != 3 {
            testing::fail(format_args!(
                "the server answered {handled} messages, not 3"
            ));
        }
        say!("an empty endpoint answered a receive that refuses to wait");
    });
}

/// A sender killed while it waits leaves the queue of the endpoint, and a
/// later receive does not see it.
#[test_case]
fn a_sender_killed_while_it_waits_leaves_the_queue_of_the_endpoint() {
    support::bring_up();
    let endpoint = support::endpoint();
    let memory = support::memory_object(1);
    let mut client = support::create_process(IPC_CLIENT);
    let page = support::share_page(&client);
    let spawned = support::add_thread(&mut client, support::DEFAULT_PRIORITY);
    let handle = support::install_badged(
        client.id,
        AnyObjectId::of(endpoint),
        Rights::SEND | Rights::DUPLICATE | Rights::TRANSFER,
        BADGE,
    );
    let carried = support::install(client.id, AnyObjectId::of(memory), TRAVELLING_MEMORY);
    support::map_memory(&client, memory, MAPPING);
    support::set_buffer_word(spawned.buffer, 0, handle.raw());
    support::set_buffer_word(spawned.buffer, 1, carried.raw());
    support::start(spawned.thread);
    support::run_threads(None);

    if support::state_of(spawned.thread) != Some(ThreadState::BlockedSend) {
        testing::fail(format_args!(
            "the client is {:?}, not waiting for a receiver",
            support::state_of(spawned.thread)
        ));
    }
    if senders_of(endpoint) != 1 {
        testing::fail(format_args!("the endpoint holds no sender"));
    }
    support::kill(spawned.thread);
    if senders_of(endpoint) != 0 {
        testing::fail(format_args!("the endpoint still names the dead sender"));
    }
    let _ = page;
    say!("a sender that was killed is out of the queue it waited in");
}

/// An endpoint destroyed under a waiter wakes it with `ObjectDestroyed`.
///
/// A sender and a receiver of one endpoint cannot both be waiting at once —
/// whichever arrives second meets the first — so the two queues are shown
/// one at a time, which is what a user thread can reach. The host test of
/// `kernel-ipc` shows both queues of one endpoint together.
#[test_case]
fn an_endpoint_destroyed_under_a_waiter_wakes_it_with_object_destroyed() {
    support::bring_up();
    for receiving in [false, true] {
        let endpoint = support::endpoint();
        let spawned = if receiving {
            waiting_server(endpoint)
        } else {
            waiting_client(endpoint)
        };
        // Every reference goes: the one the creation left and the one the
        // handle of the waiting process holds. A thread that waits on an
        // object holds none of its own (D-75), so the endpoint is destroyed
        // under it.
        for _ in 0..REFERENCES {
            if !endpoint_lives(endpoint) {
                break;
            }
            support::release(AnyObjectId::of(endpoint));
        }
        if endpoint_lives(endpoint) {
            testing::fail(format_args!("the endpoint outlived its last reference"));
        }
        let status = support::thread_status(spawned);
        if status != Status::failed(Error::ObjectDestroyed).raw() {
            testing::fail(format_args!(
                "the {} found {status:#x} in its buffer",
                if receiving { "receiver" } else { "sender" }
            ));
        }
    }
    say!("a waiter of a destroyed endpoint woke with the object gone");
}

/// A driver at ring three: an interrupt object for the interval timer, bound
/// to a bit of a notification, with the timer programmed through a range of
/// I/O ports.
#[test_case]
fn an_interrupt_reaches_a_user_thread_through_a_notification() {
    support::bring_up();
    let mut driver = support::create_process(NOTIFICATION_WAITER);
    let page = support::share_page(&driver);
    let spawned = support::add_thread(&mut driver, support::DEFAULT_PRIORITY);
    let control = support::install(
        driver.id,
        AnyObjectId::of(kernel_objects::object::SystemControl::ID),
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
    );
    support::set_buffer_word(spawned.buffer, 0, control.raw());
    support::start(spawned.thread);
    // The timer starts last, when the whole run is in place, which is the
    // rule `preemption.rs` states for the same reason: from the first tick
    // the scheduler may take the processor away, and the threads the tests
    // above this one left runnable are what it would give it to. A tick
    // that lands while this thread is halfway through making a process or
    // a thread switches away from the image itself, and the image does not
    // come back — the machine then spins in the user thread that got the
    // processor and writes nothing more. Marking the driver runnable
    // starts nothing on its own; the first `idle_until` below is what
    // gives it the processor.
    support::start_timer(|_ticks| false);
    say!("a driver takes the interval timer and waits for its line");
    support::idle_until(|| support::page_word(page, DRIVER_DONE) == DRIVER_FINISHED);

    for (word, what) in [
        (DRIVER_CREATE, "the interrupt object"),
        (DRIVER_BIND, "the binding"),
        (DRIVER_PORTS, "the port range"),
    ] {
        let status = support::page_word(page, word);
        if status != Status::OK.raw() {
            testing::fail(format_args!("{what} answered {status:#x}"));
        }
    }
    let wanted = 1_u64 << BOUND_BIT;
    for (word, which) in [(DRIVER_FIRST_WORD, "first"), (DRIVER_SECOND_WORD, "second")] {
        let bits = support::page_word(page, word);
        if bits != wanted {
            testing::fail(format_args!(
                "the {which} interrupt set {bits:#b}, not {wanted:#b}"
            ));
        }
    }
    say!("both interrupts set exactly the bit the binding named");
}

/// A client of `endpoint` that blocks with nobody receiving.
fn waiting_client(endpoint: kernel_objects::object::EndpointId) -> PhysFrame {
    let memory = support::memory_object(1);
    let mut client = support::create_process(IPC_CLIENT);
    support::share_page(&client);
    let spawned = support::add_thread(&mut client, support::DEFAULT_PRIORITY);
    let handle = support::install_badged(
        client.id,
        AnyObjectId::of(endpoint),
        Rights::SEND | Rights::DUPLICATE | Rights::TRANSFER,
        BADGE,
    );
    let carried = support::install(client.id, AnyObjectId::of(memory), TRAVELLING_MEMORY);
    support::map_memory(&client, memory, MAPPING);
    support::set_buffer_word(spawned.buffer, 0, handle.raw());
    support::set_buffer_word(spawned.buffer, 1, carried.raw());
    support::start(spawned.thread);
    support::run_threads(None);
    expect_blocked(spawned.thread, ThreadState::BlockedSend);
    spawned.buffer
}

/// A server on `endpoint` that blocks with nobody sending.
fn waiting_server(endpoint: kernel_objects::object::EndpointId) -> PhysFrame {
    let mut server = support::create_process(IPC_SERVER);
    support::share_page(&server);
    let spawned = support::add_thread(&mut server, support::DEFAULT_PRIORITY);
    let handle = support::install(
        server.id,
        AnyObjectId::of(endpoint),
        Rights::RECV | Rights::DUPLICATE | Rights::TRANSFER,
    );
    let own = support::install(server.id, AnyObjectId::of(server.id), OWN_PROCESS);
    support::set_buffer_word(spawned.buffer, 0, handle.raw());
    support::set_buffer_word(spawned.buffer, 1, own.raw());
    support::start(spawned.thread);
    support::run_threads(None);
    expect_blocked(spawned.thread, ThreadState::BlockedRecv);
    spawned.buffer
}

/// Fails the image when `thread` is not waiting the way it should be.
fn expect_blocked(thread: kernel_objects::object::ThreadId, wanted: ThreadState) {
    let state = support::state_of(thread);
    if state != Some(wanted) {
        testing::fail(format_args!("the thread is {state:?}, not {wanted:?}"));
    }
}

/// How many references an endpoint of these tests can hold: the one its
/// creation leaves and one per handle a process holds to it.
const REFERENCES: usize = 4;

/// `true` while the machine still holds `endpoint`.
fn endpoint_lives(endpoint: kernel_objects::object::EndpointId) -> bool {
    kernel_core::machine::with_machine(|machine| machine.objects.endpoints.get(endpoint).is_ok())
        .unwrap_or(false)
}

/// How many threads wait to send on `endpoint`.
fn senders_of(endpoint: kernel_objects::object::EndpointId) -> u32 {
    kernel_core::machine::with_machine(|machine| {
        machine
            .objects
            .endpoints
            .get(endpoint)
            .map_or(0, |held| held.senders.len())
    })
    .unwrap_or(0)
}

/// The label and the kind of a fault message, which `isolation.rs` reads the
/// same way; here it keeps the two helpers of the interface in use so that a
/// change to either is caught by this image as well.
#[test_case]
fn a_fault_label_names_its_kind() {
    let label = audhsos_abi::ipc_buffer::fault_label(FaultKind::PageFault);
    if fault_kind_of(label) != Some(FaultKind::PageFault) {
        testing::fail(format_args!("the label {label:#x} names no page fault"));
    }
    if !audhsos_abi::ipc_buffer::is_kernel_label(label) {
        testing::fail(format_args!("the label {label:#x} is not the kernel's"));
    }
    say!("a fault label names the kind it carries");
}
