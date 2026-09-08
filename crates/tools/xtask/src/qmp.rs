// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The QEMU machine protocol, as far as this runner speaks it.
//!
//! One line of JSON per message. The machine greets, the client answers
//! `qmp_capabilities`, and after that every command is a line and every
//! answer is a line carrying either `return` or `error`. Lines that carry
//! `event` are the machine saying something on its own and are stepped
//! over: they arrive between a command and its answer whenever they please.
//!
//! The protocol is a reader and a writer here and a Unix socket only at the
//! edge, so the whole of it is tested on the host over two byte buffers.
//!
//! Invariants: no command is sent before the greeting is answered; an error
//! answer becomes an error value and never a panic; a machine that says
//! nothing runs into the timeout of the socket rather than waiting for
//! ever.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::error::Error;
use crate::json::{self, JsonError, Value};
use crate::ppm::Image;

/// How long a screendump may take before the picture is read.
const SCREENDUMP_SETTLE: Duration = Duration::from_millis(200);

/// Why a command did not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QmpError {
    /// The line is no JSON of the subset.
    Json(JsonError),
    /// The first line was no greeting.
    NoGreeting(String),
    /// The machine answered an error.
    Refused {
        /// The class of it, as QEMU names it.
        class: String,
        /// What it said.
        message: String,
    },
    /// The answer carried neither `return` nor `error`.
    Answer(String),
    /// The connection ended.
    Closed,
}

impl fmt::Display for QmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QmpError::Json(error) => {
                write!(f, "the machine said something that is no JSON: {error}")
            }
            QmpError::NoGreeting(line) => write!(f, "the first line is no greeting: {line}"),
            QmpError::Refused { class, message } => write!(f, "{class}: {message}"),
            QmpError::Answer(line) => write!(f, "the answer says neither return nor error: {line}"),
            QmpError::Closed => f.write_str("the machine closed the connection"),
        }
    }
}

impl From<QmpError> for Error {
    fn from(error: QmpError) -> Self {
        Error::Parse(format!("{error}"))
    }
}

impl From<JsonError> for QmpError {
    fn from(error: JsonError) -> Self {
        QmpError::Json(error)
    }
}

/// The protocol over any pair of a reader and a writer.
pub(crate) struct Session<R: BufRead, W: Write> {
    /// Where the answers come from.
    reader: R,
    /// Where the commands go.
    writer: W,
}

impl<R: BufRead, W: Write> Session<R, W> {
    /// Reads the greeting and negotiates the capabilities, which is what
    /// every QMP connection begins with.
    ///
    /// # Errors
    ///
    /// [`QmpError`] when the first line is no greeting or the negotiation
    /// is refused, and the I/O errors of the two ends.
    pub(crate) fn start(reader: R, writer: W) -> Result<Self, Error> {
        let mut session = Session { reader, writer };
        let line = session.line()?;
        let greeting = json::parse(&line).map_err(QmpError::from)?;
        if greeting.get("QMP").is_none() {
            return Err(QmpError::NoGreeting(line).into());
        }
        session.execute("qmp_capabilities", Value::object([]))?;
        Ok(session)
    }

    /// Sends one command and answers with what it returned.
    ///
    /// # Errors
    ///
    /// [`QmpError::Refused`] for a command the machine refused, the other
    /// [`QmpError`] variants for an answer that is none, and the I/O errors
    /// of the two ends.
    pub(crate) fn execute(&mut self, command: &str, arguments: Value) -> Result<Value, Error> {
        let mut message = vec![("execute".to_owned(), Value::Text(command.to_owned()))];
        if arguments != Value::object([]) {
            message.push(("arguments".to_owned(), arguments));
        }
        let line = Value::object(message).write();
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .and_then(|()| self.writer.flush())
            .map_err(|source| Error::io(format!("sending `{command}` to the machine"), source))?;
        self.answer(command)
    }

    /// Presses or releases the key QEMU names `qcode`.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn send_key(&mut self, qcode: &str, pressed: bool) -> Result<(), Error> {
        self.input_send_event(vec![key_event(qcode, pressed)])
    }

    /// Moves the pointer by `dx` and `dy`.
    ///
    /// Both go out in one command, so the mouse packetizes the motion once
    /// and a test that injects a path sees one event per step and not two.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn move_pointer(&mut self, dx: i32, dy: i32) -> Result<(), Error> {
        self.input_send_event(vec![relative_event("x", dx), relative_event("y", dy)])
    }

    /// Presses or releases a button of the pointer.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn button(&mut self, button: Button, pressed: bool) -> Result<(), Error> {
        self.input_send_event(vec![button_event(button, pressed)])
    }

    /// Sends the events of one `input-send-event`.
    fn input_send_event(&mut self, events: Vec<Value>) -> Result<(), Error> {
        self.execute(
            "input-send-event",
            Value::object([("events".to_owned(), Value::Array(events))]),
        )?;
        Ok(())
    }

    /// Reads lines until one is an answer rather than an event.
    fn answer(&mut self, command: &str) -> Result<Value, Error> {
        loop {
            let line = self.line()?;
            let value = json::parse(&line).map_err(QmpError::from)?;
            if value.get("event").is_some() {
                continue;
            }
            if let Some(error) = value.get("error") {
                let class = error
                    .get("class")
                    .and_then(Value::text)
                    .unwrap_or("GenericError");
                let message = error
                    .get("desc")
                    .and_then(Value::text)
                    .unwrap_or("the machine said nothing about it");
                return Err(QmpError::Refused {
                    class: class.to_owned(),
                    message: format!("{message} (from `{command}`)"),
                }
                .into());
            }
            return match value.get("return") {
                Some(returned) => Ok(returned.clone()),
                None => Err(QmpError::Answer(line).into()),
            };
        }
    }

    /// Reads one line, which is one message.
    fn line(&mut self) -> Result<String, Error> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .map_err(|source| Error::io("reading from the machine", source))?;
        if read == 0 {
            return Err(QmpError::Closed.into());
        }
        Ok(line)
    }
}

