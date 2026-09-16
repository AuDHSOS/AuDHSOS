// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a line asks for.
//!
//! A line is one word and what follows it. The word names a command; the
//! rest is that command's to read, and two of them read something with a
//! shape of its own: `ssh` reads `[user@]host[:port] [command]`, and `get`
//! reads an HTTP or HTTPS locator. Everything here borrows from the line
//! it was given and allocates nothing.
//!
//! A locator this parser accepts is `http://host[:port][/path]` or the
//! same under `https`. What is not accepted is named as such: a line that
//! does not parse answers with the usage of the command it named, which is
//! what the shell then prints.
//!
//! Invariants: the host of a target or a locator is never empty, and the
//! path of a locator always begins with `/`, so a caller may use both
//! without checking again.

/// The port a Secure Shell session takes when the line names none.
pub const SSH_PORT: u16 = 22;

/// The user a session takes when the line names none.
pub const SSH_USER: &str = "root";

/// The port an HTTP request takes when the locator names none.
pub const HTTP_PORT: u16 = 80;

/// The port an HTTPS request takes when the locator names none.
pub const HTTPS_PORT: u16 = 443;

/// How to reach a Secure Shell server, and what to run there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target<'a> {
    /// Who to authenticate as.
    pub user: &'a str,
    /// The host the connection goes to.
    pub host: &'a str,
    /// The port on it.
    pub port: u16,
    /// What to run there, empty for a session without a command.
    pub command: &'a str,
}

/// What an HTTP or HTTPS request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Locator<'a> {
    /// Whether the connection is under TLS.
    pub secure: bool,
    /// The host the request goes to.
    pub host: &'a str,
    /// The port on it.
    pub port: u16,
    /// What is asked for, beginning with `/`.
    pub path: &'a str,
}

/// What a line of the shell asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command<'a> {
    /// The line was empty.
    Nothing,
    /// What the shell can do.
    Help,
    /// Forget the scrollback.
    Clear,
    /// What the clock says.
    Time,
    /// Write this back.
    Echo(&'a str),
    /// Run a command over a Secure Shell connection.
    Ssh(Target<'a>),
    /// Ask a server for a document.
    Get(Locator<'a>),
    /// Close the window.
    Quit,
    /// The command was named but not understood; this is how it is used.
    Usage(&'static str),
    /// No command of this shell has this name.
    Unknown(&'a str),
}

/// How `ssh` is used.
pub const SSH_USAGE: &str = "usage: ssh [user@]host[:port] [command]";

/// How `get` is used.
pub const GET_USAGE: &str = "usage: get http[s]://host[:port][/path]";

/// What the shell answers to `help`, one line each. The locator names no
/// scheme under TLS: this parser takes one, and the program around it does
/// not speak it yet.
pub const HELP: [&str; 6] = [
    "ssh [user@]host[:port] [command]",
    "get http://host[:port][/path]",
    "echo <text>",
    "time",
    "clear",
    "quit",
];

/// What a line asks for.
#[must_use]
pub fn parse(line: &str) -> Command<'_> {
    let line = line.trim();
    if line.is_empty() {
        return Command::Nothing;
    }
    let (word, rest) = split(line);
    match word {
        "help" | "?" => Command::Help,
        "clear" => Command::Clear,
        "time" => Command::Time,
        "echo" => Command::Echo(rest),
        "quit" | "exit" => Command::Quit,
        "ssh" => target(rest).map_or(Command::Usage(SSH_USAGE), Command::Ssh),
        "get" => locator(rest).map_or(Command::Usage(GET_USAGE), Command::Get),
        other => Command::Unknown(other),
    }
}

/// The first word of `line` and what stands behind it.
fn split(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(at) => {
            let word = line.get(..at).unwrap_or_default();
            let rest = line.get(at..).unwrap_or_default().trim_start();
            (word, rest)
        }
        None => (line, ""),
    }
}

/// The target `rest` names, or `None` when it names none.
#[must_use]
pub fn target(rest: &str) -> Option<Target<'_>> {
    let (where_to, command) = split(rest);
    if where_to.is_empty() {
        return None;
    }
    let (user, host_port) = match where_to.find('@') {
        Some(at) => (
            where_to.get(..at).unwrap_or_default(),
            where_to.get(at.saturating_add(1)..).unwrap_or_default(),
        ),
        None => (SSH_USER, where_to),
    };
    let (host, port) = host_and_port(host_port, SSH_PORT)?;
    if user.is_empty() {
        return None;
    }
    Some(Target {
        user,
        host,
        port,
        command,
    })
}

/// The locator `rest` names, or `None` when it names none.
#[must_use]
pub fn locator(rest: &str) -> Option<Locator<'_>> {
    let (text, _behind) = split(rest);
    let (secure, authority) = match strip(text, "https://") {
        Some(behind) => (true, behind),
        None => (false, strip(text, "http://")?),
    };
    let (host_port, path) = match authority.find('/') {
        Some(at) => (
            authority.get(..at).unwrap_or_default(),
            authority.get(at..).unwrap_or_default(),
        ),
        None => (authority, "/"),
    };
    let default = if secure { HTTPS_PORT } else { HTTP_PORT };
    let (host, port) = host_and_port(host_port, default)?;
    Some(Locator {
        secure,
        host,
        port,
        path,
    })
}

/// `text` without `prefix`, or `None` when it does not begin with it. The
/// comparison ignores the case of the scheme, which a locator may spell in
/// either.
fn strip<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    text.get(prefix.len()..)
}

/// The host and the port of `text`, which is `host` or `host:port`.
fn host_and_port(text: &str, default: u16) -> Option<(&str, u16)> {
    let Some(at) = text.find(':') else {
        return non_empty(text).map(|host| (host, default));
    };
    let host = non_empty(text.get(..at)?)?;
    let digits = text.get(at.saturating_add(1)..)?;
    let port = digits.parse::<u16>().ok()?;
    if port == 0 {
        return None;
    }
    Some((host, port))
}

/// `text` when it has bytes, and nothing when it has none.
const fn non_empty(text: &str) -> Option<&str> {
    if text.is_empty() { None } else { Some(text) }
}
