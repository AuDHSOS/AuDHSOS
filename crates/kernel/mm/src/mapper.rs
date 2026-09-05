// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Walking and editing page tables through the HAL traits.
//!
//! Invariants: a call that changes a translation flushes exactly that page
//! once; a call that fails changes nothing and flushes nothing; every table
//! frame the mapper allocates is either installed or given back.

use core::fmt;

use kernel_hal_api::paging::{FrameAccess, FrameSource, TlbControl};
use kernel_types::{Page, PageRange, PhysFrame};

use crate::page_table::{CachePolicy, EntryError, EntryFormat, PageTable, Permissions};

/// Why a mapping operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapError {
    /// The page already has a translation.
    AlreadyMapped,
    /// The page has no translation.
    NotMapped,
    /// No frame was left for a new table.
    OutOfKernelMemory,
    /// A table frame is not reachable through the frame access.
    UnreachableFrame,
    /// A table entry could not be interpreted.
    Entry(EntryError),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::AlreadyMapped => f.write_str("the page is already mapped"),
            MapError::NotMapped => f.write_str("the page is not mapped"),
            MapError::OutOfKernelMemory => f.write_str("no frame is left for a page table"),
            MapError::UnreachableFrame => f.write_str("a table frame is not reachable"),
            MapError::Entry(error) => write!(f, "{error}"),
        }
    }
}

impl From<MapError> for audhsos_abi::Error {
    fn from(error: MapError) -> Self {
        match error {
            MapError::AlreadyMapped => audhsos_abi::Error::AlreadyMapped,
            MapError::NotMapped => audhsos_abi::Error::NotMapped,
            MapError::OutOfKernelMemory => audhsos_abi::Error::OutOfKernelMemory,
            MapError::UnreachableFrame | MapError::Entry(_) => audhsos_abi::Error::InvalidArgument,
        }
    }
}

/// How far a bounded range operation came.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Progress {
    /// The whole range was processed.
    Done,
    /// The budget ran out after this many pages.
    Partial(u64),
}

/// One table that was created during a walk, so that it can be undone.
#[derive(Clone, Copy, Debug)]
struct Created {
    parent: PhysFrame,
    index: usize,
    frame: PhysFrame,
}

/// Edits the page tables of one address space.
pub struct Mapper<'a, F, A, T, S>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    root: PhysFrame,
    access: &'a mut A,
    tlb: &'a mut T,
    frames: &'a mut S,
    _format: core::marker::PhantomData<fn() -> F>,
}

