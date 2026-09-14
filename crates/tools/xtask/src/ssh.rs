// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The key material and the OpenSSH server of the Secure Shell interop
//! run.
//!
//! The run is the outside check of
//! [14.12](../../../docs/14-secure-shell-as-a-client.md): no document
//! publishes a complete SSH handshake with the keys that made it, so what
//! says the client agrees with anybody is a handshake against an
//! implementation this project did not write.
//!
//! Where the material comes from is D-146. `ssh-keygen` writes a host key
//! and a client key into a directory of this checkout that `.gitignore`
//! names, so no key of this repository is tracked and a person may drop
//! further public keys beside them. Every `.pub` file of that directory
//! contributes a fingerprint, and the fingerprints, the client's secret
//! and the port reach the guest on the scratch disk of the run.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use audhsos_encoding::base64;
use audhsos_encoding::pem;
use crypto_hash::Sha256;

use crate::error::Error;
use crate::fs;
use crate::process::Cmd;

/// Where the keys of the interop run lie, relative to the checkout.
pub(crate) const KEY_DIR: &str = "keys/ssh";

/// The host key the server of the run is given.
const HOST_KEY: &str = "host_ed25519";

/// The key the client of the image authenticates with.
const CLIENT_KEY: &str = "client_ed25519";

/// How long a `SHA256:` fingerprint line may be: the prefix and the
/// unpadded Base64 of thirty-two octets.
const FINGERPRINT_TEXT: usize = 7 + 43;

/// The seed of an Ed25519 key, which is what `auth::ClientKey` takes.
const SEED_LEN: usize = 32;

/// The private half of an Ed25519 key as OpenSSH stores it: the seed and
/// the public key after it.
const PRIVATE_LEN: usize = 64;

/// The public half, and the digest a fingerprint is taken of.
const KEY_LEN: usize = 32;

/// What the body lines of an OpenSSH private key are wrapped to. No
/// standard fixes the width of a text that is not RFC 7468, so the reader
/// is told what to expect rather than left to guess.
const OPENSSH_WRAP: usize = 70;

/// The label OpenSSH puts on a private key file.
const OPENSSH_LABEL: &str = "OPENSSH PRIVATE KEY";

/// What `PROTOCOL.key` begins with.
const OPENSSH_MAGIC: &[u8] = b"openssh-key-v1\0";

/// The key algorithm of 14.5, which is the only one either file holds.
const ED25519: &[u8] = b"ssh-ed25519";

/// How long the runner waits for the server to take its port.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(10);

/// How long it sleeps between two attempts at that port.
const LISTEN_STEP: Duration = Duration::from_millis(50);

/// The key material of the run.
pub(crate) struct Material {
    /// The private host key, which the server is given.
    pub(crate) host_key: PathBuf,
    /// The client's public key, which the server admits.
    pub(crate) authorized: PathBuf,
    /// The client's secret, which the guest signs with.
    pub(crate) seed: [u8; SEED_LEN],
    /// The fingerprint of every public key of the directory, as OpenSSH
    /// prints them.
    pub(crate) fingerprints: Vec<String>,
}

/// The key material under `root`, generated where it is not there.
///
/// # Errors
///
/// [`Error::Usage`] when `ssh-keygen` is missing or writes a file this
/// cannot read, and [`Error::Io`] for a directory that cannot be made or
/// read.
pub(crate) fn material(root: &Path) -> Result<Material, Error> {
    let directory = root.join(KEY_DIR);
    std::fs::create_dir_all(&directory)
        .map_err(|source| Error::io(format!("making {}", directory.display()), source))?;
    let host_key = directory.join(HOST_KEY);
    let client_key = directory.join(CLIENT_KEY);
    generate(&host_key, "audhsos interop host")?;
    generate(&client_key, "audhsos interop client")?;

    let seed = seed_of(&fs::read_bytes(&client_key)?)?;
    let fingerprints = fingerprints(&directory)?;
    Ok(Material {
        host_key,
        authorized: directory.join(format!("{CLIENT_KEY}.pub")),
        seed,
        fingerprints,
    })
}

/// Writes an Ed25519 key pair at `path` where either half is missing.
///
/// Both halves are asked for, not just the private one: every `*.pub` of
/// the directory carries a fingerprint into the trust file (D-146), so a
/// public half that is gone is a host the client stops admitting, and
/// `ssh-keygen` writes no public half beside a private one that already
/// stands.
fn generate(path: &Path, comment: &str) -> Result<(), Error> {
    let public = path.with_extension("pub");
    if path.exists() && public.exists() {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(&public);
    Cmd::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            comment,
            "-f",
            &path.display().to_string(),
        ])
        .run()
}

