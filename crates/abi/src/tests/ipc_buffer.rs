// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::ipc_buffer`.

use crate::ipc_buffer::{
    ARGS, Buffer, BufferMut, FAULT_LABEL_BASE, HANDLE_COUNT, HANDLES, KERNEL_LABEL_BASE, LABEL,
    MessageError, RETURN, SIZE, STATUS, SYSCALL_NUMBER, Status, WORD, WORD_COUNT, WORDS,
    argument_offset, fault_kind_of, fault_label, handle_offset, is_kernel_label, return_offset,
    word_offset,
};
use crate::layout::{
    MAX_MESSAGE_HANDLES, MAX_MESSAGE_WORDS, MAX_SYSCALL_ARGUMENTS, MAX_SYSCALL_RETURN_WORDS,
};
use crate::{Error, FaultKind, Handle};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
}

/// Reads the word at `offset` from the raw bytes, so that a test can check
/// the offset itself and not only what the accessors agree on.
fn raw_word(bytes: &[u8; SIZE], offset: usize) -> u64 {
    let chunk = bytes.get(offset..).unwrap().first_chunk::<WORD>().unwrap();
    u64::from_le_bytes(*chunk)
}

/// Writes `value` at `offset` in the raw bytes, past every accessor.
fn write_raw(bytes: &mut [u8; SIZE], offset: usize, value: u64) {
    let end = offset.checked_add(WORD).unwrap();
    bytes
        .get_mut(offset..end)
        .unwrap()
        .copy_from_slice(&value.to_le_bytes());
}

#[test]
fn the_offsets_are_the_ones_the_interface_documents() {
    assert_eq!(SYSCALL_NUMBER, 0);
    assert_eq!(ARGS, 8);
    assert_eq!(STATUS, 56);
    assert_eq!(RETURN, 64);
    assert_eq!(LABEL, 128);
    assert_eq!(WORD_COUNT, 136);
    assert_eq!(HANDLE_COUNT, 144);
    assert_eq!(HANDLES, 152);
    assert_eq!(WORDS, 184);
    assert_eq!(SIZE, 4096);
}

#[test]
fn every_area_ends_inside_the_page() {
    let areas = [
        argument_offset(MAX_SYSCALL_ARGUMENTS - 1),
        return_offset(MAX_SYSCALL_RETURN_WORDS - 1),
        handle_offset(MAX_MESSAGE_HANDLES - 1),
        word_offset(MAX_MESSAGE_WORDS - 1),
    ];
    for offset in areas {
        let offset = offset.unwrap();
        assert!(offset.checked_add(WORD).unwrap() <= SIZE, "{offset}");
    }
    assert_eq!(word_offset(MAX_MESSAGE_WORDS - 1), Some(4016));
}

#[test]
fn an_index_beyond_an_area_has_no_offset() {
    assert_eq!(argument_offset(MAX_SYSCALL_ARGUMENTS), None);
    assert_eq!(return_offset(MAX_SYSCALL_RETURN_WORDS), None);
    assert_eq!(handle_offset(MAX_MESSAGE_HANDLES), None);
    assert_eq!(word_offset(MAX_MESSAGE_WORDS), None);
    assert_eq!(word_offset(usize::MAX), None);
    assert_eq!(argument_offset(usize::MAX), None);
    assert_eq!(handle_offset(usize::MAX), None);
    assert_eq!(return_offset(usize::MAX), None);
}

