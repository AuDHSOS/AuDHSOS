// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One request of the file protocol, answered against a mounted volume.
//!
//! Nothing here reaches the machine: [`answer`] takes the volume, the open
//! files, the badge of the sender and the moment a new entry carries, and
//! gives back the reply the server sends. The process around it brings the
//! disk, the endpoint and the clock.
//!
//! Invariants: a handle is looked up under the badge it arrived with, so
//! no client reaches what another opened; every refusal of `fs-fat` leaves
//! the tables as they were.

use audhsos_abi::Error;
use audhsos_time::UnixTime;
use fs_fat::{ATTR_DIRECTORY, Dir, FileSystem, Name, SECTOR};
use fs_fat::{BlockDevice, Entry};
use user_proto::file::{
    Data, DirEntry, MAX_DATA, Name as ProtoName, Opened as ProtoOpened, ROOT, Reply, Request,
    START, Stat,
};

use crate::error::refusal;
use crate::open::{Clients, Opened};

/// Answers `request`, which arrived under `badge`.
///
/// `now` is what a new entry is stamped with; it comes from the wall clock
/// of the process (D-137).
pub fn answer<D: BlockDevice>(
    fs: &mut FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    request: &Request,
    now: UnixTime,
) -> Reply {
    match request {
        Request::Open { parent, name } => Reply::Opened(open(fs, clients, badge, *parent, name)),
        Request::Create {
            parent,
            name,
            directory,
        } => Reply::Created(create(fs, clients, badge, *parent, name, *directory, now)),
        Request::Read { file, offset, len } => {
            Reply::Read(read(fs, clients, badge, *file, *offset, *len))
        }
        Request::Write { file, offset, data } => {
            Reply::Written(write(fs, clients, badge, *file, *offset, data))
        }
        Request::ReadDir { dir, cursor } => {
            Reply::Entry(read_dir(fs, clients, badge, *dir, *cursor))
        }
        Request::Stat { file } => Reply::Stat(stat(fs, clients, badge, *file)),
        Request::Remove { parent, name } => {
            Reply::Removed(remove(fs, clients, badge, *parent, name))
        }
        Request::Close { file } => Reply::Closed(close(clients, badge, *file)),
        Request::Flush => Reply::Flushed(fs.flush().map_err(refusal)),
    }
}

/// The directory `handle` of `badge` stands for.
///
/// [`ROOT`] is the root of the volume, which every client holds without
/// opening it.
fn directory<D: BlockDevice>(
    fs: &FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
) -> Result<Dir, Error> {
    if handle == ROOT {
        return Ok(fs.root());
    }
    clients
        .get(badge, handle)
        .ok_or(Error::InvalidHandle)?
        .as_dir()
        .ok_or(Error::WrongObjectType)
}

/// The name of the protocol as `fs-fat` spells it.
fn name_of(name: &ProtoName) -> Result<Name, Error> {
    let text = core::str::from_utf8(name.as_bytes()).map_err(|_| Error::InvalidArgument)?;
    Name::new(text).map_err(refusal)
}

/// `open`: the handle of an entry that is there.
fn open<D: BlockDevice>(
    fs: &FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    parent: u32,
    name: &ProtoName,
) -> Result<ProtoOpened, Error> {
    let dir = directory(fs, clients, badge, parent)?;
    let wanted = name_of(name)?;
    let entry = fs
        .find(dir, &wanted)
        .map_err(refusal)?
        .ok_or(Error::NotFound)?;
    let opened = if entry.is_directory() {
        Opened::Dir {
            dir: Dir::at(entry.first_cluster),
            walk: fs.entries(Dir::at(entry.first_cluster)),
            handed: START,
        }
    } else {
        Opened::File {
            file: fs.open(dir, &wanted).map_err(refusal)?,
            parent: dir,
            name: wanted,
        }
    };
    let file = clients.insert(badge, opened).ok_or(Error::OutOfHandles)?;
    Ok(ProtoOpened {
        file,
        size: entry.size,
        directory: entry.is_directory(),
    })
}

