// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::display`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::{Error, FramebufferFormat, Handle};
use gfx::{DAMAGE_CAPACITY, Damage, Rect};

use crate::display::{
    CREATE_SURFACE, DESTROY_SURFACE, INFO, Mode, PRESENT, Reply, Request, SET_CURSOR, Surface,
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
fn round_trip(request: &Request) -> Result<Request, ProtoError> {
    let mut bytes = buffer();
    request.encode(&mut BufferMut::new(&mut bytes))?;
    Request::decode(Buffer::new(&bytes))
}

/// Encodes `reply` and reads it back.
fn round_trip_reply(reply: &Reply) -> Result<Reply, ProtoError> {
    let mut bytes = buffer();
    reply.encode(&mut BufferMut::new(&mut bytes))?;
    Reply::decode(Buffer::new(&bytes))
}

/// A damage set of `count` rectangles that do not touch.
fn damage(count: usize) -> Damage {
    let mut damage = Damage::new();
    for index in 0..count {
        let step = u32::try_from(index).unwrap().saturating_mul(4);
        damage.push(Rect::new(step, step, 2, 2));
    }
    damage
}

#[test]
fn every_request_comes_back_as_it_was_sent() {
    let requests = [
        Request::Info,
        Request::CreateSurface {
            width: 1280,
            height: 800,
            process: handle(5),
        },
        Request::Present {
            id: 7,
            damage: damage(3),
        },
        Request::DestroySurface { id: 9 },
        Request::SetCursor {
            x: 100,
            y: 200,
            visible: true,
        },
        Request::SetCursor {
            x: 0,
            y: 0,
            visible: false,
        },
    ];
    for request in requests {
        assert_eq!(round_trip(&request), Ok(request), "{request:?}");
    }
}

#[test]
fn every_reply_comes_back_as_it_was_sent() {
    let mode = Mode {
        width: 1280,
        height: 800,
        format: FramebufferFormat::Bgrx8888,
    };
    let surface = Surface {
        id: 3,
        memory: handle(11),
    };
    let replies = [
        Reply::Screen(Ok(mode)),
        Reply::Screen(Err(Error::NotFound)),
        Reply::Created(Ok(surface)),
        Reply::Created(Err(Error::InvalidArgument)),
        Reply::Presented(Ok(())),
        Reply::Presented(Err(Error::AccessDenied)),
        Reply::Destroyed(Ok(())),
        Reply::Destroyed(Err(Error::NotFound)),
        Reply::CursorSet(Ok(())),
        Reply::CursorSet(Err(Error::InvalidArgument)),
    ];
    for reply in replies {
        assert_eq!(round_trip_reply(&reply), Ok(reply), "{reply:?}");
    }
}

#[test]
fn the_widest_fields_survive_the_wire() {
    let request = Request::CreateSurface {
        width: u32::MAX,
        height: u32::MAX,
        process: handle(1),
    };
    assert_eq!(round_trip(&request), Ok(request));
    let request = Request::DestroySurface { id: u32::MAX };
    assert_eq!(round_trip(&request), Ok(request));
    let mut wide = Damage::new();
    wide.push(Rect::new(u32::MAX, u32::MAX, u32::MAX, u32::MAX));
    let request = Request::Present {
        id: 1,
        damage: wide,
    };
    assert_eq!(round_trip(&request), Ok(request));
}

#[test]
fn a_full_damage_set_travels_whole() {
    let full = damage(DAMAGE_CAPACITY);
    assert_eq!(full.len(), DAMAGE_CAPACITY);
    let request = Request::Present {
        id: 2,
        damage: full,
    };
    assert_eq!(round_trip(&request), Ok(request));
}

#[test]
fn a_present_of_nothing_carries_no_rectangle() {
    let request = Request::Present {
        id: 2,
        damage: Damage::new(),
    };
    assert_eq!(round_trip(&request), Ok(request));
}

#[test]
fn more_rectangles_than_a_damage_set_holds_are_refused() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut view, 5).unwrap();
    writer
        .word(
            &mut view,
            u64::try_from(DAMAGE_CAPACITY).unwrap().saturating_add(1),
        )
        .unwrap();
    writer
        .finish(&mut view, Label::new(Protocol::Display, PRESENT).raw())
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Display, PRESENT))
    );
}

#[test]
fn a_surface_number_above_its_field_is_refused() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer
        .word(&mut view, u64::from(u32::MAX).saturating_add(1))
        .unwrap();
    writer
        .finish(
            &mut view,
            Label::new(Protocol::Display, DESTROY_SURFACE).raw(),
        )
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Display, PRESENT))
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut view, 0).unwrap();
    writer
        .finish(&mut view, Label::new(Protocol::Display, 99).raw())
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Display, 99))
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Display, 99))
    );
}

#[test]
fn a_label_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut view, 0).unwrap();
    writer
        .finish(&mut view, Label::new(Protocol::Name, INFO).raw())
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::WrongProtocol {
            expected: Protocol::Display,
            found: Protocol::Name,
        })
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::WrongProtocol {
            expected: Protocol::Display,
            found: Protocol::Name,
        })
    );
}

#[test]
fn a_format_code_that_names_no_format_is_refused() {
    let mut bytes = buffer();
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut view, 0).unwrap();
    writer.word(&mut view, 0).unwrap();
    writer.word(&mut view, 9).unwrap();
    writer
        .finish(&mut view, Label::new(Protocol::Display, INFO).raw())
        .unwrap();
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Display, INFO))
    );
}

#[test]
fn every_request_and_reply_names_its_own_message() {
    assert_eq!(Request::Info.label().message, INFO);
    assert_eq!(
        Request::CreateSurface {
            width: 1,
            height: 1,
            process: handle(1)
        }
        .label()
        .message,
        CREATE_SURFACE
    );
    assert_eq!(
        Request::Present {
            id: 1,
            damage: Damage::new()
        }
        .label()
        .message,
        PRESENT
    );
    assert_eq!(
        Request::DestroySurface { id: 1 }.label().message,
        DESTROY_SURFACE
    );
    assert_eq!(
        Request::SetCursor {
            x: 1,
            y: 1,
            visible: true
        }
        .label()
        .message,
        SET_CURSOR
    );
    assert_eq!(Reply::Screen(Err(Error::NotFound)).label().message, INFO);
    assert_eq!(
        Reply::Created(Err(Error::NotFound)).label().message,
        CREATE_SURFACE
    );
    assert_eq!(Reply::Presented(Ok(())).label().message, PRESENT);
    assert_eq!(Reply::Destroyed(Ok(())).label().message, DESTROY_SURFACE);
    assert_eq!(Reply::CursorSet(Ok(())).label().message, SET_CURSOR);
}
