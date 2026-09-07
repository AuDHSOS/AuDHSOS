// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The volume: mounting one, making one, and everything a caller does
//! with the one it has.

use audhsos_time::UnixTime;

use crate::boot::{
    BACKUP_BOOT_SECTOR, FSINFO_SECTOR, FormatOptions, Geometry, boot_sector, geometry_for,
    info_sector, parse,
};
use crate::device::{BlockDevice, SECTOR, read_u32};
use crate::dir::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, DELETED, Dir, ENTRY_LEN, Entry, Location, Slot, decode, encode,
    patch,
};
use crate::error::Error;
use crate::name::Name;
use crate::table::{self, END_OF_CHAIN};

/// [`SECTOR`] as the number the offsets count in.
const SECTOR_U32: u32 = 512;

/// A mounted volume.
///
/// The free count and the hint are the volume's own bookkeeping. They are
/// counted at [`FileSystem::mount`] rather than believed from the
/// information sector, because that sector is a note the last writer left
/// and the table is the truth; [`FileSystem::flush`] writes the note back.
#[derive(Clone, Debug)]
pub struct FileSystem<D> {
    /// Where the sectors come from.
    device: D,
    /// The parameter block as numbers.
    geometry: Geometry,
    /// How many clusters are free.
    free: u32,
    /// Where the next search for a free cluster starts.
    next_free: u32,
}

/// A place in a directory, walked one entry at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entries {
    /// The cluster being read.
    cluster: u32,
    /// The slot within that cluster.
    slot: u32,
    /// Whether the walk has reached the end.
    done: bool,
}

/// An open file: where its bytes are, how many there are, which entry
/// describes it, and where the last read or write stopped.
///
/// The cursor is what keeps writing a large file linear. A chain has no
/// way back, so a write that found its place from the first cluster every
/// time would walk the whole file again for every cluster it added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct File {
    /// First cluster of the contents; zero while the file has none.
    first: u32,
    /// Size in bytes.
    size: u32,
    /// The directory slot that describes the file.
    location: Location,
    /// The cluster the cursor stands in.
    cursor: u32,
    /// Which cluster of the chain that is, counting from zero.
    cursor_index: u32,
}

impl File {
    /// The size in bytes.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// The first cluster of the contents, zero for a file with none.
    #[must_use]
    pub const fn first_cluster(&self) -> u32 {
        self.first
    }
}

impl<D: BlockDevice> FileSystem<D> {
    /// Mounts the volume on `device`.
    ///
    /// # Errors
    ///
    /// The errors of [`parse`], and the device's own.
    pub fn mount(device: D) -> Result<FileSystem<D>, Error> {
        let mut boot = [0u8; SECTOR];
        device.read(0, &mut boot)?;
        let geometry = parse(&boot)?;
        let (free, next_free) = table::count_free(&device, &geometry)?;
        Ok(FileSystem {
            device,
            geometry,
            free,
            next_free,
        })
    }

    /// Writes a fresh volume onto `device` and mounts it: the boot sector
    /// and its copy, the information sector, empty tables, and a root
    /// directory of one zeroed cluster.
    ///
    /// # Errors
    ///
    /// The errors of [`geometry_for`], and the device's own.
    pub fn format(mut device: D, options: &FormatOptions) -> Result<FileSystem<D>, Error> {
        let geometry = geometry_for(device.sectors(), options)?;
        let boot = boot_sector(&geometry, options);
        device.write(0, &boot)?;
        device.write(BACKUP_BOOT_SECTOR, &boot)?;
        table::write_empty(&mut device, &geometry, options.media)?;
        let mut volume = FileSystem {
            device,
            geometry,
            free: geometry.clusters,
            next_free: geometry.root_cluster,
        };
        let root = volume.geometry.root_cluster;
        volume.claim(root)?;
        volume.zero_cluster(root)?;
        volume.flush()?;
        Ok(volume)
    }

    /// The parameter block as numbers.
    pub const fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    /// How many clusters are free.
    pub const fn free_clusters(&self) -> u32 {
        self.free
    }

    /// Where the next search for a free cluster starts.
    pub const fn next_free(&self) -> u32 {
        self.next_free
    }

    /// The device, back again.
    pub fn into_device(self) -> D {
        self.device
    }