/// The fingerprints of every public key of `directory`, sorted so that
/// two runs write the same file.
fn fingerprints(directory: &Path) -> Result<Vec<String>, Error> {
    let entries = std::fs::read_dir(directory)
        .map_err(|source| Error::io(format!("reading {}", directory.display()), source))?;
    let mut lines = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|source| Error::io(format!("reading {}", directory.display()), source))?
            .path();
        if fs::extension(&path) != "pub" {
            continue;
        }
        lines.push(fingerprint_of(&fs::read_bytes(&path)?)?);
    }
    lines.sort_unstable();
    lines.dedup();
    if lines.is_empty() {
        return Err(Error::Usage(format!(
            "{} holds no public key to trust",
            directory.display()
        )));
    }
    Ok(lines)
}

/// The fingerprint of the key `line` holds, in the form OpenSSH prints
/// after `SHA256:`.
///
/// The line is `authorized_keys` as RFC 4253, section 6.6, leaves it to
/// OpenSSH: an algorithm name, the Base64 of the key blob, and a comment.
///
/// # Errors
///
/// [`Error::Parse`] for a line that names another algorithm, carries no
/// blob, or carries one this cannot decode.
pub(crate) fn fingerprint_of(line: &[u8]) -> Result<String, Error> {
    let text = core::str::from_utf8(line)
        .map_err(|_| Error::Parse("a public key that is not text".to_owned()))?;
    let first = text
        .lines()
        .next()
        .ok_or_else(|| Error::Parse("an empty public key".to_owned()))?;
    let mut fields = first.split_whitespace();
    let algorithm = fields
        .next()
        .ok_or_else(|| Error::Parse("a public key with no algorithm".to_owned()))?;
    if algorithm.as_bytes() != ED25519 {
        return Err(Error::Parse(format!(
            "`{algorithm}` is not the one host key algorithm of 14.5"
        )));
    }
    let body = fields
        .next()
        .ok_or_else(|| Error::Parse("a public key with no blob".to_owned()))?;
    let mut blob = vec![0u8; body.len()];
    let len = base64::decode(body.as_bytes(), &mut blob)
        .map_err(|error| Error::Parse(format!("a public key that is not Base64: {error}")))?;
    let blob = blob.get(..len).unwrap_or(&[]);
    Ok(print_fingerprint(&Sha256::digest(blob)))
}

/// The digest as OpenSSH prints it: `SHA256:` and unpadded Base64.
fn print_fingerprint(digest: &[u8; KEY_LEN]) -> String {
    let mut text = vec![0u8; FINGERPRINT_TEXT.saturating_add(4)];
    let len = base64::encode(digest, &mut text).unwrap_or(0);
    let printed = text.get(..len).unwrap_or(&[]);
    let trimmed = String::from_utf8_lossy(printed)
        .trim_end_matches('=')
        .to_owned();
    format!("SHA256:{trimmed}")
}

/// The seed of the key `bytes` holds, out of the file format of
/// `docs/openssh/PROTOCOL.key`.
///
/// Only the unencrypted form is read, which is what `ssh-keygen -N ""`
/// writes: a key under a passphrase would need the key derivation of that
/// document and a cipher, and nothing here has a passphrase to give.
///
/// # Errors
///
/// [`Error::Parse`] for a file this does not read: another envelope,
/// another magic, a cipher, more than one key, or another algorithm.
pub(crate) fn seed_of(bytes: &[u8]) -> Result<[u8; SEED_LEN], Error> {
    let wrap = core::num::NonZeroUsize::new(OPENSSH_WRAP)
        .ok_or_else(|| Error::Parse("a wrap of zero".to_owned()))?;
    let mut decoded = vec![0u8; bytes.len()];
    let block = pem::decode_wrapped(bytes, &mut decoded, wrap)
        .map_err(|error| Error::Parse(format!("a private key that is not PEM: {error}")))?;
    if block.label != OPENSSH_LABEL {
        return Err(Error::Parse(format!(
            "a private key labelled `{}` and not `{OPENSSH_LABEL}`",
            block.label
        )));
    }
    read_seed(block.bytes)
}

