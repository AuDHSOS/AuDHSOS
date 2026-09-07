// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The file allocation table: what follows a cluster, what is free, and
//! what a chain is.
//!
//! Every write goes into every copy of the table, so the copies cannot
//! drift apart. Every walk is bounded by the number of clusters the
//! volume has, so a chain that points back into itself is an error and
//! not a hang.

use crate::boot::{Geometry, ROOT_CLUSTER};
use crate::device::{BlockDevice, SECTOR, put, read_u32};
use crate::error::Error;

/// The value written into the last cluster of a chain.
pub const END_OF_CHAIN: u32 = 0x0FFF_FFFF;

/// At or above this an entry means the chain ends here.
pub const END_MARK: u32 = 0x0FFF_FFF8;

/// The entry of a cluster the volume has given up on.
pub const BAD_CLUSTER: u32 = 0x0FFF_FFF7;

/// A table entry is twenty-eight bits; the top four belong to whoever
/// wrote them and are carried through untouched.
pub const ENTRY_MASK: u32 = 0x0FFF_FFFF;

/// Entries in one sector of the table.
const ENTRIES_PER_SECTOR: u32 = 128;

/// Bytes of one entry.
const ENTRY_BYTES: u32 = 4;

/// The sector of the first table copy that holds the entry of `cluster`.
const fn entry_sector(geometry: &Geometry, cluster: u32) -> u32 {
    geometry
        .fat_start(0)
        .saturating_add(cluster.wrapping_div(ENTRIES_PER_SECTOR))
}

/// Where in that sector the entry stands.
fn entry_offset(cluster: u32) -> usize {
    usize::try_from(
        cluster
            .wrapping_rem(ENTRIES_PER_SECTOR)
            .saturating_mul(ENTRY_BYTES),
    )
    .unwrap_or(0)
}

/// The entry of `cluster`, with the four reserved bits taken off.
///
/// # Errors
///
/// [`Error::Cluster`] for a cluster outside the data region, and the
/// device's own error.
pub(crate) fn entry<D: BlockDevice>(
    device: &D,
    geometry: &Geometry,
    cluster: u32,
) -> Result<u32, Error> {
    if !geometry.holds(cluster) {
        return Err(Error::Cluster(cluster));
    }
    let mut buffer = [0u8; SECTOR];
    device.read(entry_sector(geometry, cluster), &mut buffer)?;
    Ok(read_u32(&buffer, entry_offset(cluster)) & ENTRY_MASK)
}

/// Writes the entry of `cluster` into every copy of the table, keeping
/// the four bits that are not the crate's.
///
/// # Errors
///
/// [`Error::Cluster`] for a cluster outside the data region, and the
/// device's own error.
pub(crate) fn set_entry<D: BlockDevice>(
    device: &mut D,
    geometry: &Geometry,
    cluster: u32,
    value: u32,
) -> Result<(), Error> {
    if !geometry.holds(cluster) {
        return Err(Error::Cluster(cluster));
    }
    let offset = entry_offset(cluster);
    let sector = entry_sector(geometry, cluster);
    let within = sector.saturating_sub(geometry.fat_start(0));
    let mut buffer = [0u8; SECTOR];
    device.read(sector, &mut buffer)?;
    let kept = read_u32(&buffer, offset) & !ENTRY_MASK;
    put(
        &mut buffer,
        offset,
        &(kept | (value & ENTRY_MASK)).to_le_bytes(),
    );
    for copy in 0..geometry.fat_count {
        device.write(geometry.fat_start(copy).saturating_add(within), &buffer)?;
    }
    Ok(())
}

/// The cluster after `cluster`, or `None` where the chain ends.
///
/// # Errors
///
/// [`Error::FreeInChain`] where the chain runs into a cluster the table
/// calls free, [`Error::BadCluster`] where it runs into one the volume
/// gave up on, and [`Error::Cluster`] where the entry names no cluster of
/// this volume.
pub(crate) fn next<D: BlockDevice>(
    device: &D,
    geometry: &Geometry,
    cluster: u32,
) -> Result<Option<u32>, Error> {
    let value = entry(device, geometry, cluster)?;
    if value >= END_MARK {
        return Ok(None);
    }
    if value == BAD_CLUSTER {
        return Err(Error::BadCluster(cluster));
    }
    if value == 0 {
        return Err(Error::FreeInChain(cluster));
    }
    if !geometry.holds(value) {
        return Err(Error::Cluster(value));
    }
    Ok(Some(value))
}