    /// The device under the volume, to read sectors this crate gives no
    /// name to.
    pub const fn device(&self) -> &D {
        &self.device
    }

    /// The device under the volume, to write sectors this crate gives no
    /// name to. What is written through here is not seen by the free
    /// count and the hint the volume carries, so a caller that changes
    /// the table this way mounts the volume again afterwards.
    pub const fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    /// The root directory.
    pub const fn root(&self) -> Dir {
        Dir::at(self.geometry.root_cluster)
    }

    /// Writes the free count and the hint into the information sector.
    ///
    /// # Errors
    ///
    /// The device's own error.
    pub fn flush(&mut self) -> Result<(), Error> {
        let info = info_sector(self.free, self.next_free);
        self.device.write(FSINFO_SECTOR, &info)
    }

    /// The cluster after `cluster`, or `None` where the chain ends.
    ///
    /// # Errors
    ///
    /// The errors of the table walk: a free cluster in a chain, the
    /// bad-cluster marker, and a number outside the data region.
    pub fn next_cluster(&self, cluster: u32) -> Result<Option<u32>, Error> {
        table::next(&self.device, &self.geometry, cluster)
    }

    /// How many clusters the chain at `first` has.
    ///
    /// # Errors
    ///
    /// [`Error::ChainLoop`] for a chain that points back into itself, and
    /// the errors of [`FileSystem::next_cluster`].
    pub fn chain_length(&self, first: u32) -> Result<u32, Error> {
        table::chain_length(&self.device, &self.geometry, first)
    }

    /// Takes one free cluster and marks it the end of a chain of its own.
    ///
    /// # Errors
    ///
    /// [`Error::Full`] where no cluster is free, and the device's own.
    pub fn allocate(&mut self) -> Result<u32, Error> {
        let cluster =
            table::find_free(&self.device, &self.geometry, self.next_free)?.ok_or(Error::Full)?;
        self.claim(cluster)?;
        Ok(cluster)
    }

    /// Takes one free cluster and hangs it after `last`, which has to be
    /// the end of a chain. Answers the cluster it took.
    ///
    /// # Errors
    ///
    /// [`Error::Full`] where no cluster is free, [`Error::Cluster`] where
    /// `last` is outside the data region, and the device's own.
    pub fn extend(&mut self, last: u32) -> Result<u32, Error> {
        let cluster = self.allocate()?;
        table::set_entry(&mut self.device, &self.geometry, last, cluster)?;
        Ok(cluster)
    }

    /// Gives every cluster of the chain at `first` back, and answers how
    /// many that was.
    ///
    /// # Errors
    ///
    /// The errors of the table walk.
    pub fn free_chain(&mut self, first: u32) -> Result<u32, Error> {
        let mut cluster = first;
        let mut freed = 0u32;
        loop {
            let following = table::next(&self.device, &self.geometry, cluster)?;
            table::set_entry(&mut self.device, &self.geometry, cluster, 0)?;
            self.free = self.free.saturating_add(1);
            self.next_free = self.next_free.min(cluster);
            freed = freed.saturating_add(1);
            match following {
                Some(next) => cluster = next,
                None => return Ok(freed),
            }
            if freed > self.geometry.clusters {
                return Err(Error::ChainLoop(first));
            }
        }
    }

    /// A walk over the entries of `dir`.
    pub const fn entries(&self, dir: Dir) -> Entries {
        Entries {
            cluster: dir.cluster,
            slot: 0,
            done: false,
        }
    }

    /// The next entry of a walk, or `None` where the directory ends.
    ///
    /// # Errors
    ///
    /// [`Error::EntryName`] for an entry whose eleven bytes are not a
    /// name, [`Error::Time`] for one whose date names no day, and the
    /// errors of the table walk.
    pub fn next_entry(&self, cursor: &mut Entries) -> Result<Option<Entry>, Error> {
        while !cursor.done {
            let location = Location {
                cluster: cursor.cluster,
                slot: cursor.slot,
            };
            let mut buffer = [0u8; SECTOR];
            let sector = self
                .geometry
                .cluster_sector(cursor.cluster)
                .saturating_add(cursor.slot.wrapping_div(SLOTS_PER_SECTOR));
            self.device.read(sector, &mut buffer)?;
            let offset = usize::try_from(cursor.slot.wrapping_rem(SLOTS_PER_SECTOR))
                .unwrap_or(0)
                .saturating_mul(ENTRY_LEN);
            let bytes = buffer
                .get(offset..offset.saturating_add(ENTRY_LEN))
                .ok_or(Error::EntryName)?;
            let slot = decode(bytes, location)?;
            self.step(cursor)?;
            match slot {
                Slot::End => {
                    cursor.done = true;
                    return Ok(None);
                }
                Slot::Used(entry) => return Ok(Some(entry)),
                Slot::Free | Slot::Skip => {}
            }
        }
        Ok(None)
    }

