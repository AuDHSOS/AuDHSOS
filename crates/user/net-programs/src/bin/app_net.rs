// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The program that uses the network server: it asks what the interface
//! is, resolves a name, takes a connection on the echo port, sends back
//! what it was sent, and then makes an HTTP request over that same
//! connection and parses the answer.
//!
//! The connection is `user_programs::socket::Stream`, which is what asks
//! again after a wait on the clock for every call the server answers
//! `WouldBlock` (D-142); nothing here spins and nothing here reaches a
//! ring.
//!
//! The connection carries two exchanges because the machine it runs on has
//! one port forwarded into it: the runner connects to that port, sends a
//! line and reads it back, and then answers the request this program makes
//! over the connection that is already open.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

// The package holds four programs and each uses a different part of what
// it depends on; these are the crates this one does not.
use audhsos_encoding as _;
use audhsos_ssh as _;
use audhsos_time as _;
use audhsos_x509 as _;
use crypto_rng as _;
use driver_virtio_net as _;
use net_stack as _;
use server_net as _;
use user_net_programs as _;
use virtio_queue as _;

use audhsos_abi::Error;
use net_http::{Decoder, Event, Method, Request as HttpRequest};
use net_wire::Writer as WireWriter;
use user_programs::client::write_line;
use user_programs::socket::{Idle, Listener, Stream};
use user_proto::socket::{Name, Reply, Request};
use user_rt::{EndpointHandle, Line, ProcessHandle, Startup};
use user_sys_x86_64::{self as sys, Gate};

sys::program!(main);

/// The port the runner's connection reaches, which is the port the
/// reference machine forwards (13.12).
const PORT: u16 = 7;

/// The name this program asks the resolver for.
const NAME: &[u8] = b"example.com";

/// The host the request names, which is the gateway of the machine.
const HOST: &str = "10.0.2.2";

/// The target the request asks for.
const TARGET: &str = "/";

/// Where the rings of the socket are mapped.
const RINGS: u64 = 0x0000_7400_0000_0000;

/// How long the program waits for one step, in microseconds.
const DEADLINE: u64 = 20_000_000;

/// How long it sleeps between two asks, in microseconds.
const STEP: u64 = 10_000;

/// How long a line this program writes.
type Report = Line<200>;

/// How many bytes of the exchange the program holds at once.
const CHUNK: usize = 512;

/// Asks the network server what it can, and reports each answer.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shape of `main` is what `program!` calls; the gate and the startup message belong to the program"
)]
fn main(mut gate: Gate, startup: Startup) -> ! {
    let voice = startup.log;
    let Some(server) = server_endpoint(&startup) else {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net-app] no server\n")),
        );
        gate.thread_exit()
    };
    let (Some(process), Some(_memory)) = (startup.own_process, startup.memory_server) else {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net-app] nothing to map with\n")),
        );
        gate.thread_exit()
    };

    let Ok(idle) = Idle::new(&mut gate).map(|idle| idle.with_step(STEP)) else {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net-app] nothing to wait on\n")),
        );
        gate.thread_exit()
    };

    if let Err(error) = report_interface(&mut gate, server, voice, idle) {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!(
                "[net-app] no interface: {}\n",
                error.message()
            )),
        );
        report_to_parent(&mut gate, &startup);
        gate.thread_exit()
    }
    resolve(&mut gate, server, voice, idle);
    if let Err(error) = echo_and_fetch(&mut gate, server, process, voice, idle) {
        say(
            &mut gate,
            voice,
            &Report::of(format_args!("[net-app] no exchange: {}\n", error.message())),
        );
    }
    say(
        &mut gate,
        voice,
        &Report::of(format_args!("[net-app] done\n")),
    );
    report_to_parent(&mut gate, &startup);
    gate.thread_exit()
}

/// The endpoint of the network server, badged by the root task.
///
/// A capability found under the name `net` carries no badge, and the
/// server refuses a request that names nobody.
const fn server_endpoint(startup: &Startup) -> Option<EndpointHandle> {
    startup.net_server
}

