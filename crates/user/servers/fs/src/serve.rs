// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One request of the file protocol, answered against the volumes of the
//! machine.
//!
//! Nothing here reaches the machine: [`answer`] takes the volumes, the
//! open files, the badge of the sender and the moment a new entry carries,
//! and gives back the reply the server sends. The process around it brings
//! the disks, the endpoint and the clock.
//!
//! Two volumes, and a handle says which. `ROOT` is the root of the volume
//! the system writes and `BOOT` the root of the volume the machine booted
//! from; every handle the server chooses carries the volume it came from,
//! so no message names one.
//!
//! Invariants: a handle is looked up under the badge it arrived with, so
//! no client reaches what another opened; the volume the machine booted
//! from is never written, whatever a client asks; every refusal of
//! `fs-fat` leaves the tables as they were.

use audhsos_abi::Error;
use audhsos_time::UnixTime;
use fs_fat::{ATTR_DIRECTORY, BlockDevice, Dir, Entry, FileSystem, Name, SECTOR};
use user_proto::file::{
    BOOT, Data, DirEntry, MAX_DATA, Name as ProtoName, Opened as ProtoOpened, ROOT, Reply, Request,
    START, Stat,
};

use crate::error::refusal;
use crate::open::{Clients, Opened, Which};

/// How many bytes one sector holds, which is what a client's read is
/// bounded by beside [`MAX_DATA`].
pub const SECTOR_BYTES: usize = SECTOR;

/// The volumes a server answers over.
///
/// Both are optional, because a machine may carry one disk or none, and
/// which one it carries is not this crate's to know: a server answers
/// `NotFound` for the root of a volume it has not, and for everything
/// below it.
pub struct Volumes<'a, D: BlockDevice> {
    /// The volume the system writes, whose root is [`ROOT`].
    pub written: Option<&'a mut FileSystem<D>>,
    /// The volume the machine booted from, whose root is [`BOOT`].
    pub booted: Option<&'a mut FileSystem<D>>,
}

impl<D: BlockDevice> Volumes<'_, D> {
    /// The volume `which` names.
    fn get(&self, which: Which) -> Option<&FileSystem<D>> {
        match which {
            Which::Written => self.written.as_deref(),
            Which::Booted => self.booted.as_deref(),
        }
    }

    /// The same, for a message that changes the volume.
    ///
    /// The volume the machine booted from answers `None`: it is the
    /// firmware's and the loader's, and nothing in the system writes it
    /// (D-136).
    fn get_mut(&mut self, which: Which) -> Option<&mut FileSystem<D>> {
        match which {
            Which::Written => self.written.as_deref_mut(),
            Which::Booted => None,
        }
    }
}

/// The badge of a message that arrived through a capability without one.
///
/// Every client that found this server by name would speak under it, and
/// the open files of one are the open files of all, so it names nobody and
/// is refused.
pub const NOBODY: u64 = 0;

/// Answers `request`, which arrived under `badge`.
///
/// `now` is what a new entry is stamped with; it comes from the wall clock
/// of the process (D-137).
///
/// A request under [`NOBODY`] is refused with [`Error::AccessDenied`],
/// whatever it asks for.
pub fn answer<D: BlockDevice>(
    volumes: &mut Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    request: &Request,
    now: UnixTime,
) -> Reply {
    if badge == NOBODY {
        return refused(request, Error::AccessDenied);
    }
    match request {
        Request::Open { parent, name } => {
            Reply::Opened(open(volumes, clients, badge, *parent, name))
        }
        Request::Create {
            parent,
            name,
            directory,
        } => Reply::Created(create(
            volumes, clients, badge, *parent, name, *directory, now,
        )),
        Request::Read { file, offset, len } => {
            Reply::Read(read(volumes, clients, badge, *file, *offset, *len))
        }
        Request::Write { file, offset, data } => {
            Reply::Written(write(volumes, clients, badge, *file, *offset, data))
        }
        Request::ReadDir { dir, cursor } => {
            Reply::Entry(read_dir(volumes, clients, badge, *dir, *cursor))
        }
        Request::Stat { file } => Reply::Stat(stat(volumes, clients, badge, *file)),
        Request::Remove { parent, name } => {
            Reply::Removed(remove(volumes, clients, badge, *parent, name))
        }
        Request::Close { file } => Reply::Closed(close(clients, badge, *file)),
        Request::Flush => Reply::Flushed(flush(volumes)),
    }
}

/// Refuses `request` with `error`, in the reply the client waits for.
const fn refused(request: &Request, error: Error) -> Reply {
    match request {
        Request::Open { .. } => Reply::Opened(Err(error)),
        Request::Create { .. } => Reply::Created(Err(error)),
        Request::Read { .. } => Reply::Read(Err(error)),
        Request::Write { .. } => Reply::Written(Err(error)),
        Request::ReadDir { .. } => Reply::Entry(Err(error)),
        Request::Stat { .. } => Reply::Stat(Err(error)),
        Request::Remove { .. } => Reply::Removed(Err(error)),
        Request::Close { .. } => Reply::Closed(Err(error)),
        Request::Flush => Reply::Flushed(Err(error)),
    }
}