/// `create`: the handle of an entry made now.
fn create<D: BlockDevice>(
    fs: &mut FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    parent: u32,
    name: &ProtoName,
    directory: bool,
    now: UnixTime,
) -> Result<u32, Error> {
    let dir = self::directory(fs, clients, badge, parent)?;
    let wanted = name_of(name)?;
    let opened = if directory {
        let made = fs.create_dir(dir, &wanted, now).map_err(refusal)?;
        Opened::Dir {
            dir: made,
            walk: fs.entries(made),
            handed: START,
        }
    } else {
        Opened::File {
            file: fs.create(dir, &wanted, now).map_err(refusal)?,
            parent: dir,
            name: wanted,
        }
    };
    clients.insert(badge, opened).ok_or(Error::OutOfHandles)
}

/// `read`: bytes out of a file.
fn read<D: BlockDevice>(
    fs: &FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
    offset: u32,
    len: u32,
) -> Result<Data, Error> {
    let wanted = usize::try_from(len).unwrap_or(MAX_DATA).min(MAX_DATA);
    let Opened::File { file, .. } = clients.get(badge, handle).ok_or(Error::InvalidHandle)? else {
        return Err(Error::WrongObjectType);
    };
    let mut into = [0u8; MAX_DATA];
    let taken = fs
        .read(file, offset, into.get_mut(..wanted).unwrap_or(&mut []))
        .map_err(refusal)?;
    Data::new(into.get(..taken).unwrap_or(&[])).map_err(|_| Error::BufferTooSmall)
}

/// `write`: bytes into a file.
fn write<D: BlockDevice>(
    fs: &mut FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
    offset: u32,
    data: &Data,
) -> Result<u32, Error> {
    let Opened::File { file, .. } = clients.get(badge, handle).ok_or(Error::InvalidHandle)? else {
        return Err(Error::WrongObjectType);
    };
    let taken = fs.write(file, offset, data.as_bytes()).map_err(refusal)?;
    u32::try_from(taken).map_err(|_| Error::InvalidArgument)
}

/// `read_dir`: one entry of a directory and where to carry on.
///
/// The cursor is how many entries the client has been given. A cursor that
/// is not where the walk stands starts the walk again and skips forward,
/// so a client that asks twice for the same entry gets the same one.
fn read_dir<D: BlockDevice>(
    fs: &FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
    cursor: u64,
) -> Result<Option<DirEntry>, Error> {
    let root = fs.root();
    let (dir, walk, handed) = match handle {
        // The root occupies no slot, so the client's walk over it is kept
        // beside the table rather than in it.
        ROOT => {
            let held = clients
                .root_walk(badge, || fs.entries(root))
                .ok_or(Error::OutOfHandles)?;
            (root, &mut held.0, &mut held.1)
        }
        other => match clients.get(badge, other).ok_or(Error::InvalidHandle)? {
            Opened::Dir { dir, walk, handed } => (*dir, walk, handed),
            Opened::File { .. } => return Err(Error::WrongObjectType),
        },
    };
    if *handed != cursor {
        *walk = fs.entries(dir);
        *handed = START;
        while *handed < cursor {
            if fs.next_entry(walk).map_err(refusal)?.is_none() {
                return Ok(None);
            }
            *handed = handed.saturating_add(1);
        }
    }
    let Some(entry) = fs.next_entry(walk).map_err(refusal)? else {
        return Ok(None);
    };
    *handed = handed.saturating_add(1);
    Ok(Some(DirEntry {
        name: spelled(&entry)?,
        size: entry.size,
        directory: entry.is_directory(),
        cursor: *handed,
    }))
}