/// Asks what the interface is until a lease is there, and says so.
fn report_interface(
    gate: &mut Gate,
    server: EndpointHandle,
    voice: Option<EndpointHandle>,
    idle: Idle,
) -> Result<(), Error> {
    let until = gate.clock_now()?.saturating_add(DEADLINE);
    loop {
        let reply = call(gate, server, &Request::Interface)?;
        let Reply::Interface(outcome) = reply else {
            return Err(Error::InvalidArgument);
        };
        let interface = outcome?;
        if interface.lease {
            let mut line = Report::of(format_args!(
                "[net-app] lease=yes mac={} addresses:",
                interface.mac
            ));
            for address in interface.addresses.iter() {
                line.put(b" ");
                let one: Line<64> = Line::of(format_args!("{address}"));
                line.put(one.as_bytes());
            }
            match interface.gateway {
                Some(gateway) => {
                    let one: Line<64> = Line::of(format_args!(" gateway={gateway}\n"));
                    line.put(one.as_bytes());
                }
                None => line.put(b" gateway=none\n"),
            }
            say(gate, voice, &line);
            return Ok(());
        }
        idle.wait(gate, until)?;
    }
}

/// Asks the resolver for a name and says what came back.
fn resolve(gate: &mut Gate, server: EndpointHandle, voice: Option<EndpointHandle>, idle: Idle) {
    let Ok(name) = Name::new(NAME) else {
        return;
    };
    let Ok(until) = gate.clock_now().map(|now| now.saturating_add(DEADLINE)) else {
        return;
    };
    loop {
        let outcome = match call(gate, server, &Request::Resolve { name }) {
            Ok(Reply::Resolved(outcome)) => outcome,
            _other => {
                say(
                    gate,
                    voice,
                    &Report::of(format_args!("[net-app] resolve refused\n")),
                );
                return;
            }
        };
        match outcome {
            Ok(addresses) => {
                let mut line = Report::of(format_args!("[net-app] resolved example.com:"));
                for address in addresses.iter() {
                    let one: Line<64> = Line::of(format_args!(" {address}"));
                    line.put(one.as_bytes());
                }
                line.put(b"\n");
                say(gate, voice, &line);
                return;
            }
            Err(Error::WouldBlock) => {
                if idle.wait(gate, until).is_err() {
                    say(
                        gate,
                        voice,
                        &Report::of(format_args!("[net-app] resolve timed out\n")),
                    );
                    return;
                }
            }
            Err(error) => {
                say(
                    gate,
                    voice,
                    &Report::of(format_args!(
                        "[net-app] resolve failed: {}\n",
                        error.message()
                    )),
                );
                return;
            }
        }
    }
}

/// Takes the connection the runner opens, echoes what it sends, and then
/// makes a request over the same connection.
fn echo_and_fetch(
    gate: &mut Gate,
    server: EndpointHandle,
    process: ProcessHandle,
    voice: Option<EndpointHandle>,
    idle: Idle,
) -> Result<(), Error> {
    let listener = Listener::bind(gate, server, process, PORT)?;
    say(
        gate,
        voice,
        &Report::of(format_args!("[net-app] listening on {}\n", listener.port())),
    );

    let until = gate.clock_now()?.saturating_add(DEADLINE);
    let stream = listener.accept(gate, process, RINGS, idle, until)?;
    say(
        gate,
        voice,
        &Report::of(format_args!(
            "[net-app] accepted socket {}\n",
            stream.socket()
        )),
    );

    // One line goes back, and a line that arrives in pieces is read and
    // written piece by piece: what the peer sent in one write of its own
    // is not what one read of a stream answers.
    let mut taken = [0u8; CHUNK];
    let mut echoed = 0usize;
    loop {
        let len = stream.read(gate, &mut taken, idle, until)?;
        let line = taken.get(..len).unwrap_or(&[]);
        if line.is_empty() {
            break;
        }
        stream.write_all(gate, line, idle, until)?;
        echoed = echoed.saturating_add(len);
        if line.last() == Some(&b'\n') {
            break;
        }
    }
    say(
        gate,
        voice,
        &Report::of(format_args!("[net-app] echo {echoed} bytes\n")),
    );

    fetch(gate, &stream, voice, until, idle)?;
    let _shut = stream.shutdown_write(gate, idle, until);
    let _closed = stream.close(gate);
    say(gate, voice, &Report::of(format_args!("[net-app] closed\n")));
    Ok(())
}