/// `flush`: puts what was written onto the disk.
fn flush<D: BlockDevice>(volumes: &mut Volumes<'_, D>) -> Result<(), Error> {
    volumes
        .get_mut(Which::Written)
        .ok_or(Error::NotFound)?
        .flush()
        .map_err(refusal)
}

/// Which volume `handle` is on, and the directory it stands for.
///
/// The two roots are the two volumes; every other handle carries the
/// volume it was opened on.
fn directory<D: BlockDevice>(
    volumes: &Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
) -> Result<(Which, Dir), Error> {
    let which = match handle {
        ROOT => Which::Written,
        BOOT => Which::Booted,
        other => {
            let opened = clients.get(badge, other).ok_or(Error::InvalidHandle)?;
            let dir = opened.as_dir().ok_or(Error::WrongObjectType)?;
            return Ok((opened.volume(), dir));
        }
    };
    let fs = volumes.get(which).ok_or(Error::NotFound)?;
    Ok((which, fs.root()))
}

/// Which volume `handle` is on, for a handle that is no directory.
fn volume_of(clients: &mut Clients, badge: u64, handle: u32) -> Result<Which, Error> {
    match handle {
        ROOT => Ok(Which::Written),
        BOOT => Ok(Which::Booted),
        other => Ok(clients
            .get(badge, other)
            .ok_or(Error::InvalidHandle)?
            .volume()),
    }
}

/// The name of the protocol as `fs-fat` spells it.
fn name_of(name: &ProtoName) -> Result<Name, Error> {
    let text = core::str::from_utf8(name.as_bytes()).map_err(|_| Error::InvalidArgument)?;
    Name::new(text).map_err(refusal)
}

