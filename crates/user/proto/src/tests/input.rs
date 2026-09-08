// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::input`.

use crate::input::RingPage;
use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::{Error, Handle};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::input::{
    EVENT_LEN, Event, KIND_KEY, KIND_POINTER, KeyCode, KeyEvent, PointerEvent, RING_CAPACITY,
    RING_HEADER_LEN, RING_PAGE_LEN, Reply, Request, RingReader, RingWriter, SUBSCRIBE, UNSUBSCRIBE,
    capacity_for,
};
use crate::label::{Label, ProtoError, Protocol};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
}

/// A handle every table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// Encodes `request` and reads it back.
fn round_trip(request: Request) -> Result<Request, ProtoError> {
    let mut bytes = buffer();
    request.encode(&mut BufferMut::new(&mut bytes))?;
    Request::decode(Buffer::new(&bytes))
}

/// Encodes `reply` and reads it back.
fn round_trip_reply(reply: Reply) -> Result<Reply, ProtoError> {
    let mut bytes = buffer();
    reply.encode(&mut BufferMut::new(&mut bytes))?;
    Reply::decode(Buffer::new(&bytes))
}

/// A key event of one key.
fn key(code: KeyCode, pressed: bool) -> Event {
    Event::Key(KeyEvent { code, pressed })
}

/// A page for a ring.
fn page() -> RingPage {
    RingPage::new()
}

#[test]
fn every_request_comes_back_as_it_was_sent() {
    let requests = [
        Request::Subscribe {
            notification: handle(9),
            process: handle(10),
        },
        Request::Unsubscribe,
    ];
    for request in requests {
        assert_eq!(round_trip(request), Ok(request));
        assert_eq!(request.label().protocol, Protocol::Input);
    }
}

#[test]
fn every_reply_comes_back_as_it_was_sent() {
    let replies = [
        Reply::Subscribed(Ok(handle(3))),
        Reply::Subscribed(Err(Error::QuotaExceeded)),
        Reply::Unsubscribed(Ok(())),
        Reply::Unsubscribed(Err(Error::NotFound)),
    ];
    for reply in replies {
        assert_eq!(round_trip_reply(reply), Ok(reply));
        assert_eq!(reply.label().protocol, Protocol::Input);
    }
}

#[test]
fn a_message_of_another_protocol_or_another_number_is_refused() {
    let mut bytes = buffer();
    {
        let mut area = BufferMut::new(&mut bytes);
        area.set_label(Label::new(Protocol::Console, SUBSCRIBE).raw());
        area.set_counts(0, 0).unwrap();
    }
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::WrongProtocol {
            expected: Protocol::Input,
            found: Protocol::Console,
        })
    );

    let mut bytes = buffer();
    {
        let mut area = BufferMut::new(&mut bytes);
        area.set_label(Label::new(Protocol::Input, 9).raw());
        area.set_word(0, 0);
        area.set_counts(1, 0).unwrap();
    }
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Input, 9))
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Input, 9))
    );
    assert_eq!(UNSUBSCRIBE, 2);
}

#[test]
fn an_event_record_is_sixteen_bytes_and_comes_back_as_it_was_written() {
    let events = [
        key(KeyCode::A, true),
        key(KeyCode::Pause, false),
        Event::Pointer(PointerEvent {
            dx: -300,
            dy: 17,
            wheel: -1,
            buttons: 0b101,
        }),
        Event::Pointer(PointerEvent::default()),
    ];
    for event in events {
        let record = event.to_bytes();
        assert_eq!(record.len(), EVENT_LEN);
        assert_eq!(Event::from_bytes(&record), Some(event));
    }
    assert_eq!(key(KeyCode::A, true).kind(), KIND_KEY);
    assert_eq!(Event::Pointer(PointerEvent::default()).kind(), KIND_POINTER);
}

#[test]
fn the_bytes_a_record_does_not_use_are_zero_and_are_read_back_as_such() {
    let record = key(KeyCode::Escape, true).to_bytes();
    assert_eq!(record.get(1).copied(), Some(0));
    assert!(
        record
            .get(5..)
            .is_some_and(|rest| rest.iter().all(|byte| *byte == 0)),
        "a key record uses five bytes and leaves the rest at zero"
    );
    let record = Event::Pointer(PointerEvent {
        dx: 1,
        dy: 2,
        wheel: 3,
        buttons: 4,
    })
    .to_bytes();
    assert!(
        record
            .get(8..)
            .is_some_and(|rest| rest.iter().all(|byte| *byte == 0))
    );
}

