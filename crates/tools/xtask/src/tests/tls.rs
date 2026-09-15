// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tls`, covering the half of catalog item 6.6.65 that
//! lives on the development machine: the HTTPS server the acceptance run
//! of Phase 15 starts, reached over a real socket.
//!
//! The client here is the client of the image — `audhsos-tls::client`
//! over `std`'s socket instead of over `server-net` — so what this test
//! says is that a client of this project completes a handshake against
//! this server, validates the chain against the root the image will
//! carry, and reads the answer. What the run adds is the guest.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use audhsos_time::CivilTime;
use audhsos_tls::client::{Buffers, Connection, Event, MIN_HANDSHAKE, MIN_INCOMING, MIN_OUTGOING};
use audhsos_tls::config::ClientConfig;
use audhsos_tls::error::TlsError;
use audhsos_tls::record::MAX_PLAINTEXT;
use audhsos_x509::{TrustAnchor, TrustAnchors};
use crypto_rng::ChaChaRng;
use crypto_rng::doubles::CountingEntropy;

use crate::tls::{Material, NAME, RESPONSE, Server, guest, scratch_files};

/// What the client asks for.
const REQUEST: &str = "GET / HTTP/1.1\r\nHost: audhsos.test\r\nConnection: close\r\n\r\n";

/// A moment inside the window of the chain.
fn now() -> CivilTime {
    CivilTime {
        year: 2026,
        month: 1,
        day: 1,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// Runs one request against `port`, trusting `anchor`, and answers what
/// the server said.
fn request(port: u16, anchor: &[u8], name: &str, now: CivilTime) -> Result<String, TlsError> {
    let anchors = [TrustAnchor::from_certificate(anchor).expect("a well formed root")];
    let config = ClientConfig::new(name, TrustAnchors::new(&anchors), now);
    let mut rng = ChaChaRng::from_seed(&[0x24; 32], CountingEntropy::new(7));

    let mut incoming = vec![0u8; MIN_INCOMING];
    let mut outgoing = vec![0u8; MIN_OUTGOING];
    let mut handshake = vec![0u8; MIN_HANDSHAKE];
    let mut connection = Connection::new(
        &config,
        &mut rng,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
            handshake: &mut handshake,
        },
    )?;

    let mut stream = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .expect("the server is listening");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("a timeout the socket takes");

    let mut wire = vec![0u8; MAX_PLAINTEXT + 512];
    let mut plaintext = vec![0u8; MAX_PLAINTEXT];
    let mut said = String::new();
    let mut asked = false;
    loop {
        match connection.poll()? {
            Event::WantsWrite => {
                let length = connection.write_tls(&mut wire)?;
                stream
                    .write_all(wire.get(..length).unwrap_or(&[]))
                    .expect("the socket takes the bytes");
            }
            Event::WantsRead => {
                let read = stream.read(&mut wire).unwrap_or(0);
                if read == 0 {
                    break;
                }
                connection.read_tls(wire.get(..read).unwrap_or(&[]))?;
            }
            Event::Handshaked => {
                if !asked {
                    connection.send(REQUEST.as_bytes())?;
                    asked = true;
                    continue;
                }
                let length = connection.recv(&mut plaintext)?;
                if length != 0 {
                    said.push_str(&String::from_utf8_lossy(
                        plaintext.get(..length).unwrap_or(&[]),
                    ));
                    break;
                }
                let read = stream.read(&mut wire).unwrap_or(0);
                if read == 0 {
                    break;
                }
                connection.read_tls(wire.get(..read).unwrap_or(&[]))?;
            }
            Event::PeerClosed => break,
        }
    }
    Ok(said)
}

#[test]
fn the_client_of_this_project_reaches_the_server_over_a_socket() {
    let material = Material::new().expect("the chain is built");
    let server = Server::start(&material).expect("the server takes a port");

    let said = request(server.port(), material.root.as_slice(), NAME, now())
        .expect("the handshake completes and the answer arrives");

    assert_eq!(said, RESPONSE, "the client read what the server wrote");
}

#[test]
fn a_client_that_asks_for_another_name_refuses_the_chain() {
    let material = Material::new().expect("the chain is built");
    let server = Server::start(&material).expect("the server takes a port");

    let outcome = request(
        server.port(),
        material.root.as_slice(),
        "somewhere.else",
        now(),
    );

    // The chain reaches the anchor, so the certificate is what is wrong.
    assert_eq!(outcome, Err(TlsError::BadCertificate));
}

#[test]
fn a_client_that_trusts_another_root_refuses_the_chain() {
    let material = Material::new().expect("the chain is built");
    let server = Server::start(&material).expect("the server takes a port");

    // The leaf is a certificate like any other and no anchor of this
    // chain: a client that carries it reaches nothing it trusts.
    let outcome = request(server.port(), material.leaf.as_slice(), NAME, now());

    assert_eq!(outcome, Err(TlsError::UnknownAuthority));
}

#[test]
fn a_client_whose_clock_is_past_the_window_refuses_the_chain() {
    let material = Material::new().expect("the chain is built");
    let server = Server::start(&material).expect("the server takes a port");

    let late = CivilTime {
        year: 2036,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    let outcome = request(server.port(), material.root.as_slice(), NAME, late);

    assert_eq!(outcome, Err(TlsError::CertificateExpired));
}

#[test]
fn the_scratch_files_name_the_port_the_name_and_the_root() {
    let material = Material::new().expect("the chain is built");

    let files = scratch_files(4711, &material);

    let config = files
        .iter()
        .find(|(path, _)| path == guest::CONFIG)
        .map(|(_, bytes)| String::from_utf8(bytes.clone()).expect("text"))
        .expect("the configuration is written");
    assert_eq!(config, format!("port 4711\nname {NAME}\n"));

    let root = files
        .iter()
        .find(|(path, _)| path == guest::ROOT)
        .map(|(_, bytes)| bytes.clone())
        .expect("the root is written");
    // The bytes are the root itself, because the program of the image
    // reads it with `TrustAnchor::from_certificate` and nothing else.
    assert_eq!(root, material.root.as_slice());
    assert_eq!(files.len(), 2);
}

#[test]
fn the_root_of_the_run_is_an_anchor_the_image_can_use() {
    let material = Material::new().expect("the chain is built");
    let files = scratch_files(4711, &material);
    let root = files
        .iter()
        .find(|(path, _)| path == guest::ROOT)
        .map(|(_, bytes)| bytes.clone())
        .expect("the root is written");

    // What `app-tls` does with the file, and all it does with it (D-150).
    let anchor = TrustAnchor::from_certificate(&root).expect("an anchor");

    assert!(!anchor.subject.is_empty());
    assert!(!anchor.spki.is_empty());
}