/// `open`: the handle of an entry that is there.
fn open<D: BlockDevice>(
    volumes: &Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    parent: u32,
    name: &ProtoName,
) -> Result<ProtoOpened, Error> {
    let (volume, dir) = directory(volumes, clients, badge, parent)?;
    let fs = volumes.get(volume).ok_or(Error::NotFound)?;
    let wanted = name_of(name)?;
    let entry = fs
        .find(dir, &wanted)
        .map_err(refusal)?
        .ok_or(Error::NotFound)?;
    let opened = if entry.is_directory() {
        let found = Dir::at(entry.first_cluster);
        Opened::Dir {
            volume,
            dir: found,
            walk: fs.entries(found),
            handed: START,
        }
    } else {
        let file = fs.open_entry(&entry).map_err(refusal)?;
        let handle = clients
            .insert_file(badge, volume, file, dir, wanted)
            .ok_or(Error::OutOfHandles)?;
        return Ok(ProtoOpened {
            file: handle,
            size: entry.size,
            directory: false,
        });
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
    volumes: &mut Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    parent: u32,
    name: &ProtoName,
    directory: bool,
    now: UnixTime,
) -> Result<u32, Error> {
    let (volume, dir) = self::directory(volumes, clients, badge, parent)?;
    let fs = volumes.get_mut(volume).ok_or(Error::AccessDenied)?;
    let wanted = name_of(name)?;
    if !clients.has_room(badge) {
        return Err(Error::OutOfHandles);
    }
    let opened = if directory {
        let made = fs.create_dir(dir, &wanted, now).map_err(refusal)?;
        Opened::Dir {
            volume,
            dir: made,
            walk: fs.entries(made),
            handed: START,
        }
    } else {
        let file = fs.create(dir, &wanted, now).map_err(refusal)?;
        return clients
            .insert_file(badge, volume, file, dir, wanted)
            .ok_or(Error::OutOfHandles);
    };
    clients.insert(badge, opened).ok_or(Error::OutOfHandles)
}

/// `read`: bytes out of a file.
fn read<D: BlockDevice>(
    volumes: &Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
    offset: u32,
    len: u32,
) -> Result<Data, Error> {
    let which = volume_of(clients, badge, handle)?;
    let fs = volumes.get(which).ok_or(Error::NotFound)?;
    let wanted = usize::try_from(len).unwrap_or(MAX_DATA).min(MAX_DATA);
    let shared = match clients.get(badge, handle).ok_or(Error::InvalidHandle)? {
        Opened::File { shared, .. } => *shared,
        Opened::Dir { .. } => return Err(Error::WrongObjectType),
    };
    let file = clients.file_mut(shared).ok_or(Error::InvalidHandle)?;
    let mut into = [0u8; MAX_DATA];
    let taken = fs
        .read(file, offset, into.get_mut(..wanted).unwrap_or(&mut []))
        .map_err(refusal)?;
    Data::new(into.get(..taken).unwrap_or(&[])).map_err(|_| Error::BufferTooSmall)
}

/// `write`: bytes into a file.
fn write<D: BlockDevice>(
    volumes: &mut Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
    offset: u32,
    data: &Data,
) -> Result<u32, Error> {
    let which = volume_of(clients, badge, handle)?;
    let fs = volumes.get_mut(which).ok_or(Error::AccessDenied)?;
    let shared = match clients.get(badge, handle).ok_or(Error::InvalidHandle)? {
        Opened::File { shared, .. } => *shared,
        Opened::Dir { .. } => return Err(Error::WrongObjectType),
    };
    let file = clients.file_mut(shared).ok_or(Error::InvalidHandle)?;
    let taken = fs.write(file, offset, data.as_bytes()).map_err(refusal)?;
    u32::try_from(taken).map_err(|_| Error::InvalidArgument)
}

/// `read_dir`: one entry of a directory and where to carry on.
///
/// The cursor is how many entries the client has been given. A cursor that
/// is not where the walk stands starts the walk again and skips forward,
/// so a client that asks twice for the same entry gets the same one.
fn read_dir<D: BlockDevice>(
    volumes: &Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
    cursor: u64,
) -> Result<Option<DirEntry>, Error> {
    let (which, root) = directory(volumes, clients, badge, handle)?;
    let fs = volumes.get(which).ok_or(Error::NotFound)?;
    let (dir, walk, handed) = match handle {
        // A root occupies no slot, so the client's walk over it is kept
        // beside the table rather than in it.
        ROOT | BOOT => {
            let index = usize::from(handle == BOOT);
            let held = clients
                .root_walk(badge, index, || fs.entries(root))
                .ok_or(Error::OutOfHandles)?;
            (root, &mut held.0, &mut held.1)
        }
        other => match clients.get(badge, other).ok_or(Error::InvalidHandle)? {
            Opened::Dir {
                dir, walk, handed, ..
            } => (*dir, walk, handed),
            Opened::File { .. } => return Err(Error::WrongObjectType),
        },
    };
    let result = next_dir_entry(fs, dir, walk, handed, cursor);
    if matches!(handle, ROOT | BOOT) && matches!(result, Ok(None)) {
        let _cleared = clients.clear_root(badge, usize::from(handle == BOOT));
    }
    result
}

fn next_dir_entry<D: BlockDevice>(
    fs: &FileSystem<D>,
    dir: Dir,
    walk: &mut fs_fat::Entries,
    handed: &mut u64,
    cursor: u64,
) -> Result<Option<DirEntry>, Error> {
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
    volumes: &Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    handle: u32,
) -> Result<Stat, Error> {
    let which = volume_of(clients, badge, handle)?;
    let fs = volumes.get(which).ok_or(Error::NotFound)?;
    let (parent, name) = match handle {
        ROOT | BOOT => return Ok(a_directory()),
        other => match clients.get(badge, other).ok_or(Error::InvalidHandle)? {
            Opened::File { parent, name, .. } => (*parent, *name),
            Opened::Dir { .. } => return Ok(a_directory()),
        },
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

/// The attribute byte of a directory, as the protocol carries it.
#[expect(
    clippy::as_conversions,
    reason = "one byte of attributes widened into the word the protocol carries, in a const"
)]
const ATTR_DIRECTORY_WORD: u32 = ATTR_DIRECTORY as u32;

/// What a stat of a directory answers. A directory has no size of its own
/// and no entry this server reads a moment out of.
const fn a_directory() -> Stat {
    Stat {
        size: 0,
        attributes: ATTR_DIRECTORY_WORD,
        modified: 0,
    }
}

/// `remove`: take an entry off the volume.
fn remove<D: BlockDevice>(
    volumes: &mut Volumes<'_, D>,
    clients: &mut Clients,
    badge: u64,
    parent: u32,
    name: &ProtoName,
) -> Result<(), Error> {
    let (volume, dir) = directory(volumes, clients, badge, parent)?;
    let fs = volumes.get_mut(volume).ok_or(Error::AccessDenied)?;
    let wanted = name_of(name)?;
    if clients.has_open_file(volume, dir, wanted) {
        return Err(Error::Busy);
    }
    fs.remove(dir, &wanted).map_err(refusal)?;
    Ok(())
}

/// `close`: give a file handle back.
fn close(clients: &mut Clients, badge: u64, handle: u32) -> Result<(), Error> {
    if matches!(handle, ROOT | BOOT) {
        return if clients.clear_root(badge, usize::from(handle == BOOT)) {
            Ok(())
        } else {
            Err(Error::InvalidHandle)
        };
    }
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