/// The seed out of the decoded body.
fn read_seed(body: &[u8]) -> Result<[u8; SEED_LEN], Error> {
    let mut reader = Reader::new(body);
    if reader.take(OPENSSH_MAGIC.len())? != OPENSSH_MAGIC {
        return Err(Error::Parse("a private key of another format".to_owned()));
    }
    for name in ["cipher", "key derivation"] {
        if reader.string()? != b"none" {
            return Err(Error::Parse(format!(
                "a private key under a {name}, which needs a passphrase"
            )));
        }
    }
    let _options = reader.string()?;
    if reader.u32()? != 1 {
        return Err(Error::Parse(
            "a private key file holding more than one key".to_owned(),
        ));
    }
    let _public = reader.string()?;
    let private = reader.string()?;

    let mut inner = Reader::new(private);
    let first = inner.u32()?;
    if first != inner.u32()? {
        return Err(Error::Parse(
            "a private key whose two check words differ".to_owned(),
        ));
    }
    if inner.string()? != ED25519 {
        return Err(Error::Parse(
            "a private key that is not `ssh-ed25519`".to_owned(),
        ));
    }
    let _key = inner.string()?;
    let secret = inner.string()?;
    if secret.len() != PRIVATE_LEN {
        return Err(Error::Parse(format!(
            "a private key of {} octets and not {PRIVATE_LEN}",
            secret.len()
        )));
    }
    let seed: [u8; SEED_LEN] = secret
        .get(..SEED_LEN)
        .and_then(|head| head.try_into().ok())
        .ok_or_else(|| Error::Parse("a private key with no seed".to_owned()))?;
    Ok(seed)
}

/// The length-prefixed strings of RFC 4251, section 5, as the host reads
/// them. `audhsos-ssh` has a reader of its own; this one answers the
/// error type of the xtask and borrows from a `Vec`.
struct Reader<'a> {
    /// What is left to read.
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader over `bytes`.
    const fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { rest: bytes }
    }

    /// The next `len` octets.
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let head = self
            .rest
            .get(..len)
            .ok_or_else(|| Error::Parse("a private key that ends early".to_owned()))?;
        self.rest = self.rest.get(len..).unwrap_or(&[]);
        Ok(head)
    }

    /// The next `uint32`.
    fn u32(&mut self) -> Result<u32, Error> {
        let head: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| Error::Parse("a private key that ends early".to_owned()))?;
        Ok(u32::from_be_bytes(head))
    }

    /// The next `string`.
    fn string(&mut self) -> Result<&'a [u8], Error> {
        let len = usize::try_from(self.u32()?)
            .map_err(|_| Error::Parse("a string longer than this machine".to_owned()))?;
        self.take(len)
    }
}

/// The OpenSSH server of the run, which ends when this is dropped.
pub(crate) struct Server {
    /// The process.
    child: Child,
    /// The loopback port it took, which the guest reaches at the gateway
    /// of its user-mode network.
    port: u16,
    /// Where its configuration and its log lie.
    directory: PathBuf,
}