/// How many clusters the chain at `first` has.
///
/// # Errors
///
/// [`Error::ChainLoop`] where the chain has more steps than the volume
/// has clusters, and the errors of [`next`].
pub(crate) fn chain_length<D: BlockDevice>(
    device: &D,
    geometry: &Geometry,
    first: u32,
) -> Result<u32, Error> {
    let mut cluster = first;
    let mut length = 0u32;
    loop {
        length = length.saturating_add(1);
        if length > geometry.clusters {
            return Err(Error::ChainLoop(first));
        }
        match next(device, geometry, cluster)? {
            Some(following) => cluster = following,
            None => return Ok(length),
        }
    }
}

/// The first free cluster at or after `hint`, wrapping once round the
/// table, or `None` where the volume is full.
///
/// # Errors
///
/// The device's own error.
pub(crate) fn find_free<D: BlockDevice>(
    device: &D,
    geometry: &Geometry,
    hint: u32,
) -> Result<Option<u32>, Error> {
    let last_cluster = geometry.last_cluster();
    let mut cluster = if geometry.holds(hint) {
        hint
    } else {
        ROOT_CLUSTER
    };
    let mut buffer = [0u8; SECTOR];
    let mut loaded = u32::MAX;
    for _ in 0..geometry.clusters {
        let sector = entry_sector(geometry, cluster);
        if sector != loaded {
            device.read(sector, &mut buffer)?;
            loaded = sector;
        }
        if read_u32(&buffer, entry_offset(cluster)) & ENTRY_MASK == 0 {
            return Ok(Some(cluster));
        }
        cluster = if cluster >= last_cluster {
            ROOT_CLUSTER
        } else {
            cluster.saturating_add(1)
        };
    }
    Ok(None)
}

/// How many clusters are free, and which is the first of them. The first
/// is [`Geometry::last_cluster`] plus one where none is free, which is
/// what a hint that finds nothing means.
///
/// # Errors
///
/// The device's own error.
pub(crate) fn count_free<D: BlockDevice>(
    device: &D,
    geometry: &Geometry,
) -> Result<(u32, u32), Error> {
    let mut buffer = [0u8; SECTOR];
    let mut loaded = u32::MAX;
    let mut free = 0u32;
    let mut first = geometry.last_cluster().saturating_add(1);
    for cluster in ROOT_CLUSTER..=geometry.last_cluster() {
        let sector = entry_sector(geometry, cluster);
        if sector != loaded {
            device.read(sector, &mut buffer)?;
            loaded = sector;
        }
        if read_u32(&buffer, entry_offset(cluster)) & ENTRY_MASK == 0 {
            free = free.saturating_add(1);
            first = first.min(cluster);
        }
    }
    Ok((free, first))
}

/// Writes an empty table: the two reserved entries and nothing else.
///
/// # Errors
///
/// The device's own error.
pub(crate) fn write_empty<D: BlockDevice>(
    device: &mut D,
    geometry: &Geometry,
    media: u8,
) -> Result<(), Error> {
    let blank = [0u8; SECTOR];
    for copy in 0..geometry.fat_count {
        let start = geometry.fat_start(copy);
        for sector in 0..geometry.fat_sectors {
            device.write(start.saturating_add(sector), &blank)?;
        }
    }
    let mut first = [0u8; SECTOR];
    put(
        &mut first,
        0,
        &(0x0FFF_FF00 | u32::from(media)).to_le_bytes(),
    );
    put(&mut first, 4, &END_OF_CHAIN.to_le_bytes());
    for copy in 0..geometry.fat_count {
        device.write(geometry.fat_start(copy), &first)?;
    }
    Ok(())
}