#[test]
fn a_written_field_reads_back_at_its_offset() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_syscall_number(41);
    assert!(writer.set_argument(0, 0x1122_3344_5566_7788));
    assert!(writer.set_argument(MAX_SYSCALL_ARGUMENTS - 1, u64::MAX));
    writer.set_status(Status::failed(Error::AccessDenied));
    assert!(writer.set_return_word(0, 7));
    assert!(writer.set_return_word(MAX_SYSCALL_RETURN_WORDS - 1, 8));
    writer.set_label(0xFEED);
    writer.set_counts(2, 1).unwrap();
    assert!(writer.set_word(0, 0xAA));
    assert!(writer.set_word(MAX_MESSAGE_WORDS - 1, 0xBB));

    assert_eq!(raw_word(&bytes, SYSCALL_NUMBER), 41);
    assert_eq!(raw_word(&bytes, ARGS), 0x1122_3344_5566_7788);
    assert_eq!(
        raw_word(&bytes, STATUS),
        u64::from(Error::AccessDenied.code())
    );
    assert_eq!(raw_word(&bytes, RETURN), 7);
    assert_eq!(raw_word(&bytes, LABEL), 0xFEED);
    assert_eq!(raw_word(&bytes, WORD_COUNT), 2);
    assert_eq!(raw_word(&bytes, HANDLE_COUNT), 1);
    assert_eq!(raw_word(&bytes, 4016), 0xBB);

    let view = Buffer::new(&bytes);
    assert_eq!(view.syscall_number(), 41);
    assert_eq!(view.argument(0), Some(0x1122_3344_5566_7788));
    assert_eq!(view.argument(MAX_SYSCALL_ARGUMENTS - 1), Some(u64::MAX));
    assert_eq!(view.status(), Ok(Status::failed(Error::AccessDenied)));
    assert_eq!(view.return_word(0), Some(7));
    assert_eq!(view.return_word(MAX_SYSCALL_RETURN_WORDS - 1), Some(8));
    assert_eq!(view.word(0), Some(0xAA));
    assert_eq!(view.word(MAX_MESSAGE_WORDS - 1), Some(0xBB));
}

#[test]
fn a_write_beyond_an_area_changes_nothing() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    assert!(!writer.set_argument(MAX_SYSCALL_ARGUMENTS, 1));
    assert!(!writer.set_return_word(MAX_SYSCALL_RETURN_WORDS, 1));
    assert!(!writer.set_word(MAX_MESSAGE_WORDS, 1));
    assert!(!writer.set_handle_word(MAX_MESSAGE_HANDLES, 1));
    assert!(!writer.set_word(usize::MAX, 1));
    assert_eq!(bytes, buffer(), "no byte of the page was touched");
}

#[test]
fn a_read_beyond_an_area_is_none() {
    let bytes = buffer();
    let view = Buffer::new(&bytes);
    assert_eq!(view.argument(MAX_SYSCALL_ARGUMENTS), None);
    assert_eq!(view.return_word(MAX_SYSCALL_RETURN_WORDS), None);
    assert_eq!(view.word(MAX_MESSAGE_WORDS), None);
    assert_eq!(view.handle_word(MAX_MESSAGE_HANDLES), None);
    assert_eq!(view.handle(MAX_MESSAGE_HANDLES), None);
    assert_eq!(view.word(usize::MAX), None);
}

#[test]
fn arguments_reads_every_slot_in_order() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    for index in 0..MAX_SYSCALL_ARGUMENTS {
        let value = u64::try_from(index).unwrap().checked_add(100).unwrap();
        assert!(writer.set_argument(index, value));
    }
    let view = Buffer::new(&bytes);
    let args = view.arguments();
    assert_eq!(args.len(), MAX_SYSCALL_ARGUMENTS);
    for (index, value) in args.iter().enumerate() {
        let expected = u64::try_from(index).unwrap().checked_add(100).unwrap();
        assert_eq!(*value, expected);
    }
}

#[test]
fn a_zero_buffer_reads_as_zero_everywhere() {
    let bytes = buffer();
    let view = Buffer::new(&bytes);
    assert_eq!(view.syscall_number(), 0);
    assert_eq!(view.argument(0), Some(0));
    assert_eq!(view.status(), Ok(Status::OK));
    assert_eq!(view.return_word(0), Some(0));
    assert_eq!(view.word(0), Some(0));
    assert_eq!(view.handle(0), None, "a zero word is no handle");
    let message = view.message().unwrap();
    assert_eq!(message.label, 0);
    assert_eq!(message.word_count, 0);
    assert_eq!(message.handle_count, 0);
}