/// Writes a request over the connection and reads the answer.
fn fetch(
    gate: &mut Gate,
    stream: &Stream,
    voice: Option<EndpointHandle>,
    until: u64,
    idle: Idle,
) -> Result<(), Error> {
    let mut request = [0u8; CHUNK];
    let mut writer = WireWriter::new(&mut request);
    HttpRequest::get(TARGET, HOST)
        .write(&mut writer)
        .map_err(|_| Error::InvalidArgument)?;
    let len = writer.position();
    stream.write_all(gate, request.get(..len).unwrap_or(&[]), idle, until)?;

    let mut head = [0u8; 1024];
    let mut decoder = Decoder::<8>::new(&mut head, Method::Get);
    let mut body = 0usize;
    let mut status = 0u16;
    let mut taken = [0u8; CHUNK];
    loop {
        let len = stream.read(gate, &mut taken, idle, until)?;
        let mut rest = taken.get(..len).unwrap_or(&[]);
        if rest.is_empty() {
            // A body whose end is the end of the connection is complete
            // the moment the peer closes; any other is short.
            if decoder.ends_at_close() && decoder.finish().is_ok() {
                said(gate, voice, status, body);
                return Ok(());
            }
            return Err(Error::Unavailable);
        }
        while !rest.is_empty() {
            let (used, event) = decoder.feed(rest).map_err(|_| Error::InvalidArgument)?;
            rest = rest.get(used..).unwrap_or(&[]);
            match event {
                // A call that took nothing can take nothing more of what
                // is left; one that took a line of the head and asks for
                // another is fed the rest of the same read.
                Event::NeedMore => {
                    if used == 0 {
                        break;
                    }
                }
                Event::Head => status = decoder.head().map_or(0, |head| head.status.get()),
                Event::Body(bytes) => body = body.saturating_add(bytes.len()),
                Event::Done => {
                    said(gate, voice, status, body);
                    return Ok(());
                }
            }
        }
        // The last byte of a body of known length ends the message with
        // the event that carried it, and no further byte follows it.
        if decoder.is_done() {
            said(gate, voice, status, body);
            return Ok(());
        }
    }
}

/// Says what the answer was.
fn said(gate: &mut Gate, voice: Option<EndpointHandle>, status: u16, body: usize) {
    say(
        gate,
        voice,
        &Report::of(format_args!("[net-app] http status={status} body={body}\n")),
    );
}

/// Sends one request and reads the reply out of the buffer.
fn call(gate: &mut Gate, server: EndpointHandle, request: &Request) -> Result<Reply, Error> {
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    Ok(Reply::decode(gate.reader())?)
}

/// Writes one line, if there is anywhere to write it.
fn say(gate: &mut Gate, voice: Option<EndpointHandle>, line: &Report) {
    let Some(console) = voice else {
        return;
    };
    let _logged = write_line(gate, console, line.as_bytes());
}

/// Tells the process that started this one that the work is done.
fn report_to_parent(gate: &mut Gate, startup: &Startup) {
    let Some(parent) = startup.parent else {
        return;
    };
    let finished = user_proto::parent::Request::Finished {
        status: user_proto::parent::SUCCESS,
    };
    if finished.encode(&mut gate.writer()).is_ok() {
        let _reported = gate.ipc_send(parent);
    }
}
