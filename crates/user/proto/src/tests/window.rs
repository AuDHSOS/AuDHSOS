// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::window`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::{Error, Handle};
use gfx::draw::{Command, LIST_CAPACITY, List, Text};
use gfx::{Color, Rect};

use crate::input::KeyCode;
use crate::label::{Label, ProtoError, Protocol};
use crate::window::{
    CLOSE, DRAW, Event, MAX_TITLE, OPEN, POLL, Reply, Request, Title, Window, color_of, color_word,
};

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

/// A list holding one of every command.
fn every_command() -> List {
    let mut list = List::new();
    list.push(Command::Clear {
        color: Color::new(1, 2, 3),
    });
    list.push(Command::Fill {
        rect: Rect::new(4, 5, 6, 7),
        color: Color::new(8, 9, 10),
    });
    list.push(Command::Frame {
        rect: Rect::new(11, 12, 13, 14),
        color: Color::WHITE,
    });
    list.push(Command::Line {
        from: (1, 2),
        to: (30, 40),
        color: Color::new(0xF0, 0x0F, 0x55),
    });
    list.push(Command::Text {
        x: 16,
        y: 32,
        color: Color::WHITE,
        background: Some(Color::new(0x10, 0x20, 0x30)),
        text: Text::new("audhsos $ ").unwrap(),
    });
    list.push(Command::Text {
        x: 0,
        y: 0,
        color: Color::BLACK,
        background: None,
        text: Text::empty(),
    });
    list
}

#[test]
fn every_request_comes_back_as_it_was_sent() {
    let requests = [
        Request::Open {
            width: 640,
            height: 400,
            title: Title::new(b"shell").unwrap(),
            notification: handle(3),
            process: handle(4),
        },
        Request::Draw {
            id: 2,
            commands: every_command(),
        },
        Request::Close { id: 5 },
        Request::Poll { id: 6 },
    ];
    for request in requests {
        assert_eq!(round_trip(&request).unwrap(), request);
    }
}

#[test]
fn a_draw_of_a_full_list_fits_one_message() {
    let mut commands = List::new();
    for index in 0..LIST_CAPACITY {
        commands.push(Command::Text {
            x: u32::try_from(index).unwrap(),
            y: 0,
            color: Color::WHITE,
            background: None,
            text: Text::new("0123456789012345678901234567890123456789").unwrap(),
        });
    }
    let request = Request::Draw { id: 1, commands };
    assert_eq!(round_trip(&request).unwrap(), request);
}

#[test]
fn a_draw_of_no_command_is_a_draw_of_no_command() {
    let request = Request::Draw {
        id: 1,
        commands: List::new(),
    };
    assert_eq!(round_trip(&request).unwrap(), request);
}

#[test]
fn every_reply_comes_back_as_it_was_sent() {
    let replies = [
        Reply::Opened(Ok(Window {
            id: 1,
            width: 640,
            height: 400,
        })),
        Reply::Opened(Err(Error::QuotaExceeded)),
        Reply::Drawn(Ok(())),
        Reply::Drawn(Err(Error::AccessDenied)),
        Reply::Closed(Ok(())),
        Reply::Closed(Err(Error::NotFound)),
        Reply::Polled(Ok(None)),
        Reply::Polled(Ok(Some(Event::Key {
            code: KeyCode::A,
            pressed: true,
        }))),
        Reply::Polled(Ok(Some(Event::Key {
            code: KeyCode::Enter,
            pressed: false,
        }))),
        Reply::Polled(Ok(Some(Event::Pointer {
            x: 12,
            y: 34,
            buttons: 1,
        }))),
        Reply::Polled(Ok(Some(Event::Focus { has: true }))),
        Reply::Polled(Ok(Some(Event::Focus { has: false }))),
        Reply::Polled(Ok(Some(Event::Closed))),
        Reply::Polled(Err(Error::NotFound)),
    ];
    for reply in replies {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply);
    }
}