#[test]
fn a_record_whose_kind_names_neither_is_refused() {
    let mut record = key(KeyCode::A, true).to_bytes();
    if let Some(slot) = record.first_mut() {
        *slot = 0;
    }
    assert_eq!(Event::from_bytes(&record), None, "zero names no kind");
    if let Some(slot) = record.first_mut() {
        *slot = 3;
    }
    assert_eq!(Event::from_bytes(&record), None);

    // The second byte is reserved and has to be zero.
    let mut record = key(KeyCode::A, true).to_bytes();
    if let Some(slot) = record.get_mut(1) {
        *slot = 1;
    }
    assert_eq!(Event::from_bytes(&record), None);

    // A key code no key has is no key event.
    let mut record = key(KeyCode::A, true).to_bytes();
    if let Some(slot) = record.get_mut(2..4) {
        slot.copy_from_slice(&0xFFFFu16.to_le_bytes());
    }
    assert_eq!(Event::from_bytes(&record), None);
}

#[test]
fn a_ring_over_one_page_holds_the_records_that_fit_into_it() {
    assert_eq!(capacity_for(RING_PAGE_LEN), RING_CAPACITY);
    assert_eq!(capacity_for(RING_HEADER_LEN), 0);
    assert_eq!(capacity_for(0), 0);
    assert_eq!(capacity_for(RING_HEADER_LEN + EVENT_LEN), 1);
    let bytes = page();
    let writer = RingWriter::create(&bytes).unwrap();
    assert_eq!(writer.header().capacity, RING_CAPACITY);
    assert_eq!(writer.header().write_seq, 0);
    assert_eq!(writer.header().read_seq, 0);
    assert_eq!(writer.header().overflow, 0);
}

#[test]
fn a_ring_that_holds_no_record_is_no_ring() {
    let nothing = page();
    for capacity in [0, 1, RING_CAPACITY + 1, u32::MAX] {
        nothing.capacity.store(capacity, Ordering::Relaxed);
        assert!(RingWriter::adopt(&nothing).is_none());
        assert!(RingReader::new(&nothing).is_none());
    }
}

#[test]
fn what_the_writer_pushes_the_reader_pops_in_the_order_it_was_pushed() {
    let bytes = page();
    let events = [
        key(KeyCode::A, true),
        key(KeyCode::A, false),
        Event::Pointer(PointerEvent {
            dx: 3,
            dy: -4,
            wheel: 0,
            buttons: 1,
        }),
    ];
    {
        let mut writer = RingWriter::create(&bytes).unwrap();
        for event in events {
            assert!(writer.push(event));
        }
        assert_eq!(writer.header().write_seq, 3);
    }
    let mut reader = RingReader::new(&bytes).unwrap();
    assert!(!reader.is_empty());
    for event in events {
        assert_eq!(reader.pop(), Some(event));
    }
    assert!(reader.is_empty());
    assert_eq!(reader.pop(), None, "and nothing more");
    assert_eq!(reader.header().read_seq, 3);
    assert_eq!(reader.take_overflow(), 0);
}

#[test]
fn a_full_ring_drops_the_newest_event_and_counts_it_and_the_reader_clears_that() {
    let bytes = page();
    let capacity = usize::try_from(RING_CAPACITY).unwrap();
    {
        let mut writer = RingWriter::create(&bytes).unwrap();
        for _ in 0..capacity {
            assert!(writer.push(key(KeyCode::A, true)));
        }
        assert!(
            !writer.push(key(KeyCode::Z, true)),
            "the ring is full and the newest event goes"
        );
        assert!(!writer.push(key(KeyCode::Z, true)));
        assert_eq!(writer.header().overflow, 2);
        assert_eq!(writer.header().write_seq, u64::from(RING_CAPACITY));
    }
    let mut reader = RingReader::new(&bytes).unwrap();
    assert_eq!(reader.take_overflow(), 2);
    assert_eq!(reader.take_overflow(), 0, "a gap is reported once");
    assert_eq!(
        reader.pop(),
        Some(key(KeyCode::A, true)),
        "and the oldest event is still there"
    );
}

#[test]
fn the_records_wrap_and_the_sequence_numbers_stay_contiguous() {
    let bytes = page();
    let capacity = u64::from(RING_CAPACITY);
    RingWriter::create(&bytes).unwrap();
    let mut written = 0u64;
    let mut read = 0u64;
    for round in 0..(capacity.saturating_mul(2)) {
        let code = if round % 2 == 0 {
            KeyCode::A
        } else {
            KeyCode::B
        };
        {
            let mut writer = RingWriter::adopt(&bytes).unwrap();
            assert!(writer.push(key(code, true)));
            written = written.saturating_add(1);
        }
        let mut reader = RingReader::new(&bytes).unwrap();
        assert_eq!(reader.pop(), Some(key(code, true)));
        read = read.saturating_add(1);
        assert_eq!(reader.header().write_seq, written);
        assert_eq!(reader.header().read_seq, read);
    }
}