impl<'a, F, A, T, S> Mapper<'a, F, A, T, S>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    /// A mapper over the tables rooted in `root`.
    pub fn new(root: PhysFrame, access: &'a mut A, tlb: &'a mut T, frames: &'a mut S) -> Self {
        Mapper {
            root,
            access,
            tlb,
            frames,
            _format: core::marker::PhantomData,
        }
    }

    /// The frame the tables are rooted in.
    #[must_use]
    pub const fn root(&self) -> PhysFrame {
        self.root
    }

    /// The frame source the mapper builds tables from, for a caller that
    /// needs frames for the pages it is about to map.
    pub const fn frames_mut(&mut self) -> &mut S {
        self.frames
    }

    fn read(&self, frame: PhysFrame, index: usize) -> Result<F, MapError> {
        let table = self.access.table(frame).ok_or(MapError::UnreachableFrame)?;
        let entry = table.entry(index);
        entry.validate().map_err(MapError::Entry)?;
        Ok(entry)
    }

    fn write(&mut self, frame: PhysFrame, index: usize, entry: F) -> Result<(), MapError> {
        let table = self
            .access
            .table_mut(frame)
            .ok_or(MapError::UnreachableFrame)?;
        table.set_entry(index, entry);
        Ok(())
    }

    /// Undoes the tables created during a failed walk, newest first.
    fn rollback(&mut self, created: &[Option<Created>; 3], len: usize) {
        for entry in created.iter().take(len).rev().flatten() {
            let _ = self.write(entry.parent, entry.index, F::EMPTY);
            self.frames.release_frame(entry.frame);
        }
    }

    /// The frame of the table that holds the leaf for `page`, creating the
    /// intermediate tables when `create` is set.
    fn walk(&mut self, page: Page, create: bool) -> Result<PhysFrame, MapError> {
        let mut created: [Option<Created>; 3] = [None; 3];
        let mut created_len = 0usize;
        let mut table_frame = self.root;
        let mut level = F::LEVELS.saturating_sub(1);
        while level > 0 {
            let index = F::index(level, page);
            let entry = match self.read(table_frame, index) {
                Ok(entry) => entry,
                Err(error) => {
                    self.rollback(&created, created_len);
                    return Err(error);
                }
            };
            match entry.frame() {
                Some(next) => table_frame = next,
                None if !create => {
                    self.rollback(&created, created_len);
                    return Err(MapError::NotMapped);
                }
                None => {
                    let Some(fresh) = self.frames.allocate_frame() else {
                        self.rollback(&created, created_len);
                        return Err(MapError::OutOfKernelMemory);
                    };
                    if let Some(table) = self.access.table_mut(fresh) {
                        *table = PageTable::default();
                    } else {
                        self.frames.release_frame(fresh);
                        self.rollback(&created, created_len);
                        return Err(MapError::UnreachableFrame);
                    }
                    if let Err(error) = self.write(table_frame, index, F::table(fresh)) {
                        self.frames.release_frame(fresh);
                        self.rollback(&created, created_len);
                        return Err(error);
                    }
                    if let Some(slot) = created.get_mut(created_len) {
                        *slot = Some(Created {
                            parent: table_frame,
                            index,
                            frame: fresh,
                        });
                        created_len = created_len.saturating_add(1);
                    }
                    table_frame = fresh;
                }
            }
            level = level.saturating_sub(1);
        }
        Ok(table_frame)
    }

    /// Maps `page` to `frame`.
    ///
    /// # Errors
    ///
    /// [`MapError::AlreadyMapped`] if the page has a translation;
    /// [`MapError::OutOfKernelMemory`] if a table could not be created;
    /// [`MapError::UnreachableFrame`] or [`MapError::Entry`] if a table
    /// could not be read.
    pub fn map(
        &mut self,
        page: Page,
        frame: PhysFrame,
        perms: Permissions,
        cache: CachePolicy,
    ) -> Result<(), MapError> {
        let leaf_table = self.walk(page, true)?;
        let index = F::index(0, page);
        let entry = self.read(leaf_table, index)?;
        if entry.is_present() {
            return Err(MapError::AlreadyMapped);
        }
        let global = !page.is_user();
        self.write(leaf_table, index, F::leaf(frame, perms, cache, global))?;
        self.tlb.flush_page(page);
        Ok(())
    }

    /// Removes the translation of `page` and frees every table that became
    /// empty.
    ///
    /// # Errors
    ///
    /// [`MapError::NotMapped`] if the page has no translation.
    pub fn unmap(&mut self, page: Page) -> Result<PhysFrame, MapError> {
        let leaf_table = self.walk(page, false)?;
        let index = F::index(0, page);
        let entry = self.read(leaf_table, index)?;
        let frame = entry.frame().ok_or(MapError::NotMapped)?;
        self.write(leaf_table, index, F::EMPTY)?;
        self.tlb.flush_page(page);
        self.collect_empty_tables(page)?;
        Ok(frame)
    }

    /// Frees every table below the root that has no entry left, lowest
    /// level first.
    fn collect_empty_tables(&mut self, page: Page) -> Result<(), MapError> {
        let mut target = 0;
        while target < F::LEVELS.saturating_sub(1) {
            let Some((parent, table_frame)) = self.table_pair(page, target)? else {
                return Ok(());
            };
            let table = self
                .access
                .table(table_frame)
                .ok_or(MapError::UnreachableFrame)?;
            if !table.is_unused() {
                return Ok(());
            }
            self.write(parent, F::index(target.saturating_add(1), page), F::EMPTY)?;
            self.frames.release_frame(table_frame);
            target = target.saturating_add(1);
        }
        Ok(())
    }

    /// The frame of the table at `target` together with the frame of the
    /// table one level above it, or `None` if the walk ends earlier or
    /// `target` names the root.
    fn table_pair(
        &self,
        page: Page,
        target: usize,
    ) -> Result<Option<(PhysFrame, PhysFrame)>, MapError> {
        let mut current = F::LEVELS.saturating_sub(1);
        let mut frame = self.root;
        while current > target {
            let entry = self.read(frame, F::index(current, page))?;
            let Some(next) = entry.frame() else {
                return Ok(None);
            };
            if current == target.saturating_add(1) {
                return Ok(Some((frame, next)));
            }
            frame = next;
            current = current.saturating_sub(1);
        }
        Ok(None)
    }

    /// Changes the permissions of a mapped page.
    ///
    /// # Errors
    ///
    /// [`MapError::NotMapped`] if the page has no translation.
    pub fn protect(&mut self, page: Page, perms: Permissions) -> Result<(), MapError> {
        let leaf_table = self.walk(page, false)?;
        let index = F::index(0, page);
        let entry = self.read(leaf_table, index)?;
        if !entry.is_present() {
            return Err(MapError::NotMapped);
        }
        self.write(leaf_table, index, entry.with_permissions(perms))?;
        self.tlb.flush_page(page);
        Ok(())
    }

    /// The frame and the permissions of `page`, if it is mapped.
    #[must_use]
    pub fn translate(&self, page: Page) -> Option<(PhysFrame, Permissions)> {
        let mut table_frame = self.root;
        let mut level = F::LEVELS.saturating_sub(1);
        while level > 0 {
            let entry = self.read(table_frame, F::index(level, page)).ok()?;
            table_frame = entry.frame()?;
            level = level.saturating_sub(1);
        }
        let entry = self.read(table_frame, F::index(0, page)).ok()?;
        entry.frame().map(|frame| (frame, entry.permissions()))
    }

    /// Maps the pages of `pages` to consecutive frames starting at
    /// `first_frame`, at most `budget` pages per call.
    ///
    /// # Errors
    ///
    /// The errors of [`Mapper::map`]; the pages mapped before the failure
    /// stay mapped.
    pub fn map_range(
        &mut self,
        pages: PageRange,
        first_frame: PhysFrame,
        perms: Permissions,
        cache: CachePolicy,
        budget: u64,
    ) -> Result<Progress, MapError> {
        let mut done = 0u64;
        for page in pages {
            if done >= budget {
                return Ok(Progress::Partial(done));
            }
            let frame = first_frame
                .checked_add(done)
                .ok_or(MapError::UnreachableFrame)?;
            self.map(page, frame, perms, cache)?;
            done = done.saturating_add(1);
        }
        Ok(Progress::Done)
    }

    /// Removes the translations of `pages`, at most `budget` pages per call.
    ///
    /// # Errors
    ///
    /// The errors of [`Mapper::unmap`]; the pages unmapped before the
    /// failure stay unmapped.
    pub fn unmap_range(&mut self, pages: PageRange, budget: u64) -> Result<Progress, MapError> {
        let mut done = 0u64;
        for page in pages {
            if done >= budget {
                return Ok(Progress::Partial(done));
            }
            self.unmap(page)?;
            done = done.saturating_add(1);
        }
        Ok(Progress::Done)
    }
}