#[test]
fn a_color_survives_the_word_it_travels_in() {
    for color in [
        Color::BLACK,
        Color::WHITE,
        Color::new(0x12, 0x34, 0x56),
        Color::new(0xFF, 0, 0),
    ] {
        assert_eq!(color_of(color_word(color)), color);
    }
}

#[test]
fn a_title_longer_than_its_field_is_refused() {
    let long = [b'x'; MAX_TITLE + 1];
    assert!(matches!(Title::new(&long), Err(ProtoError::TooLong { .. })));
}

#[test]
fn a_message_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    let request = crate::name::Request::Lookup {
        name: crate::name::Name::new(b"desk").unwrap(),
    };
    request.encode(&mut BufferMut::new(&mut bytes)).unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::WrongProtocol {
            expected: Protocol::Window,
            found: Protocol::Name,
        })
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::WrongProtocol {
            expected: Protocol::Window,
            found: Protocol::Name,
        })
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut buffer, 0).unwrap();
    writer
        .finish(&mut buffer, Label::new(Protocol::Window, 99).raw())
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Window, 99))
    );
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Window, 99))
    );
}

#[test]
fn a_command_kind_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut buffer, 1).unwrap();
    writer.word(&mut buffer, 1).unwrap();
    writer.word(&mut buffer, 99).unwrap();
    writer
        .finish(&mut buffer, Label::new(Protocol::Window, DRAW).raw())
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Window, DRAW))
    );
}

#[test]
fn more_commands_than_a_list_holds_are_refused() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut buffer, 1).unwrap();
    writer
        .word(&mut buffer, u64::try_from(LIST_CAPACITY + 1).unwrap())
        .unwrap();
    writer
        .finish(&mut buffer, Label::new(Protocol::Window, DRAW).raw())
        .unwrap();
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Window, DRAW))
    );
}

#[test]
fn an_event_kind_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut buffer, 0).unwrap();
    writer.word(&mut buffer, 99).unwrap();
    writer
        .finish(&mut buffer, Label::new(Protocol::Window, POLL).raw())
        .unwrap();
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Window, POLL))
    );
}

#[test]
fn a_key_code_this_system_does_not_have_is_refused() {
    let mut bytes = buffer();
    let mut buffer = BufferMut::new(&mut bytes);
    let mut writer = user_rt::message::Writer::new();
    writer.word(&mut buffer, 0).unwrap();
    writer.word(&mut buffer, 1).unwrap();
    writer.word(&mut buffer, u64::from(u32::MAX) << 32).unwrap();
    writer
        .finish(&mut buffer, Label::new(Protocol::Window, POLL).raw())
        .unwrap();
    assert_eq!(
        Reply::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Window, POLL))
    );
}

#[test]
fn a_request_and_the_reply_that_answers_it_carry_the_same_label() {
    assert_eq!(
        Request::Open {
            width: 1,
            height: 1,
            title: Title::empty(),
            notification: handle(1),
            process: handle(2),
        }
        .label(),
        Reply::Opened(Err(Error::NotFound)).label()
    );
    assert_eq!(
        Request::Draw {
            id: 1,
            commands: List::new(),
        }
        .label(),
        Reply::Drawn(Ok(())).label()
    );
    assert_eq!(
        Request::Close { id: 1 }.label(),
        Reply::Closed(Ok(())).label()
    );
    assert_eq!(
        Request::Poll { id: 1 }.label(),
        Reply::Polled(Ok(None)).label()
    );
    assert_eq!(
        Request::Open {
            width: 1,
            height: 1,
            title: Title::empty(),
            notification: handle(1),
            process: handle(2),
        }
        .label(),
        Label::new(Protocol::Window, OPEN)
    );
    assert_eq!(
        Request::Close { id: 1 }.label(),
        Label::new(Protocol::Window, CLOSE)
    );
}
