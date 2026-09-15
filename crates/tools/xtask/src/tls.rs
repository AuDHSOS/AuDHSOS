// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The HTTPS server the acceptance run of Phase 15 starts on the
//! development machine, and the chain it presents (D-149).
//!
//! Everything above the socket is this repository's code:
//! `audhsos-x509::builder` writes the certificates, `audhsos-tls::server`
//! speaks the protocol, and this module is the socket and the thread
//! around them. The counterpart on the target is `app-tls`; what the run
//! checks is in catalog 6.6.65.
//!
//! The server answers one request per connection and serves connections
//! until it is dropped. A thread per connection would buy nothing: the run
//! opens them one at a time.

use std::fs::File;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use audhsos_time::CivilTime;
use audhsos_tls::client::{Buffers, Event, MIN_HANDSHAKE, MIN_INCOMING, MIN_OUTGOING};
use audhsos_tls::error::TlsError;
use audhsos_tls::handshake::ED25519;
use audhsos_tls::record::MAX_PLAINTEXT;
use audhsos_tls::server::{Connection, ServerConfig};
use audhsos_x509::builder::{MAX_CERTIFICATE, Params, TestKey, build};
use crypto_rng::{ChaChaRng, Entropy, EntropyError, Rng};

use crate::error::Error;
use crate::out::note;

/// The name the certificate carries and the client asks for.
pub(crate) const NAME: &str = "audhsos.test";

/// The secret of the authority the image trusts for this run.
const ROOT_SECRET: [u8; 32] = [0x51; 32];
/// The secret of the server's own key.
const LEAF_SECRET: [u8; 32] = [0x52; 32];
/// What a connection falls back to where the host device answers
/// nothing, which is a machine without `/dev/urandom`. A fixed value
/// costs a test server nothing: it presents the same certificate to every
/// caller anyway.
const FALLBACK_EPHEMERAL: [u8; 32] = [0x53; 32];

/// What the server answers every request with.
pub(crate) const RESPONSE: &str = "HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nhello, image";

/// How long a connection may take before the server drops it.
const TIMEOUT: Duration = Duration::from_secs(20);

/// How often the accept loop looks at the stop flag.
const TICK: Duration = Duration::from_millis(100);

/// One certificate and how many bytes of the array it takes.
pub(crate) struct Certificate {
    /// The encoding.
    bytes: [u8; MAX_CERTIFICATE],
    /// How much of it is the certificate.
    length: usize,
}

impl Certificate {
    /// The encoded certificate.
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.length).unwrap_or(&[])
    }
}

/// What the run needs on both sides: the chain the server presents and
/// the root the image has to carry to believe it.
pub(crate) struct Material {
    /// The root, which is the anchor of the run.
    pub(crate) root: Certificate,
    /// The leaf, issued by that root for [`NAME`].
    pub(crate) leaf: Certificate,
}

impl Material {
    /// A root and a leaf, valid over a window that contains every clock a
    /// run of this project meets.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] when the builder refuses the parameters, which
    /// only a change in this function can cause.
    pub(crate) fn new() -> Result<Material, Error> {
        let from = CivilTime {
            year: 2020,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let until = CivilTime {
            year: 2035,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        };
        let root = certificate(
            &Params::authority("AuDHSOS Test Root", "AuDHSOS Test Root", None, from, until),
            TestKey::Ed25519(ROOT_SECRET),
            TestKey::Ed25519(ROOT_SECRET),
        )?;
        let names = [NAME];
        let leaf = certificate(
            &Params::leaf("AuDHSOS Test Root", NAME, &names, from, until),
            TestKey::Ed25519(LEAF_SECRET),
            TestKey::Ed25519(ROOT_SECRET),
        )?;
        Ok(Material { root, leaf })
    }
}

/// Builds one certificate.
fn certificate(
    params: &Params<'_>,
    subject: TestKey,
    issuer: TestKey,
) -> Result<Certificate, Error> {
    let mut bytes = [0u8; MAX_CERTIFICATE];
    let length = build(params, subject, issuer, &mut bytes)
        .map_err(|error| Error::Parse(format!("the test certificate: {error}")))?;
    Ok(Certificate { bytes, length })
}

/// The server, running on a thread of its own.
pub(crate) struct Server {
    /// The port the operating system gave it.
    port: u16,
    /// What tells the thread to stop.
    stop: Arc<AtomicBool>,
    /// The thread, which [`Server::drop`] joins.
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts a server on a free port of the loopback, presenting the
    /// chain of `material`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the port cannot be taken.
    pub(crate) fn start(material: &Material) -> Result<Server, Error> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|source| Error::io("binding the TLS test server".to_owned(), source))?;
        let port = listener
            .local_addr()
            .map_err(|source| Error::io("reading the port of the TLS server".to_owned(), source))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|source| Error::io("the TLS server's listener".to_owned(), source))?;

        let leaf = material.leaf.as_slice().to_vec();
        let root = material.root.as_slice().to_vec();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || serve(&listener, &leaf, &root, &flag));
        Ok(Server {
            port,
            stop,
            thread: Some(thread),
        })
    }

    /// The port it listens on.
    #[must_use]
    pub(crate) const fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

/// Takes connections until the flag is set, answering each one.
fn serve(listener: &TcpListener, leaf: &[u8], root: &[u8], stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                // What one connection does wrong is that connection's
                // business: the run reports what the guest saw.
                let _served = answer(stream, leaf, root);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(TICK);
            }
            Err(_) => return,
        }
    }
}

