// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::command`.

use crate::command::{
    Command, GET_USAGE, HELP, HTTP_PORT, HTTPS_PORT, Locator, SSH_PORT, SSH_USAGE, SSH_USER,
    Target, parse,
};

#[test]
fn an_empty_line_asks_for_nothing() {
    assert_eq!(parse(""), Command::Nothing);
    assert_eq!(parse("   "), Command::Nothing);
    assert_eq!(parse("\t"), Command::Nothing);
}

#[test]
fn the_four_commands_of_the_shell_itself() {
    assert_eq!(parse("help"), Command::Help);
    assert_eq!(parse("?"), Command::Help);
    assert_eq!(parse("clear"), Command::Clear);
    assert_eq!(parse("time"), Command::Time);
    assert_eq!(parse("quit"), Command::Quit);
    assert_eq!(parse("exit"), Command::Quit);
    assert_eq!(parse("echo hello world"), Command::Echo("hello world"));
    assert_eq!(parse("echo"), Command::Echo(""));
}

#[test]
fn a_name_no_command_has_is_reported_as_such() {
    assert_eq!(parse("sl"), Command::Unknown("sl"));
    assert_eq!(parse("sl -l /"), Command::Unknown("sl"));
}

#[test]
fn a_session_takes_the_user_and_the_port_of_the_line() {
    assert_eq!(
        parse("ssh manuel@10.0.2.2:2222 uname -a"),
        Command::Ssh(Target {
            user: "manuel",
            host: "10.0.2.2",
            port: 2222,
            command: "uname -a",
        })
    );
}

#[test]
fn a_session_without_a_user_or_a_port_takes_the_defaults() {
    assert_eq!(
        parse("ssh 10.0.2.2"),
        Command::Ssh(Target {
            user: SSH_USER,
            host: "10.0.2.2",
            port: SSH_PORT,
            command: "",
        })
    );
}

#[test]
fn a_session_that_names_no_host_is_no_session() {
    assert_eq!(parse("ssh"), Command::Usage(SSH_USAGE));
    assert_eq!(parse("ssh @host"), Command::Usage(SSH_USAGE));
    assert_eq!(parse("ssh user@"), Command::Usage(SSH_USAGE));
    assert_eq!(parse("ssh :22"), Command::Usage(SSH_USAGE));
}

#[test]
fn a_port_that_is_no_number_or_is_zero_is_refused() {
    assert_eq!(parse("ssh host:zwei"), Command::Usage(SSH_USAGE));
    assert_eq!(parse("ssh host:0"), Command::Usage(SSH_USAGE));
    assert_eq!(parse("ssh host:70000"), Command::Usage(SSH_USAGE));
    assert_eq!(parse("ssh host:"), Command::Usage(SSH_USAGE));
}

#[test]
fn the_help_names_what_the_shell_does() {
    assert_eq!(HELP.len(), 6);
    assert!(HELP.iter().any(|line| line.starts_with("ssh ")));
    assert!(HELP.iter().any(|line| line.starts_with("get http://")));
}

#[test]
fn a_locator_carries_its_scheme_its_host_its_port_and_its_path() {
    assert_eq!(
        parse("get http://example.com/index.html"),
        Command::Get(Locator {
            secure: false,
            host: "example.com",
            port: HTTP_PORT,
            path: "/index.html",
        })
    );
    assert_eq!(
        parse("get https://example.com:8443/a/b"),
        Command::Get(Locator {
            secure: true,
            host: "example.com",
            port: 8443,
            path: "/a/b",
        })
    );
}

#[test]
fn a_locator_without_a_path_asks_for_the_root() {
    assert_eq!(
        parse("get http://10.0.2.2"),
        Command::Get(Locator {
            secure: false,
            host: "10.0.2.2",
            port: HTTP_PORT,
            path: "/",
        })
    );
}

#[test]
fn a_locator_under_tls_takes_the_other_port() {
    let Command::Get(locator) = parse("get https://example.com/") else {
        panic!("a locator was expected");
    };
    assert!(locator.secure);
    assert_eq!(locator.port, HTTPS_PORT);
}

#[test]
fn the_scheme_may_be_spelled_in_either_case() {
    assert_eq!(
        parse("get HTTP://example.com/"),
        Command::Get(Locator {
            secure: false,
            host: "example.com",
            port: HTTP_PORT,
            path: "/",
        })
    );
    assert_eq!(
        parse("get HTTPS://example.com/"),
        Command::Get(Locator {
            secure: true,
            host: "example.com",
            port: HTTPS_PORT,
            path: "/",
        })
    );
}

#[test]
fn a_locator_of_another_scheme_or_of_none_is_refused() {
    assert_eq!(parse("get"), Command::Usage(GET_USAGE));
    assert_eq!(parse("get example.com"), Command::Usage(GET_USAGE));
    assert_eq!(parse("get ftp://example.com/"), Command::Usage(GET_USAGE));
    assert_eq!(parse("get http://"), Command::Usage(GET_USAGE));
    assert_eq!(parse("get http://:80/"), Command::Usage(GET_USAGE));
    assert_eq!(parse("get htt"), Command::Usage(GET_USAGE));
}

#[test]
fn the_space_around_a_line_belongs_to_no_word() {
    assert_eq!(
        parse("   ssh   host   uname   "),
        Command::Ssh(Target {
            user: SSH_USER,
            host: "host",
            port: SSH_PORT,
            command: "uname",
        })
    );
}