impl Server {
    /// Starts `sshd` on a free port of the loopback, with the host key and
    /// the authorized key of `material`.
    ///
    /// The server runs as the person who started the check and
    /// authenticates that same account, which is what `sshd` without
    /// privileges can do; nothing here needs a password or a root.
    ///
    /// # Errors
    ///
    /// [`Error::Usage`] when no `sshd` is installed, when no port is free,
    /// or when the server did not take its port within
    /// [`LISTEN_TIMEOUT`], and [`Error::Io`] when the configuration cannot
    /// be written.
    pub(crate) fn start(root: &Path, material: &Material) -> Result<Server, Error> {
        let program = locate()?;
        let port = crate::qemu::free_port()
            .ok_or_else(|| Error::Usage("no free port for the interop server".to_owned()))?;
        let directory = root.join("target").join("qemu").join("sshd");
        std::fs::create_dir_all(&directory)
            .map_err(|source| Error::io(format!("making {}", directory.display()), source))?;
        let config = directory.join("sshd_config");
        fs::write_bytes(
            &config,
            configuration(port, material, &directory).as_bytes(),
        )?;

        let log = std::fs::File::create(directory.join("sshd.log"))
            .map_err(|source| Error::io("making the log of the interop server", source))?;
        let errors = log
            .try_clone()
            .map_err(|source| Error::io("making the log of the interop server", source))?;
        let child = Command::new(&program)
            .args(["-f", &config.display().to_string(), "-D", "-e"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(errors))
            .spawn()
            .map_err(|source| Error::io(format!("starting {}", program.display()), source))?;

        let mut server = Server {
            child,
            port,
            directory,
        };
        server.wait_for_the_port()?;
        Ok(server)
    }

    /// The port the guest connects to.
    pub(crate) const fn port(&self) -> u16 {
        self.port
    }

    /// What the server wrote, for a run that failed.
    pub(crate) fn log(&self) -> String {
        std::fs::read_to_string(self.directory.join("sshd.log")).unwrap_or_default()
    }

    /// Waits until the port answers, so that the guest does not reach a
    /// server that is not listening yet.
    ///
    /// Whether the child is still running is asked before the port is,
    /// because `free_port` lets its port go before `sshd` takes it: a
    /// second process that took it in between answers a connection and
    /// would otherwise read as a server that is up, while the `sshd` that
    /// could not bind has already exited.
    fn wait_for_the_port(&mut self) -> Result<(), Error> {
        let until = Instant::now().checked_add(LISTEN_TIMEOUT);
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(Error::Usage(format!(
                    "the interop server ended before it served ({status}): {}",
                    self.log()
                )));
            }
            if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return Ok(());
            }
            if until.is_none_or(|deadline| Instant::now() >= deadline) {
                return Err(Error::Usage(format!(
                    "the interop server did not take port {}: {}",
                    self.port,
                    self.log()
                )));
            }
            std::thread::sleep(LISTEN_STEP);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The configuration the server is started with.
///
/// Every algorithm of 14.5 is named, because an OpenSSH of this decade
/// offers neither `diffie-hellman-group14-sha256` nor a single host key
/// algorithm by default, and a run that negotiated whatever both sides
/// happened to prefer would say nothing about the set this client offers.
///
/// The three paths are quoted, because sshd splits a keyword's arguments
/// on whitespace and a checkout whose path holds a space would otherwise
/// make it refuse the file with `extra arguments at end of line`.
fn configuration(port: u16, material: &Material, directory: &Path) -> String {
    format!(
        "Port {port}\n\
         ListenAddress 127.0.0.1\n\
         HostKey \"{host}\"\n\
         AuthorizedKeysFile \"{authorized}\"\n\
         PidFile \"{pid}\"\n\
         StrictModes no\n\
         UsePAM no\n\
         PasswordAuthentication no\n\
         KbdInteractiveAuthentication no\n\
         PermitRootLogin no\n\
         KexAlgorithms curve25519-sha256,diffie-hellman-group14-sha256\n\
         HostKeyAlgorithms ssh-ed25519\n\
         PubkeyAcceptedAlgorithms ssh-ed25519\n\
         Ciphers chacha20-poly1305@openssh.com\n\
         LogLevel DEBUG1\n",
        host = material.host_key.display(),
        authorized = material.authorized.display(),
        pid = directory.join("sshd.pid").display(),
    )
}

/// Where `sshd` is on this machine.
fn locate() -> Result<PathBuf, Error> {
    for path in ["/usr/sbin/sshd", "/usr/local/sbin/sshd", "/sbin/sshd"] {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(Error::Usage(
        "no `sshd` on this machine; the Secure Shell interop run needs one \
         (Debian and Ubuntu call the package `openssh-server`)"
            .to_owned(),
    ))
}

/// The account the client authenticates as, which is the one that started
/// the check.
///
/// # Errors
///
/// [`Error::Usage`] when the environment names none.
pub(crate) fn account() -> Result<String, Error> {
    for name in ["USER", "LOGNAME"] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            return Ok(value);
        }
    }
    Err(Error::Usage(
        "neither `USER` nor `LOGNAME` names an account for the interop run".to_owned(),
    ))
}

/// Where the guest reads the three files off the scratch volume, and what
/// the command it runs writes.
pub(crate) mod guest {
    /// The file holding the port, the account and the command.
    ///
    /// The three lie in the root of the volume and carry 8.3 names,
    /// because the volume is FAT32 and a program that opened a directory
    /// first would read a path for no gain.
    pub(crate) const CONFIG: &str = "SSHCONF.TXT";
    /// The file holding one fingerprint per line.
    pub(crate) const TRUST: &str = "SSHTRUST.TXT";
    /// The file holding the thirty-two octets of the client's seed.
    pub(crate) const SECRET: &str = "SSHKEY.BIN";
    /// What the command writes on its standard output.
    pub(crate) const SAID: &str = "audhsos-over-ssh";
    /// What it writes on its standard error, which reaches the client as
    /// the extended data of RFC 4254, section 5.2.
    pub(crate) const COMPLAINED: &str = "audhsos-on-stderr";
    /// What it exits with, which is not zero so that a client that reports
    /// no status cannot pass.
    pub(crate) const STATUS: u32 = 7;
}

/// The command the guest asks the server to run.
fn command() -> String {
    format!(
        "echo {said}; echo {complained} 1>&2; exit {status}",
        said = guest::SAID,
        complained = guest::COMPLAINED,
        status = guest::STATUS,
    )
}

/// The three files the scratch disk of the interop run carries.
pub(crate) fn scratch_files(
    port: u16,
    account: &str,
    material: &Material,
) -> Vec<(String, Vec<u8>)> {
    let config = format!(
        "port {port}\nuser {account}\ncommand {command}\n",
        command = command()
    );
    let mut trust = material.fingerprints.join("\n");
    trust.push('\n');
    vec![
        (guest::CONFIG.to_owned(), config.into_bytes()),
        (guest::TRUST.to_owned(), trust.into_bytes()),
        (guest::SECRET.to_owned(), material.seed.to_vec()),
    ]
}