    /// The entry `name` names, or `None` where the directory has none.
    ///
    /// # Errors
    ///
    /// The errors of [`FileSystem::next_entry`].
    pub fn find(&self, dir: Dir, name: &Name) -> Result<Option<Entry>, Error> {
        let mut cursor = self.entries(dir);
        while let Some(entry) = self.next_entry(&mut cursor)? {
            if entry.name == *name {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    /// Opens the file `name` names.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] where there is no such name and
    /// [`Error::Kind`] where it is a directory.
    pub fn open(&self, dir: Dir, name: &Name) -> Result<File, Error> {
        let entry = self.find(dir, name)?.ok_or(Error::NotFound)?;
        if entry.is_directory() {
            return Err(Error::Kind);
        }
        Ok(File {
            first: entry.first_cluster,
            size: entry.size,
            location: entry.location,
            cursor: entry.first_cluster,
            cursor_index: 0,
        })
    }

    /// Opens the directory `name` names.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] where there is no such name and
    /// [`Error::Kind`] where it is a file.
    pub fn open_dir(&self, dir: Dir, name: &Name) -> Result<Dir, Error> {
        let entry = self.find(dir, name)?.ok_or(Error::NotFound)?;
        if entry.is_directory() {
            Ok(Dir::at(entry.first_cluster))
        } else {
            Err(Error::Kind)
        }
    }

    /// Makes an empty file in `dir`.
    ///
    /// # Errors
    ///
    /// [`Error::Exists`] where the name is taken, [`Error::Time`] for a
    /// moment an entry cannot carry, [`Error::Full`] where the directory
    /// needs a cluster and none is free.
    pub fn create(&mut self, dir: Dir, name: &Name, at: UnixTime) -> Result<File, Error> {
        if self.find(dir, name)?.is_some() {
            return Err(Error::Exists);
        }
        let bytes = encode(name, ATTR_ARCHIVE, 0, 0, at)?;
        let location = self.insert(dir, &bytes)?;
        Ok(File {
            first: 0,
            size: 0,
            location,
            cursor: 0,
            cursor_index: 0,
        })
    }

    /// Makes a directory in `dir`, with the two entries a directory keeps
    /// for itself.
    ///
    /// # Errors
    ///
    /// [`Error::Exists`] where the name is taken, [`Error::Full`] where no
    /// cluster is free, and [`Error::Time`] for a moment an entry cannot
    /// carry.
    pub fn create_dir(&mut self, dir: Dir, name: &Name, at: UnixTime) -> Result<Dir, Error> {
        if self.find(dir, name)?.is_some() {
            return Err(Error::Exists);
        }
        let cluster = self.allocate()?;
        self.zero_cluster(cluster)?;
        let parent = if dir.cluster == self.geometry.root_cluster {
            0
        } else {
            dir.cluster
        };
        let dot = encode(&Name::DOT, ATTR_DIRECTORY, cluster, 0, at)?;
        let dot_dot = encode(&Name::DOT_DOT, ATTR_DIRECTORY, parent, 0, at)?;
        let mut buffer = [0u8; SECTOR];
        self.device
            .read(self.geometry.cluster_sector(cluster), &mut buffer)?;
        if let Some(slot) = buffer.get_mut(..ENTRY_LEN) {
            slot.copy_from_slice(&dot);
        }
        if let Some(slot) = buffer.get_mut(ENTRY_LEN..ENTRY_LEN.saturating_mul(2)) {
            slot.copy_from_slice(&dot_dot);
        }
        self.device
            .write(self.geometry.cluster_sector(cluster), &buffer)?;
        let bytes = encode(name, ATTR_DIRECTORY, cluster, 0, at)?;
        self.insert(dir, &bytes)?;
        Ok(Dir::at(cluster))
    }

    /// The directory `name` names, made where it is not there yet.
    ///
    /// # Errors
    ///
    /// [`Error::Kind`] where the name is a file, and the errors of
    /// [`FileSystem::create_dir`].
    pub fn open_or_create_dir(
        &mut self,
        dir: Dir,
        name: &Name,
        at: UnixTime,
    ) -> Result<Dir, Error> {
        match self.find(dir, name)? {
            Some(entry) if entry.is_directory() => Ok(Dir::at(entry.first_cluster)),
            Some(_) => Err(Error::Kind),
            None => self.create_dir(dir, name, at),
        }
    }

    /// Removes the file `name` names and answers how many clusters that
    /// gave back. A directory is refused, because removing one without
    /// walking into it would leave its contents behind.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`], [`Error::Kind`] for a directory, and the
    /// errors of the table walk.
    pub fn remove(&mut self, dir: Dir, name: &Name) -> Result<u32, Error> {
        let entry = self.find(dir, name)?.ok_or(Error::NotFound)?;
        if entry.is_directory() {
            return Err(Error::Kind);
        }
        let freed = if entry.first_cluster == 0 {
            0
        } else {
            self.free_chain(entry.first_cluster)?
        };
        let (sector, offset) = self.slot_at(entry.location);
        let mut buffer = [0u8; SECTOR];
        self.device.read(sector, &mut buffer)?;
        if let Some(byte) = buffer.get_mut(offset) {
            *byte = DELETED;
        }
        self.device.write(sector, &buffer)?;
        Ok(freed)
    }

    /// Reads from `file` at `offset` into `into`, and answers how many
    /// bytes that was: what is left of the file, at most.
    ///
    /// # Errors
    ///
    /// [`Error::Offset`] for an offset past the end, and the errors of the
    /// table walk.
    pub fn read(&self, file: &mut File, offset: u32, into: &mut [u8]) -> Result<usize, Error> {
        if offset > file.size {
            return Err(Error::Offset(offset));
        }
        let left = usize::try_from(file.size.saturating_sub(offset)).unwrap_or(usize::MAX);
        let wanted = into.len().min(left);
        let mut done = 0usize;
        while done < wanted {
            let position = offset.saturating_add(u32::try_from(done).unwrap_or(0));
            let (sector, within) = self.place(file, position)?;
            let take = SECTOR
                .saturating_sub(within)
                .min(wanted.saturating_sub(done));
            let mut buffer = [0u8; SECTOR];
            self.device.read(sector, &mut buffer)?;
            let source = buffer
                .get(within..within.saturating_add(take))
                .ok_or(Error::Offset(position))?;
            let target = into
                .get_mut(done..done.saturating_add(take))
                .ok_or(Error::Offset(position))?;
            target.copy_from_slice(source);
            done = done.saturating_add(take);
        }
        Ok(done)
    }

    /// Writes `from` into `file` at `offset`, extending it where the
    /// write reaches past its end, and answers how many bytes that was.
    ///
    /// The moment the directory entry carries is the one the file was
    /// made with. A write does not touch it, because the one caller this
    /// crate has writes an image that has to come out the same bytes
    /// twice; a caller that wants a modification time writes the entry
    /// itself.
    ///
    /// # Errors
    ///
    /// [`Error::Offset`] for an offset past the end, which would leave
    /// bytes nobody wrote; [`Error::TooLarge`] for a file above four
    /// gigabytes; [`Error::Full`] where the volume has no room; and the
    /// errors of the table walk.
    pub fn write(&mut self, file: &mut File, offset: u32, from: &[u8]) -> Result<usize, Error> {
        if offset > file.size {
            return Err(Error::Offset(offset));
        }
        let length = u32::try_from(from.len()).map_err(|_| Error::TooLarge)?;
        let end = offset.checked_add(length).ok_or(Error::TooLarge)?;
        self.reserve(file, end)?;
        let mut done = 0usize;
        while done < from.len() {
            let position = offset.saturating_add(u32::try_from(done).unwrap_or(0));
            let (sector, within) = self.place(file, position)?;
            let take = SECTOR
                .saturating_sub(within)
                .min(from.len().saturating_sub(done));
            let source = from
                .get(done..done.saturating_add(take))
                .ok_or(Error::Offset(position))?;
            let mut buffer = [0u8; SECTOR];
            if take != SECTOR {
                self.device.read(sector, &mut buffer)?;
            }
            if let Some(target) = buffer.get_mut(within..within.saturating_add(take)) {
                target.copy_from_slice(source);
            }
            self.device.write(sector, &buffer)?;
            done = done.saturating_add(take);
        }
        if end > file.size {
            file.size = end;
        }
        self.write_back(file)?;
        Ok(done)
    }

    /// Writes the first cluster and the size of `file` into the entry
    /// that describes it.
    fn write_back(&mut self, file: &File) -> Result<(), Error> {
        let (sector, offset) = self.slot_at(file.location);
        let mut buffer = [0u8; SECTOR];
        self.device.read(sector, &mut buffer)?;
        if let Some(slot) = buffer.get_mut(offset..offset.saturating_add(ENTRY_LEN)) {
            patch(slot, file.first, file.size);
        }
        self.device.write(sector, &buffer)
    }

    /// Where the entry at `location` stands: its sector and its offset in
    /// it.
    fn slot_at(&self, location: Location) -> (u32, usize) {
        let sector = self
            .geometry
            .cluster_sector(location.cluster)
            .saturating_add(location.slot.wrapping_div(SLOTS_PER_SECTOR));
        let offset = usize::try_from(location.slot.wrapping_rem(SLOTS_PER_SECTOR))
            .unwrap_or(0)
            .saturating_mul(ENTRY_LEN);
        (sector, offset)
    }

    /// The sector holding byte `position` of `file`, and where in it that
    /// byte is. Moves the file's cursor there.
    fn place(&self, file: &mut File, position: u32) -> Result<(u32, usize), Error> {
        let index = position
            .checked_div(self.geometry.cluster_bytes())
            .unwrap_or(0);
        let cluster = self.seek(file, index)?.ok_or(Error::Offset(position))?;
        let within = position
            .checked_rem(self.geometry.cluster_bytes())
            .unwrap_or(0);
        let sector = self
            .geometry
            .cluster_sector(cluster)
            .saturating_add(within.wrapping_div(SECTOR_U32));
        let offset = usize::try_from(within.wrapping_rem(SECTOR_U32)).unwrap_or(0);
        Ok((sector, offset))
    }

    /// The `index`-th cluster of `file`, walking on from the cursor where
    /// it is not past the place wanted.
    fn seek(&self, file: &mut File, index: u32) -> Result<Option<u32>, Error> {
        if file.first == 0 {
            return Ok(None);
        }
        let (mut cluster, mut at) = if index >= file.cursor_index && file.cursor != 0 {
            (file.cursor, file.cursor_index)
        } else {
            (file.first, 0)
        };
        while at < index {
            match table::next(&self.device, &self.geometry, cluster)? {
                Some(following) => cluster = following,
                None => return Ok(None),
            }
            at = at.saturating_add(1);
        }
        file.cursor = cluster;
        file.cursor_index = at;
        Ok(Some(cluster))
    }

    /// Makes sure `file` has clusters enough for `bytes` bytes.
    fn reserve(&mut self, file: &mut File, bytes: u32) -> Result<(), Error> {
        if bytes == 0 {
            return Ok(());
        }
        let needed = bytes.div_ceil(self.geometry.cluster_bytes());
        if file.first == 0 {
            let cluster = self.allocate()?;
            file.first = cluster;
            file.cursor = cluster;
            file.cursor_index = 0;
            self.write_back(file)?;
        }
        // From the cursor and not from the first cluster: the tail of a
        // chain is found by walking it, and an append that walked from
        // the start every time would be quadratic in the size of the file
        // it appends to. The cursor stands on a cluster of the chain
        // whenever the file has one, which the block above has just made
        // true, so the length is the cursor's place plus what is behind
        // it.
        let (behind, mut tail) = self.tail(file.cursor)?;
        let mut have = file.cursor_index.saturating_add(behind);
        while have < needed {
            let cluster = self.allocate()?;
            table::set_entry(&mut self.device, &self.geometry, tail, cluster)?;
            tail = cluster;
            have = have.saturating_add(1);
        }
        Ok(())
    }

    /// How long the chain at `first` is and which cluster ends it.
    fn tail(&self, first: u32) -> Result<(u32, u32), Error> {
        let mut cluster = first;
        let mut length = 1u32;
        while let Some(following) = table::next(&self.device, &self.geometry, cluster)? {
            cluster = following;
            length = length.saturating_add(1);
            if length > self.geometry.clusters {
                return Err(Error::ChainLoop(first));
            }
        }
        Ok((length, cluster))
    }

    /// Puts `bytes` into the first slot of `dir` that is free, growing the
    /// directory by one cluster where none is.
    fn insert(&mut self, dir: Dir, bytes: &[u8; ENTRY_LEN]) -> Result<Location, Error> {
        let mut cluster = dir.cluster;
        let mut steps = 0u32;
        loop {
            for slot in 0..self.slots_per_cluster() {
                let location = Location { cluster, slot };
                let (sector, offset) = self.slot_at(location);
                let mut buffer = [0u8; SECTOR];
                self.device.read(sector, &mut buffer)?;
                let first = buffer.get(offset).copied().unwrap_or(0);
                if first == 0 || first == DELETED {
                    if let Some(target) = buffer.get_mut(offset..offset.saturating_add(ENTRY_LEN)) {
                        target.copy_from_slice(bytes);
                    }
                    self.device.write(sector, &buffer)?;
                    return Ok(location);
                }
            }
            steps = steps.saturating_add(1);
            if steps > self.geometry.clusters {
                return Err(Error::ChainLoop(dir.cluster));
            }
            cluster = if let Some(following) = table::next(&self.device, &self.geometry, cluster)? {
                following
            } else {
                let grown = self.allocate()?;
                self.zero_cluster(grown)?;
                table::set_entry(&mut self.device, &self.geometry, cluster, grown)?;
                grown
            };
        }
    }

    /// How many entries one cluster holds.
    fn slots_per_cluster(&self) -> u32 {
        self.geometry
            .cluster_bytes()
            .checked_div(u32::try_from(ENTRY_LEN).unwrap_or(32))
            .unwrap_or(0)
    }

    /// Moves a walk one slot on, following the chain at the end of a
    /// cluster.
    fn step(&self, cursor: &mut Entries) -> Result<(), Error> {
        cursor.slot = cursor.slot.saturating_add(1);
        if cursor.slot < self.slots_per_cluster() {
            return Ok(());
        }
        cursor.slot = 0;
        match table::next(&self.device, &self.geometry, cursor.cluster)? {
            Some(following) => cursor.cluster = following,
            None => cursor.done = true,
        }
        Ok(())
    }

    /// Marks `cluster` the end of a chain and takes it out of the free
    /// count.
    fn claim(&mut self, cluster: u32) -> Result<(), Error> {
        table::set_entry(&mut self.device, &self.geometry, cluster, END_OF_CHAIN)?;
        self.free = self.free.saturating_sub(1);
        self.next_free = cluster.saturating_add(1);
        Ok(())
    }

    /// Writes zeroes over every sector of `cluster`.
    fn zero_cluster(&mut self, cluster: u32) -> Result<(), Error> {
        let blank = [0u8; SECTOR];
        let start = self.geometry.cluster_sector(cluster);
        for sector in 0..self.geometry.sectors_per_cluster {
            self.device.write(start.saturating_add(sector), &blank)?;
        }
        Ok(())
    }
}

/// Directory entries in one sector.
const SLOTS_PER_SECTOR: u32 = 16;

/// The free count and the hint the information sector of `device`
/// records, where it carries the two signatures that say it is one.
///
/// This is what a volume left behind, not what its table says. Nothing in
/// this crate believes it; it is here so that a caller can see it.
///
/// # Errors
///
/// The device's own error.
pub fn info<D: BlockDevice>(device: &D) -> Result<Option<(u32, u32)>, Error> {
    let mut buffer = [0u8; SECTOR];
    device.read(FSINFO_SECTOR, &mut buffer)?;
    if read_u32(&buffer, 0) != 0x4161_5252 || read_u32(&buffer, 484) != 0x6141_7272 {
        return Ok(None);
    }
    Ok(Some((read_u32(&buffer, 488), read_u32(&buffer, 492))))
}
