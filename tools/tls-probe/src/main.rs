// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A real call, over the crates this repository builds.
//!
//! The probe opens a TCP socket, drives `audhsos-tls` across it, and
//! speaks HTTP/1.1 inside the result. Everything above the socket is this
//! project's own code: `crypto-rng` produces the private value and the
//! randoms, `crypto-ec` does X25519 and ECDSA, `crypto-hash` and
//! `crypto-aead` do the key schedule and the record protection,
//! `audhsos-der` and `audhsos-x509` read the chain, and `audhsos-time`
//! supplies the moment the validity windows are checked against. The host
//! contributes a socket and a device full of random bytes.
//!
//! What the probe is for: the trace tests say the machine agrees with a
//! recorded transcript. They cannot say it agrees with a server that was
//! never asked. This does.

mod anchor;
mod entropy;
mod error;
mod http;
mod wire;

use std::net::TcpStream;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audhsos_time::{CivilTime, UnixTime};
use audhsos_tls::client::{Buffers, Connection, MIN_HANDSHAKE, MIN_INCOMING};
use audhsos_tls::config::ClientConfig;
use audhsos_tls::record::MAX_RECORD;
use audhsos_x509::{TrustAnchor, TrustAnchors};
use crypto_rng::ChaChaRng;

use crate::entropy::OsEntropy;
use crate::error::ProbeError;

/// The anchor a run pins when the caller names no other.
///
/// It is `GTS Root R4`, the Google Trust Services root the chain for
/// `google.de` reaches. Its key is P-384, so until `crypto-ec` had that
/// curve this had to be the intermediate below it instead.
const DEFAULT_ANCHOR: &[u8] = include_bytes!("../anchors/gts-root-r4.der");

/// The host a run reaches when the caller names no other.
const DEFAULT_HOST: &str = "google.de";

/// How long the socket waits before it gives up.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The protocol the client offers.
const HTTP11: &[u8] = b"http/1.1";

/// How much of a body is printed.
const PREVIEW: usize = 512;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tls-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What the command line asked for.
struct Arguments {
    /// The name to connect to and to check the certificate against.
    host: String,
    /// The port.
    port: u16,
    /// The request target.
    path: String,
    /// A file holding the anchor, DER or PEM.
    anchor: Option<String>,
}

/// One run.
fn run() -> Result<(), ProbeError> {
    let arguments = parse_arguments()?;

    let now = wall_clock()?;
    let anchor_file = match &arguments.anchor {
        Some(path) => std::fs::read(path)?,
        None => DEFAULT_ANCHOR.to_vec(),
    };
    let anchor_der = anchor::der(&anchor_file)?;
    let anchors = [TrustAnchor::from_certificate(&anchor_der)
        .map_err(|error| ProbeError::Anchor(error.into()))?];

    let protocols: [&[u8]; 1] = [HTTP11];
    let mut config = ClientConfig::new(&arguments.host, TrustAnchors::new(&anchors), now);
    config.alpn = &protocols;

    let mut rng = ChaChaRng::new(OsEntropy::open()?)?;
    let mut incoming = vec![0u8; MIN_INCOMING];
    let mut outgoing = vec![0u8; MAX_RECORD.saturating_mul(2)];
    let mut handshake = vec![0u8; MIN_HANDSHAKE];

    println!(
        "connecting to {}:{} at {:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        arguments.host,
        arguments.port,
        now.year,
        now.month,
        now.day,
        now.hour,
        now.minute,
        now.second
    );

    let stream = TcpStream::connect((arguments.host.as_str(), arguments.port))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let peer = stream.peer_addr()?;
    println!("tcp    {peer}");

    let mut connection = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
            handshake: &mut handshake,
        },
    )?;
    let mut wire = wire::Wire::new(stream);

    wire.handshake(&mut connection)?;
    let chosen = connection.alpn().map_or_else(
        || "none".to_owned(),
        |name| String::from_utf8_lossy(name).into_owned(),
    );
    println!("tls    handshake complete, alpn {chosen}");
    println!(
        "x509   chain verified to the pinned anchor for {}",
        arguments.host
    );

    let request = http::get(&arguments.host, &arguments.path);
    let raw = wire.exchange(&mut connection, &request)?;
    let response = http::parse(&raw)?;
    println!(
        "http   {} {} ({} bytes of body)",
        response.status,
        response.reason,
        response.body.len()
    );
    for name in ["server", "content-type", "location", "date"] {
        if let Some(value) = response.header(name) {
            println!("       {name}: {value}");
        }
    }

    let preview = response
        .body
        .get(..PREVIEW.min(response.body.len()))
        .unwrap_or(&[]);
    println!("---");
    println!("{}", String::from_utf8_lossy(preview));
    if response.body.len() > PREVIEW {
        println!(
            "... ({} bytes omitted)",
            response.body.len().saturating_sub(PREVIEW)
        );
    }

    // Best effort: the peer usually closed first, in which case the client
    // has no keys left to seal an alert with.
    let _ = wire.close(&mut connection);
    Ok(())
}

/// Reads the command line.
fn parse_arguments() -> Result<Arguments, ProbeError> {
    let mut host = None;
    let mut path = None;
    let mut anchor = None;
    let mut rest = std::env::args().skip(1);

    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--anchor" => {
                anchor = Some(
                    rest.next()
                        .ok_or(ProbeError::Usage("--anchor needs a file"))?,
                );
            }
            "-h" | "--help" => {
                return Err(ProbeError::Usage(
                    "tls-probe [host[:port]] [path] [--anchor <file.der|file.pem>]",
                ));
            }
            _ if host.is_none() => host = Some(argument),
            _ if path.is_none() => path = Some(argument),
            _ => return Err(ProbeError::Usage("too many arguments")),
        }
    }

    let authority = host.unwrap_or_else(|| DEFAULT_HOST.to_owned());
    let (host, port) = match authority.rsplit_once(':') {
        Some((name, port)) => (
            name.to_owned(),
            port.parse::<u16>()
                .map_err(|_| ProbeError::Usage("the port is not a number"))?,
        ),
        None => (authority, 443),
    };
    if host.is_empty() {
        return Err(ProbeError::Usage("the host is empty"));
    }

    Ok(Arguments {
        host,
        port,
        path: path.unwrap_or_else(|| "/".to_owned()),
        anchor,
    })
}

/// The moment the certificate windows are checked against.
///
/// `ClientConfig::now` is a parameter because this system has no clock
/// yet. Here the host has one, and this is the seam where it enters.
fn wall_clock() -> Result<CivilTime, ProbeError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProbeError::Usage("the host clock is before the epoch"))?
        .as_secs();
    let seconds =
        i64::try_from(seconds).map_err(|_| ProbeError::Usage("the host clock is out of range"))?;
    Ok(UnixTime::from_seconds(seconds).to_civil()?)
}