#[test]
fn a_handle_round_trips_through_the_message_area() {
    let mut bytes = buffer();
    let handle = Handle::new(7, 3).unwrap();
    let mut writer = BufferMut::new(&mut bytes);
    assert!(writer.set_handle(0, handle));
    assert!(writer.set_handle(MAX_MESSAGE_HANDLES - 1, handle));
    let view = Buffer::new(&bytes);
    assert_eq!(view.handle(0), Some(handle));
    assert_eq!(view.handle(MAX_MESSAGE_HANDLES - 1), Some(handle));
    assert_eq!(view.handle_word(0), Some(handle.raw()));
}

#[test]
fn a_handle_word_that_names_no_handle_is_none() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    // Generation zero: an index without a generation is not a handle.
    assert!(writer.set_handle_word(0, 5));
    let view = Buffer::new(&bytes);
    assert_eq!(view.handle_word(0), Some(5));
    assert_eq!(view.handle(0), None);
}

#[test]
fn the_message_counts_are_checked_against_the_area() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    writer
        .set_counts(MAX_MESSAGE_WORDS, MAX_MESSAGE_HANDLES)
        .unwrap();
    let view = Buffer::new(&bytes);
    let message = view.message().unwrap();
    assert_eq!(message.word_count, MAX_MESSAGE_WORDS);
    assert_eq!(message.handle_count, MAX_MESSAGE_HANDLES);
}

#[test]
fn a_count_above_the_area_is_rejected_and_writes_nothing() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    assert_eq!(
        writer.set_counts(MAX_MESSAGE_WORDS + 1, 0),
        Err(MessageError::TooManyWords)
    );
    assert_eq!(
        writer.set_counts(0, MAX_MESSAGE_HANDLES + 1),
        Err(MessageError::TooManyHandles)
    );
    assert_eq!(bytes, buffer());
}

#[test]
fn a_message_header_with_an_impossible_count_is_rejected() {
    let mut bytes = buffer();
    write_raw(&mut bytes, WORD_COUNT, u64::MAX);
    assert_eq!(
        Buffer::new(&bytes).message(),
        Err(MessageError::TooManyWords)
    );

    let mut bytes = buffer();
    let over = u64::try_from(MAX_MESSAGE_WORDS)
        .unwrap()
        .checked_add(1)
        .unwrap();
    write_raw(&mut bytes, WORD_COUNT, over);
    assert_eq!(
        Buffer::new(&bytes).message(),
        Err(MessageError::TooManyWords)
    );

    let mut bytes = buffer();
    let over = u64::try_from(MAX_MESSAGE_HANDLES)
        .unwrap()
        .checked_add(1)
        .unwrap();
    write_raw(&mut bytes, HANDLE_COUNT, over);
    assert_eq!(
        Buffer::new(&bytes).message(),
        Err(MessageError::TooManyHandles)
    );
}

#[test]
fn a_message_error_is_an_argument_error_to_the_caller() {
    assert_eq!(
        Error::from(MessageError::TooManyWords),
        Error::InvalidArgument
    );
    assert_eq!(
        Error::from(MessageError::TooManyHandles),
        Error::InvalidArgument
    );
}

#[test]
fn the_status_word_carries_success_failure_and_progress() {
    assert!(Status::OK.is_success());
    assert!(!Status::OK.is_partial());
    assert_eq!(Status::OK.error(), None);
    assert_eq!(Status::OK.raw(), 0);

    assert!(Status::PARTIAL.is_success());
    assert!(Status::PARTIAL.is_partial());
    assert_eq!(Status::PARTIAL.error(), None);

    for &error in Error::ALL {
        let status = Status::failed(error);
        assert!(!status.is_success());
        assert!(!status.is_partial());
        assert_eq!(status.error(), Some(error));
        assert_eq!(Status::from_raw(status.raw()), Ok(status));
        assert_eq!(Status::from(error), status);
    }
}

