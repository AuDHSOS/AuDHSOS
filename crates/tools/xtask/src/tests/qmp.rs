// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::qmp`, covering the catalog items 6.6.28. The protocol
//! runs over two byte buffers here; the socket is only what carries it in a
//! run of the machine.

use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use crate::error::Error;
use crate::json::Value;
use crate::qmp::{Button, Qmp, Session};

/// The greeting QEMU sends.
const GREETING: &str =
    r#"{"QMP":{"version":{"qemu":{"micro":1,"minor":1,"major":11}},"capabilities":[]}}"#;

/// The session of these tests: the machine's lines out of a buffer, and
/// what the runner writes into another.
type Recorded = Session<BufReader<std::io::Cursor<Vec<u8>>>, Vec<u8>>;

/// A session over `lines`.
fn session(lines: &[&str]) -> Recorded {
    let text = lines.join("\n");
    let reader = BufReader::new(std::io::Cursor::new(text.into_bytes()));
    Session::start(reader, Vec::new()).expect("the session starts")
}

/// What the session wrote, one line per message.
fn written(session: &Recorded) -> Vec<String> {
    session
        .sent()
        .lines()
        .map(std::string::ToString::to_string)
        .collect()
}

#[test]
fn the_greeting_is_read_and_the_capabilities_are_negotiated_first() {
    let session = session(&[GREETING, r#"{"return":{}}"#]);
    assert_eq!(written(&session), vec![r#"{"execute":"qmp_capabilities"}"#]);
}

#[test]
fn a_first_line_that_is_no_greeting_is_refused() {
    let reader = BufReader::new(std::io::Cursor::new(
        format!("{}\n", r#"{"return":{}}"#).into_bytes(),
    ));
    let message = match Session::start(reader, Vec::new()) {
        Ok(_started) => String::from("the session started"),
        Err(error) => format!("{error}"),
    };
    assert!(message.contains("no greeting"), "{message}");
}

#[test]
fn a_connection_that_ends_before_the_greeting_is_refused() {
    let reader = BufReader::new(std::io::Cursor::new(Vec::new()));
    let message = match Session::start(reader, Vec::new()) {
        Ok(_started) => String::from("the session started"),
        Err(error) => format!("{error}"),
    };
    assert!(message.contains("closed the connection"), "{message}");
}

#[test]
fn a_command_carries_its_arguments_and_answers_with_what_it_returned() {
    let mut session = session(&[GREETING, r#"{"return":{}}"#, r#"{"return":{"seen":7}}"#]);
    let returned = session
        .execute(
            "screendump",
            Value::object([("filename".to_owned(), Value::Text("/tmp/x.ppm".to_owned()))]),
        )
        .expect("the command answers");
    assert_eq!(returned.get("seen"), Some(&Value::Int(7)));
    assert_eq!(
        written(&session).get(1).map(String::as_str),
        Some(r#"{"arguments":{"filename":"/tmp/x.ppm"},"execute":"screendump"}"#)
    );
}

#[test]
fn events_between_a_command_and_its_answer_are_stepped_over() {
    let mut session = session(&[
        GREETING,
        r#"{"event":"RESUME","timestamp":{"seconds":1,"microseconds":2}}"#,
        r#"{"return":{}}"#,
        r#"{"event":"STOP","timestamp":{"seconds":1,"microseconds":3}}"#,
        r#"{"return":{"here":1}}"#,
    ]);
    let returned = session
        .execute("stop", Value::object([]))
        .expect("an answer");
    assert_eq!(returned.get("here"), Some(&Value::Int(1)));
}

#[test]
fn an_error_answer_becomes_an_error_value_and_not_a_panic() {
    let mut session = session(&[
        GREETING,
        r#"{"return":{}}"#,
        r#"{"error":{"class":"CommandNotFound","desc":"no such command"}}"#,
    ]);
    let outcome = session.execute("nonsense", Value::object([]));
    let message = format!("{}", outcome.expect_err("an error"));
    assert!(message.contains("CommandNotFound"), "{message}");
    assert!(message.contains("no such command"), "{message}");
    assert!(message.contains("nonsense"), "{message}");
}

#[test]
fn an_error_without_a_class_or_a_description_still_reads() {
    let mut session = session(&[GREETING, r#"{"return":{}}"#, r#"{"error":{}}"#]);
    let outcome = session.execute("nonsense", Value::object([]));
    let message = format!("{}", outcome.expect_err("an error"));
    assert!(message.contains("GenericError"), "{message}");
}

#[test]
fn an_answer_that_is_neither_a_return_nor_an_error_is_refused() {
    let mut session = session(&[GREETING, r#"{"return":{}}"#, r#"{"nothing":1}"#]);
    let outcome = session.execute("stop", Value::object([]));
    let message = format!("{}", outcome.expect_err("an error"));
    assert!(message.contains("neither return nor error"), "{message}");
}

#[test]
fn a_line_that_is_no_json_is_refused() {
    let mut session = session(&[GREETING, r#"{"return":{}}"#, "not json"]);
    let outcome = session.execute("stop", Value::object([]));
    let message = format!("{}", outcome.expect_err("an error"));
    assert!(message.contains("no JSON"), "{message}");
}

#[test]
fn a_machine_that_says_nothing_runs_into_the_timeout() {
    let path = PathBuf::from(format!(
        "/tmp/audhsos-qmp-test-{}-{:?}.sock",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("a socket to listen on");
    let accepted = std::thread::spawn(move || {
        // Accept and say nothing at all, which is what a machine that hangs
        // does.
        let held = listener.accept();
        std::thread::sleep(Duration::from_millis(400));
        drop(held);
    });
    let outcome = Qmp::connect(&path, Duration::from_millis(50));
    let message = match outcome {
        Ok(_connected) => String::from("the connection answered"),
        Err(error) => format!("{error}"),
    };
    let _joined = accepted.join();
    let _ = std::fs::remove_file(&path);
    assert!(
        message.contains("reading from the machine"),
        "the timeout was not what ended it: {message}"
    );
}

#[test]
fn a_socket_that_is_not_there_is_refused_at_once() {
    let outcome = Qmp::connect(
        &PathBuf::from("/tmp/audhsos-qmp-nothing-here.sock"),
        Duration::from_millis(50),
    );
    assert!(matches!(outcome, Err(Error::Io { .. })));
}

#[test]
fn a_stream_that_cannot_be_written_to_is_reported() {
    let mut session = session(&[GREETING, r#"{"return":{}}"#]);
    drop(UnixStream::pair());
    let outcome = session.execute("stop", Value::object([]));
    // The reader is at its end, so the answer is what fails, and it says so.
    let message = format!("{}", outcome.expect_err("an error"));
    assert!(message.contains("closed the connection"), "{message}");
}

#[test]
fn a_key_goes_down_and_comes_up_by_the_name_qemu_knows_it_under() {
    let mut session = session(&[
        GREETING,
        r#"{"return":{}}"#,
        r#"{"return":{}}"#,
        r#"{"return":{}}"#,
    ]);
    session.send_key("esc", true).expect("the key goes down");
    session.send_key("esc", false).expect("and comes up");
    assert_eq!(
        written(&session).get(1..),
        Some(
            [
                r#"{"arguments":{"events":[{"data":{"down":true,"key":{"data":"esc","type":"qcode"}},"type":"key"}]},"execute":"input-send-event"}"#.to_owned(),
                r#"{"arguments":{"events":[{"data":{"down":false,"key":{"data":"esc","type":"qcode"}},"type":"key"}]},"execute":"input-send-event"}"#.to_owned(),
            ]
            .as_slice()
        )
    );
}

#[test]
fn one_motion_of_the_pointer_is_one_command_carrying_both_axes() {
    let mut session = session(&[GREETING, r#"{"return":{}}"#, r#"{"return":{}}"#]);
    session.move_pointer(5, -3).expect("the pointer moves");
    assert_eq!(
        written(&session).get(1).map(String::as_str),
        Some(
            r#"{"arguments":{"events":[{"data":{"axis":"x","value":5},"type":"rel"},{"data":{"axis":"y","value":-3},"type":"rel"}]},"execute":"input-send-event"}"#
        ),
        "both axes go out together, so the mouse packetizes the motion once"
    );
}

#[test]
fn a_button_goes_down_and_comes_up_and_every_button_has_a_name() {
    let mut session = session(&[
        GREETING,
        r#"{"return":{}}"#,
        r#"{"return":{}}"#,
        r#"{"return":{}}"#,
    ]);
    session
        .button(Button::Left, true)
        .expect("the button goes down");
    session.button(Button::Left, false).expect("and comes up");
    assert_eq!(
        written(&session).get(1..),
        Some(
            [
                r#"{"arguments":{"events":[{"data":{"button":"left","down":true},"type":"btn"}]},"execute":"input-send-event"}"#.to_owned(),
                r#"{"arguments":{"events":[{"data":{"button":"left","down":false},"type":"btn"}]},"execute":"input-send-event"}"#.to_owned(),
            ]
            .as_slice()
        )
    );

    let names: Vec<&str> = [
        Button::Left,
        Button::Middle,
        Button::Right,
        Button::WheelUp,
        Button::WheelDown,
    ]
    .iter()
    .map(|button| button.name())
    .collect();
    assert_eq!(
        names,
        vec!["left", "middle", "right", "wheel-up", "wheel-down"],
        "QEMU names its buttons, so this does too"
    );
}

#[test]
fn an_injection_the_machine_refuses_is_an_error_and_not_a_panic() {
    let mut session = session(&[
        GREETING,
        r#"{"return":{}}"#,
        r#"{"error":{"class":"GenericError","desc":"no input device"}}"#,
    ]);
    let message = match session.send_key("a", true) {
        Ok(()) => String::from("the key went down"),
        Err(error) => format!("{error}"),
    };
    assert!(message.contains("no input device"), "{message}");
}