/// `stat`: what a file handle stands for.
fn stat<D: BlockDevice>(
    fs: &FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
) -> Result<Stat, Error> {
    if handle == ROOT {
        return Ok(Stat {
            size: 0,
            attributes: u32::from(ATTR_DIRECTORY),
            modified: 0,
        });
    }
    let (parent, name) = match clients.get(badge, handle).ok_or(Error::InvalidHandle)? {
        Opened::File { parent, name, .. } => (*parent, *name),
        Opened::Dir { .. } => {
            return Ok(Stat {
                size: 0,
                attributes: u32::from(ATTR_DIRECTORY),
                modified: 0,
            });
        }
    };
    let entry = fs
        .find(parent, &name)
        .map_err(refusal)?
        .ok_or(Error::NotFound)?;
    Ok(Stat {
        size: entry.size,
        attributes: u32::from(entry.attributes),
        modified: u64::try_from(entry.modified.seconds()).unwrap_or(0),
    })
}

/// `remove`: take an entry off the volume.
fn remove<D: BlockDevice>(
    fs: &mut FileSystem<D>,
    clients: &mut Clients,
    badge: u64,
    parent: u32,
    name: &ProtoName,
) -> Result<(), Error> {
    let dir = directory(fs, clients, badge, parent)?;
    let wanted = name_of(name)?;
    fs.remove(dir, &wanted).map_err(refusal)?;
    Ok(())
}

/// `close`: give a file handle back.
fn close(clients: &mut Clients, badge: u64, handle: u32) -> Result<(), Error> {
    if clients.remove(badge, handle) {
        return Ok(());
    }
    Err(Error::InvalidHandle)
}

/// The name of an entry as a client writes it: `STEM.EXT`, or `STEM` where
/// there is no extension.
fn spelled(entry: &Entry) -> Result<ProtoName, Error> {
    let bytes = entry.name.as_bytes();
    let stem = trimmed(bytes.get(..8).unwrap_or(&[]));
    let extension = trimmed(bytes.get(8..).unwrap_or(&[]));
    let mut spelled = [0u8; 12];
    let mut len = 0usize;
    for byte in stem.iter().chain(dot(extension)).chain(extension) {
        if let Some(slot) = spelled.get_mut(len) {
            *slot = *byte;
            len = len.saturating_add(1);
        }
    }
    ProtoName::new(spelled.get(..len).unwrap_or(&[])).map_err(|_| Error::BufferTooSmall)
}

/// The bytes of a field without the spaces it is padded with.
fn trimmed(field: &[u8]) -> &[u8] {
    let len = field
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |at| at.saturating_add(1));
    field.get(..len).unwrap_or(&[])
}

/// The dot between a stem and an extension, and nothing where there is no
/// extension.
const fn dot(extension: &[u8]) -> &'static [u8] {
    if extension.is_empty() { &[] } else { b"." }
}

/// How many bytes one sector holds, which is what a client's read is
/// bounded by beside [`MAX_DATA`].
pub const SECTOR_BYTES: usize = SECTOR;

/// The moment a new entry carries, from the microseconds the wall clock
/// answers.
///
/// A directory entry holds the seconds in units of two from a date in
/// 1980, and `fs-fat` refuses anything else rather than rounding it
/// quietly. This is where the rounding happens: down to an even second,
/// and up to the first moment the format has, so that a machine whose
/// firmware named no clock still writes files.
#[must_use]
pub fn moment(micros: u64) -> UnixTime {
    let seconds = micros.wrapping_div(MICROS_PER_SECOND) & !1;
    UnixTime::from_seconds(i64::try_from(seconds.max(FAT_EPOCH)).unwrap_or(EPOCH_SECONDS))
}

/// Microseconds in one second.
const MICROS_PER_SECOND: u64 = 1_000_000;

/// Seconds from 1970-01-01 to 1980-01-01, which is the first moment a
/// directory entry can carry.
const FAT_EPOCH: u64 = 315_532_800;

/// The same as a signed count, for a value that does not fit `i64`.
const EPOCH_SECONDS: i64 = 315_532_800;