#[test]
fn a_status_word_that_is_no_status_is_rejected() {
    // A bit outside the error code and the partial flag.
    assert_eq!(Status::from_raw(1 << 33), Err(Error::InvalidArgument));
    assert_eq!(Status::from_raw(u64::MAX), Err(Error::InvalidArgument));
    // An error code the table does not name.
    let unknown = u64::from(
        u32::try_from(Error::ALL.len())
            .unwrap()
            .checked_add(1)
            .unwrap(),
    );
    assert_eq!(Status::from_raw(unknown), Err(Error::InvalidArgument));
    // A failure cannot claim partial progress.
    let contradiction = (1 << 32) | u64::from(Error::AccessDenied.code());
    assert_eq!(Status::from_raw(contradiction), Err(Error::InvalidArgument));
}

#[test]
fn the_default_status_is_success() {
    assert_eq!(Status::default(), Status::OK);
}

#[test]
fn clearing_the_result_leaves_the_message_alone() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_status(Status::failed(Error::Busy));
    assert!(writer.set_return_word(0, 1));
    assert!(writer.set_return_word(1, 2));
    writer.set_label(0x1234);
    assert!(writer.set_word(0, 0x5678));
    writer.clear_result();

    let view = writer.reader();
    assert_eq!(view.status(), Ok(Status::OK));
    assert_eq!(view.return_word(0), Some(0));
    assert_eq!(view.return_word(1), Some(0));
    assert_eq!(view.message().unwrap().label, 0x1234);
    assert_eq!(view.word(0), Some(0x5678));
}

#[test]
fn the_reader_of_a_writer_sees_the_same_bytes() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_syscall_number(13);
    assert_eq!(writer.reader().syscall_number(), 13);
    assert_eq!(writer.bytes_mut().len(), SIZE);
    assert_eq!(Buffer::new(&bytes).bytes().len(), SIZE);
}

#[test]
fn the_reserved_labels_are_the_top_two_hundred_and_fifty_six() {
    assert_eq!(KERNEL_LABEL_BASE, u64::MAX - 0xFF);
    assert!(!is_kernel_label(KERNEL_LABEL_BASE - 1));
    assert!(is_kernel_label(KERNEL_LABEL_BASE));
    assert!(is_kernel_label(u64::MAX));
    assert!(!is_kernel_label(0));
}

#[test]
fn every_fault_kind_has_a_label_of_its_own_inside_the_reserved_range() {
    let mut seen = std::collections::HashSet::new();
    for &kind in FaultKind::ALL {
        let label = fault_label(kind);
        assert!(is_kernel_label(label), "{kind:?}");
        assert_eq!(fault_kind_of(label), Some(kind));
        assert!(seen.insert(label), "{kind:?} shares its label");
    }
    // The six kinds occupy the first six labels above the base, so 250 of
    // the 256 are left for the kernel messages of later phases.
    assert_eq!(seen.len(), 6);
    assert_eq!(fault_label(FaultKind::PageFault), FAULT_LABEL_BASE + 1);
    assert_eq!(fault_label(FaultKind::AlignmentCheck), FAULT_LABEL_BASE + 6);
    assert_eq!(fault_kind_of(FAULT_LABEL_BASE), None);
    assert_eq!(fault_kind_of(FAULT_LABEL_BASE + 7), None);
    assert_eq!(fault_kind_of(FAULT_LABEL_BASE - 1), None);
}

#[test]
fn a_message_says_whether_its_label_is_the_kernels() {
    let mut bytes = buffer();
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_label(fault_label(FaultKind::PageFault));
    writer.set_counts(3, 0).unwrap();
    let message = Buffer::new(&bytes).message().unwrap();
    assert!(message.is_kernel_label());
    assert_eq!(message.fault_kind(), Some(FaultKind::PageFault));

    BufferMut::new(&mut bytes).set_label(KERNEL_LABEL_BASE - 1);
    let message = Buffer::new(&bytes).message().unwrap();
    assert!(!message.is_kernel_label());
    assert_eq!(message.fault_kind(), None);
}

#[test]
fn a_label_far_above_the_range_names_no_fault_kind() {
    assert_eq!(fault_kind_of(u64::MAX), None);
}