/// Runs one connection: the handshake, one request, one response, and the
/// close.
fn answer(mut stream: TcpStream, leaf: &[u8], root: &[u8]) -> Result<(), TlsError> {
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let _ = stream.set_nonblocking(false);

    let chain: [&[u8]; 2] = [leaf, root];
    let config = ServerConfig {
        ephemeral: ephemeral(),
        ..ServerConfig::new(&chain, TestKey::Ed25519(LEAF_SECRET), ED25519)
    };
    let mut incoming = vec![0u8; MIN_INCOMING];
    let mut outgoing = vec![0u8; MIN_OUTGOING];
    let mut handshake = vec![0u8; MIN_HANDSHAKE];
    let mut connection = Connection::new(
        &config,
        Buffers {
            incoming: &mut incoming,
            outgoing: &mut outgoing,
            handshake: &mut handshake,
        },
    )?;

    let mut wire = vec![0u8; MAX_PLAINTEXT + 512];
    let mut plaintext = vec![0u8; MAX_PLAINTEXT];
    loop {
        match connection.poll()? {
            Event::WantsWrite => {
                let length = connection.write_tls(&mut wire)?;
                if stream.write_all(wire.get(..length).unwrap_or(&[])).is_err() {
                    return Ok(());
                }
            }
            Event::WantsRead => {
                let read = stream.read(&mut wire).unwrap_or(0);
                if read == 0 {
                    return Ok(());
                }
                connection.read_tls(wire.get(..read).unwrap_or(&[]))?;
            }
            Event::Handshaked => break,
            Event::PeerClosed => return Ok(()),
        }
    }

    // One request, whose content is not read: the run checks what the
    // client makes of the answer, and a server that parsed HTTP here would
    // be answering a question this project does not ask.
    let taken = read_more(&mut stream, &mut connection, &mut wire, &mut plaintext)?;
    if taken == 0 {
        return Ok(());
    }
    connection.send(RESPONSE.as_bytes())?;
    connection.close()?;
    flush(&mut stream, &mut connection, &mut wire)?;
    Ok(())
}

/// A private value for one connection, out of the host's device.
fn ephemeral() -> [u8; 32] {
    let mut value = FALLBACK_EPHEMERAL;
    let Ok(mut device) = File::open("/dev/urandom") else {
        return value;
    };
    let mut seed = [0u8; 32];
    if device.read_exact(&mut seed).is_err() {
        return value;
    }
    let mut rng = ChaChaRng::from_seed(&seed, Device { device });
    if rng.fill(&mut value).is_err() {
        return FALLBACK_EPHEMERAL;
    }
    value
}

/// The host's entropy, which stands in for the `random_bytes` system call
/// the guest has.
struct Device {
    /// `/dev/urandom`, held open while the generator reseeds from it.
    device: File,
}

impl Entropy for Device {
    fn fill(&mut self, out: &mut [u8]) -> Result<(), EntropyError> {
        self.device
            .read_exact(out)
            .map_err(|_| EntropyError::Unavailable)
    }
}

/// Reads until one record of application data has arrived.
fn read_more(
    stream: &mut TcpStream,
    connection: &mut Connection<'_>,
    wire: &mut [u8],
    plaintext: &mut [u8],
) -> Result<usize, TlsError> {
    loop {
        let length = connection.recv(plaintext)?;
        if length != 0 {
            return Ok(length);
        }
        let read = stream.read(wire).unwrap_or(0);
        if read == 0 {
            return Ok(0);
        }
        connection.read_tls(wire.get(..read).unwrap_or(&[]))?;
    }
}

/// Hands everything the connection has written to the socket.
fn flush(
    stream: &mut TcpStream,
    connection: &mut Connection<'_>,
    wire: &mut [u8],
) -> Result<(), TlsError> {
    loop {
        let length = connection.write_tls(wire)?;
        if length == 0 {
            return Ok(());
        }
        if stream.write_all(wire.get(..length).unwrap_or(&[])).is_err() {
            return Ok(());
        }
    }
}

/// `tls-server`: starts the server and waits, so that a person can point
/// `tools/tls-probe` or the program of the image at it while Phase 15 is
/// being written.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option, and whatever the chain and the
/// port answer.
pub(crate) fn command(options: &[String]) -> Result<(), Error> {
    if let Some(option) = options.first() {
        return Err(Error::Usage(format!(
            "unknown option `{option}` for tls-server"
        )));
    }
    let material = Material::new()?;
    let server = Server::start(&material)?;
    note!(
        "tls-server: https://{NAME}:{port} on the loopback, answering `{RESPONSE}`",
        port = server.port()
    );
    note!("tls-server: the anchor is the root of the chain; stop it with ctrl-c");
    loop {
        std::thread::sleep(TICK);
    }
}

/// What the acceptance run puts on the scratch volume for the program of
/// the image (D-150).
pub(crate) mod guest {
    /// The file naming the port of the server and the name its
    /// certificate carries.
    ///
    /// Both files lie in the root of the volume and carry 8.3 names, as
    /// the files of the Secure Shell run do: a program that opened a
    /// directory first would read a path for no gain.
    pub(crate) const CONFIG: &str = "TLSCONF.TXT";
    /// The file holding the root of the run's chain, as DER.
    pub(crate) const ROOT: &str = "TLSROOT.DER";
}

/// The files of one run: where the server listens, what name it answers
/// for, and the root its chain reaches.
///
/// The root travels here and not in the anchor table of the boot volume,
/// which carries the public roots an operator dropped into `anchors/`
/// (D-148, D-150). An image whose table held this root would trust the
/// project's test authority on every machine it ever booted on.
pub(crate) fn scratch_files(port: u16, material: &Material) -> Vec<(String, Vec<u8>)> {
    let config = format!("port {port}\nname {NAME}\n");
    vec![
        (guest::CONFIG.to_owned(), config.into_bytes()),
        (guest::ROOT.to_owned(), material.root.as_slice().to_vec()),
    ]
}