#[cfg(test)]
impl<R: BufRead> Session<R, Vec<u8>> {
    /// What the session has sent, which is what a test holds it to.
    pub(crate) fn sent(&self) -> String {
        String::from_utf8_lossy(&self.writer).into_owned()
    }
}

/// The protocol over the Unix socket of a running machine.
pub(crate) struct Qmp {
    /// The session over the socket.
    session: Session<BufReader<UnixStream>, UnixStream>,
}

impl Qmp {
    /// Connects to the socket at `path` and negotiates.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the socket cannot be reached inside `timeout`,
    /// and the errors of [`Session::start`].
    pub(crate) fn connect(path: &Path, timeout: Duration) -> Result<Self, Error> {
        let stream = UnixStream::connect(path)
            .map_err(|source| Error::io(format!("connecting to {}", path.display()), source))?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .map_err(|source| Error::io("setting the timeout of the machine socket", source))?;
        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|source| Error::io("cloning the machine socket", source))?,
        );
        Ok(Qmp {
            session: Session::start(reader, stream)?,
        })
    }

    /// Runs one command.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn execute(&mut self, command: &str, arguments: Value) -> Result<Value, Error> {
        self.session.execute(command, arguments)
    }

    /// Presses or releases the key QEMU names `qcode`.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn send_key(&mut self, qcode: &str, pressed: bool) -> Result<(), Error> {
        self.session.send_key(qcode, pressed)
    }

    /// Moves the pointer by `dx` and `dy`.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn move_pointer(&mut self, dx: i32, dy: i32) -> Result<(), Error> {
        self.session.move_pointer(dx, dy)
    }

    /// Presses or releases a button of the pointer.
    ///
    /// # Errors
    ///
    /// As [`Session::execute`].
    pub(crate) fn button(&mut self, button: Button, pressed: bool) -> Result<(), Error> {
        self.session.button(button, pressed)
    }

    /// Takes a picture of the screen and reads it.
    ///
    /// # Errors
    ///
    /// The errors of the command, of reading the file, and of the picture.
    pub(crate) fn screendump(&mut self, path: &Path) -> Result<Image, Error> {
        let _ = std::fs::remove_file(path);
        self.execute(
            "screendump",
            Value::object([
                (
                    "filename".to_owned(),
                    Value::Text(path.display().to_string()),
                ),
                ("format".to_owned(), Value::Text("ppm".to_owned())),
            ]),
        )?;
        // The command returns when the machine has taken the picture, but
        // the file is written by the display back end; a moment later it is
        // whole, and a picture read half written is a test that fails for
        // nothing.
        std::thread::sleep(SCREENDUMP_SETTLE);
        let bytes = crate::fs::read_bytes(path)?;
        Image::parse(&bytes).map_err(|error| Error::Parse(format!("{error}")))
    }
}

/// A button of the pointer, as QEMU names it.
///
/// The table is the machine's vocabulary and not the runner's: the runner
/// presses the left button and turns nothing, and the other four stand here
/// so that a test which needs one names it rather than counting.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the five buttons the machine protocol names; the runner presses one of them and the tests name them all"
    )
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Button {
    /// The left button.
    Left,
    /// The middle button.
    Middle,
    /// The right button.
    Right,
    /// The wheel, turned away from the hand.
    WheelUp,
    /// The wheel, turned towards it.
    WheelDown,
}

impl Button {
    /// The name QEMU knows the button by. It is a name and not an index,
    /// because that is what the machine protocol takes.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Button::Left => "left",
            Button::Middle => "middle",
            Button::Right => "right",
            Button::WheelUp => "wheel-up",
            Button::WheelDown => "wheel-down",
        }
    }
}

/// One event of `input-send-event`, with its type and its data.
fn event(kind: &str, data: Vec<(String, Value)>) -> Value {
    Value::object([
        ("type".to_owned(), Value::Text(kind.to_owned())),
        ("data".to_owned(), Value::object(data)),
    ])
}

/// A key going down or coming up, named by its `qcode`.
fn key_event(qcode: &str, pressed: bool) -> Value {
    event(
        "key",
        vec![
            ("down".to_owned(), Value::Bool(pressed)),
            (
                "key".to_owned(),
                Value::object([
                    ("type".to_owned(), Value::Text("qcode".to_owned())),
                    ("data".to_owned(), Value::Text(qcode.to_owned())),
                ]),
            ),
        ],
    )
}

/// A movement along one axis.
fn relative_event(axis: &str, value: i32) -> Value {
    event(
        "rel",
        vec![
            ("axis".to_owned(), Value::Text(axis.to_owned())),
            ("value".to_owned(), Value::Int(i64::from(value))),
        ],
    )
}

/// A button going down or coming up.
fn button_event(button: Button, pressed: bool) -> Value {
    event(
        "btn",
        vec![
            ("down".to_owned(), Value::Bool(pressed)),
            ("button".to_owned(), Value::Text(button.name().to_owned())),
        ],
    )
}