#[test]
fn a_ring_whose_record_is_nonsense_gives_nothing_and_still_moves_on() {
    let bytes = page();
    {
        let mut writer = RingWriter::create(&bytes).unwrap();
        assert!(writer.push(key(KeyCode::A, true)));
    }
    // Whoever holds the page can write into it, and the reader has to
    // survive that: it is shared memory and not a message.
    if let Some(slot) = bytes.records.first().and_then(|record| record.first()) {
        slot.store(0xFF, Ordering::Relaxed);
    }
    let mut reader = RingReader::new(&bytes).unwrap();
    assert_eq!(reader.pop(), None, "the record names no kind");
    assert_eq!(
        reader.header().read_seq,
        1,
        "and it is stepped over rather than read for ever"
    );
}

#[test]
fn reader_and_writer_run_concurrently_without_torn_or_reordered_records() {
    const EVENTS: i16 = 10_000;
    let page = page();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut writer = RingWriter::adopt(&page).unwrap();
            for index in 0..EVENTS {
                let event = Event::Pointer(PointerEvent {
                    dx: index,
                    dy: -index,
                    wheel: 1,
                    buttons: 3,
                });
                while !writer.push(event) {
                    std::thread::yield_now();
                }
            }
            done.store(true, Ordering::Release);
        });
        let mut reader = RingReader::new(&page).unwrap();
        let mut next = 0;
        while next < EVENTS {
            if let Some(event) = reader.pop() {
                assert_eq!(
                    event,
                    Event::Pointer(PointerEvent {
                        dx: next,
                        dy: -next,
                        wheel: 1,
                        buttons: 3
                    })
                );
                next += 1;
            } else {
                assert!(!done.load(Ordering::Acquire) || !reader.is_empty() || next == EVENTS);
                std::thread::yield_now();
            }
        }
    });
}

#[test]
fn overflow_exchange_does_not_erase_concurrent_increments() {
    let page = page();
    let mut writer = RingWriter::adopt(&page).unwrap();
    for _ in 0..RING_CAPACITY {
        assert!(writer.push(key(KeyCode::A, true)));
    }
    let done = AtomicBool::new(false);
    let mut reported = 0;
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut writer = RingWriter::adopt(&page).unwrap();
            for _ in 0..100_000 {
                assert!(!writer.push(key(KeyCode::B, true)));
            }
            done.store(true, Ordering::Release);
        });
        let mut reader = RingReader::new(&page).unwrap();
        while !done.load(Ordering::Acquire) {
            reported += reader.take_overflow();
        }
        reported += reader.take_overflow();
    });
    assert_eq!(reported, 100_000);
}

#[test]
fn input_requests_reject_missing_extra_and_unexpected_handles_and_words() {
    for request in [
        Request::Unsubscribe,
        Request::Subscribe {
            notification: handle(1),
            process: handle(2),
        },
    ] {
        let expected = if matches!(request, Request::Unsubscribe) {
            0
        } else {
            2
        };
        for count in 0..=4 {
            let mut bytes = buffer();
            let mut writer = BufferMut::new(&mut bytes);
            request.encode(&mut writer).unwrap();
            for index in 0..count {
                writer.set_handle(index, handle(u32::try_from(index).unwrap() + 1));
            }
            writer.set_counts(0, count).unwrap();
            assert_eq!(
                Request::decode(Buffer::new(&bytes)).is_ok(),
                count == expected
            );
        }
        let mut bytes = buffer();
        let mut writer = BufferMut::new(&mut bytes);
        request.encode(&mut writer).unwrap();
        writer.set_counts(1, expected).unwrap();
        assert!(Request::decode(Buffer::new(&bytes)).is_err());
    }
}

#[test]
fn ring_capacity_changes_after_adoption_cannot_escape_the_page() {
    let page = page();
    let mut writer = RingWriter::adopt(&page).unwrap();
    let mut reader = RingReader::new(&page).unwrap();
    assert!(writer.push(key(KeyCode::A, true)));
    page.capacity.store(u32::MAX, Ordering::Relaxed);
    assert!(!writer.push(key(KeyCode::B, true)));
    assert_eq!(reader.pop(), None);
    assert_eq!(reader.take_overflow(), 1);
    page.capacity.store(RING_CAPACITY, Ordering::Relaxed);
    assert_eq!(reader.pop(), Some(key(KeyCode::A, true)));
}
