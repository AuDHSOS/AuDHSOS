// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pages of a database being written, and the trees on them.
//!
//! `src/btree.c` is the document. A row goes on the leaf its key belongs
//! to, which is the descent a reader makes; a payload longer than the
//! page holds runs onto overflow pages, which section 1.6 chains by
//! their first four bytes.
//!
//! A page that will not hold a cell is balanced: `balance_deeper` where
//! the page is the root of its tree, `balance_quick` where the cell goes
//! at the end of the right-most leaf, and `balance_nonroot` otherwise,
//! which writes the page and its siblings again so that every cell fits.
//! A balance that leaves the parent a divider the parent will not hold
//! balances the parent next, up to the root.

use alloc::vec::Vec;

use crate::bytes::size;
use crate::error::Error;
use crate::header::Header;
use crate::image::MAX_DEPTH;
use crate::page::{Cell, Kind, Page, Payload, Source, Writer, build, local_len, write_cell};
use crate::value::Value;

/// Writes `value` where `at` names a slot of `list`, and writes nothing
/// where the list is shorter, which no caller of this is.
fn one<T: Clone>(list: &mut [T], at: usize, value: &T) {
    for slot in list.iter_mut().skip(at).take(1) {
        slot.clone_from(value);
    }
}

/// The pages of a database being written, page one first.
#[derive(Clone)]
pub struct Pages {
    /// Bytes per page.
    page_size: usize,
    /// Bytes of every page the b-tree may use.
    usable: usize,
    /// The pages themselves.
    held: Vec<Vec<u8>>,
    /// The first trunk page of the free list, or nought, which the
    /// header holds at offset 32.
    freelist: u32,
    /// What a page held when the transaction began, kept for the pages
    /// the transaction has opened to write.
    before: Vec<Option<Vec<u8>>>,
    /// What a page held when the statement began, kept for the pages the
    /// statement has opened to write, which is the statement journal
    /// `sqlite3VdbeOpenStatement` opens. A statement outside a
    /// transaction keeps none, because the transaction of that
    /// statement is the unit that is put back.
    started: Vec<Option<Start>>,
    /// How many pages the database held when the statement began, the
    /// trunk its free list begins at and how many pages lie on it,
    /// which undoing the statement puts back.
    statement: (u32, u32, u32),
    /// What the file holds for a page a commit left out, which is a page
    /// freed onto a trunk: `sqlite3PagerDontWrite` keeps it out of every
    /// commit until the page is written again, so the file and the cache
    /// part ways until then.
    skipped: Vec<Option<Vec<u8>>>,
    /// Which pages this transaction freed, which is `pHasContent`: a
    /// page the free list gives back comes as noughts unless this
    /// transaction is the one that put it there.
    freed: Vec<bool>,
    /// How many pages the database held when the transaction began,
    /// which is `dbOrigSize`.
    origin: u32,
    /// The pages the transaction has opened to write that the database
    /// already held, in the order it opened them, which is the order the
    /// rollback journal holds their records in.
    journalled: Vec<u32>,
    /// The first trunk page of the free list and how many pages lay on
    /// it when the transaction began, which a rollback puts back.
    list: (u32, u32),
    /// How many pages the free list holds, which the header holds at
    /// offset 36.
    freelist_count: u32,
    /// Whether the file keeps pointer maps, which is what a file that
    /// vacuums itself needs to say which page names each other page.
    vacuum: bool,
    /// The most pages the file may hold, which `PRAGMA max_page_count`
    /// sets and which a connection told nothing leaves at the most a
    /// page number counts to.
    most: u32,
    /// How many pages the cache holds before it writes one out, which
    /// `sqlite3PcacheSetCachesize` holds the cache to. No count where the
    /// caller named none, and the transaction then writes every page at
    /// its commit.
    caching: Option<usize>,
    /// How many pages of the journal each spill of this transaction had
    /// written when it ran, in the order the spills ran, which says which
    /// records one spill wrote.
    spills: Vec<usize>,
}

/// One page as the statement found it: what it held, what the commit
/// was to write for it, and whether this transaction had freed it.
#[derive(Clone)]
struct Start {
    /// The bytes of the page.
    held: Vec<u8>,
    /// What a commit was to write for it, where the commit was to leave
    /// the page it holds out.
    skipped: Option<Vec<u8>>,
    /// Whether this transaction had freed it.
    freed: bool,
}

/// What a pointer map says one page is, which is `PTRMAP_ROOTPAGE` and
/// the four after it in `src/btree.c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// The root of a tree, which no page names.
    Root,
    /// A page on the free list, which no page names.
    Free,
    /// The first page of an overflow chain, named by the page holding
    /// the cell the chain belongs to.
    Head,
    /// A later page of an overflow chain, named by the page before it.
    Tail,
    /// A page of a tree that is not its root, named by its parent.
    Branch,
}

impl Point {
    /// What a pointer-map byte says, or nothing where it says nothing a
    /// page can be.
    const fn of(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Point::Root),
            2 => Some(Point::Free),
            3 => Some(Point::Head),
            4 => Some(Point::Tail),
            5 => Some(Point::Branch),
            _ => None,
        }
    }

    /// The byte a pointer map holds for it.
    const fn byte(self) -> u8 {
        match self {
            Point::Root => 1,
            Point::Free => 2,
            Point::Head => 3,
            Point::Tail => 4,
            Point::Branch => 5,
        }
    }
}

/// How many bytes one pointer-map entry takes.
const ENTRY: usize = 5;

impl Pages {
    /// A database of one page: page one, which is the leaf the schema
    /// table begins as.
    ///
    /// # Errors
    ///
    /// [`Error::PageSize`] for a page size the format does not allow and
    /// [`Error::Reserved`] for a tail that leaves too little of it.
    pub fn new(page_size: u32, reserved: u8) -> Result<Self, Error> {
        if !(512..=crate::header::MAX_PAGE_SIZE).contains(&page_size)
            || !page_size.is_power_of_two()
        {
            return Err(Error::PageSize(page_size));
        }
        let usable = page_size.saturating_sub(u32::from(reserved));
        if usable < crate::header::MIN_USABLE {
            return Err(Error::Reserved(reserved));
        }
        let page_size = size(u64::from(page_size));
        let usable = size(u64::from(usable));
        let first =
            build(Kind::LeafTable, 1, page_size, usable, &[], None).ok_or(Error::Overrun)?;
        Ok(Pages {
            page_size,
            usable,
            held: alloc::vec![first],
            freelist: 0,
            before: alloc::vec![None],
            started: alloc::vec![None],
            statement: (1, 0, 0),
            skipped: alloc::vec![None],
            freed: alloc::vec![false],
            origin: 1,
            journalled: Vec::new(),
            caching: None,
            spills: Vec::new(),
            list: (0, 0),
            freelist_count: 0,
            vacuum: false,
            most: u32::MAX,
        })
    }

    /// The pages a file already written holds, page one first, which
    /// is what a connection that opened such a file reads.
    ///
    /// The header claims how many pages the file holds; a file whose
    /// claim is nought, which is what a library older than 3.7.0 wrote,
    /// is read as the pages the image itself carries. Splitting the
    /// image costs O(n) in its pages.
    ///
    /// # Errors
    ///
    /// [`Error::PageSize`] for a page size the format does not allow,
    /// [`Error::Reserved`] for a tail that leaves too little of the
    /// page, and [`Error::Overrun`] where the image carries fewer bytes
    /// than the header claims pages.
    pub fn opened(image: &[u8], header: &Header) -> Result<Self, Error> {
        if !(512..=crate::header::MAX_PAGE_SIZE).contains(&header.page_size)
            || !header.page_size.is_power_of_two()
        {
            return Err(Error::PageSize(header.page_size));
        }
        let usable = header.usable();
        if usable < crate::header::MIN_USABLE {
            return Err(Error::Reserved(header.reserved));
        }
        let page_size = size(u64::from(header.page_size));
        let usable = size(u64::from(usable));
        // The page size is 512 at least, so the division answers.
        let whole = image.len().checked_div(page_size).unwrap_or(0);
        let claimed = usize::try_from(header.pages).unwrap_or(0);
        let count = if header.pages == 0 { whole } else { claimed };
        if count > whole || count == 0 {
            return Err(Error::Overrun);
        }
        let held: Vec<Vec<u8>> = image
            .chunks_exact(page_size)
            .take(count)
            .map(<[u8]>::to_vec)
            .collect();
        let origin = u32::try_from(count).unwrap_or(u32::MAX);
        Ok(Pages {
            page_size,
            usable,
            held,
            freelist: header.freelist,
            before: alloc::vec![None; count],
            started: alloc::vec![None; count],
            statement: (origin, header.freelist, header.freelist_pages),
            skipped: alloc::vec![None; count],
            freed: alloc::vec![false; count],
            origin,
            journalled: Vec::new(),
            caching: None,
            spills: Vec::new(),
            list: (header.freelist, header.freelist_pages),
            freelist_count: header.freelist_pages,
            // Section 1.6: a file keeps pointer maps where the header
            // names a largest root page.
            vacuum: header.largest_root != 0,
            most: u32::MAX,
        })
    }

    /// Keeps pointer maps from here on, which is what `PRAGMA
    /// auto_vacuum` turns on before the first table is written.
    pub const fn vacuums(&mut self) {
        self.vacuum = true;
    }

    /// The pointer-map page that carries the entry for `number`, which
    /// is `ptrmapPageno`, and nought where the file keeps no maps or
    /// where `number` is page one.
    #[must_use]
    pub fn map_of(&self, number: u32) -> u32 {
        if !self.vacuum || number < 2 {
            return 0;
        }
        map_page(self.usable, number)
    }

    /// Whether `number` is a pointer-map page rather than a page a tree
    /// or a chain may take.
    #[must_use]
    pub fn is_map(&self, number: u32) -> bool {
        self.map_of(number) == number
    }

    /// What the pointer map says about `number`, which is `ptrmapGet`.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the map page lies outside the file, and
    /// [`Error::PageKind`] for an entry no kind of page answers.
    pub fn point_of(&self, number: u32) -> Result<(Point, u32), Error> {
        let map = self.map_of(number);
        if map == 0 {
            return Err(Error::Page(number));
        }
        let at =
            size(u64::from(number.saturating_sub(map).saturating_sub(1))).saturating_mul(ENTRY);
        let entry = self
            .bytes(map)?
            .get(at..at.saturating_add(ENTRY))
            .ok_or(Error::Page(map))?;
        let byte = entry.first().copied().unwrap_or(0);
        let kind = Point::of(byte).ok_or(Error::PageKind(byte))?;
        Ok((kind, crate::bytes::u32_at(entry, 1).ok_or(Error::Overrun)?))
    }

    /// Writes what the pointer map says about every page the cells of
    /// `number` name: the child of each cell of a page that has
    /// children, the page to its right, and the first page of the chain
    /// each row that runs onto one begins.
    ///
    /// This is `ptrmapPut` of `insertCell` and of `balance_nonroot`
    /// together. The C library writes an entry only where it cannot
    /// tell the entry already stands; writing every one of them leaves
    /// the same file, because a map page is opened only where the entry
    /// it holds changes.
    ///
    /// # Errors
    ///
    /// [`Error`] names what reading the page or its cells refused.
    pub fn point_cells(&mut self, number: u32) -> Result<(), Error> {
        if !self.vacuum {
            return Ok(());
        }
        let mut named: Vec<(u32, Point)> = Vec::new();
        {
            let page = self.page(number)?;
            let interior = page.kind().is_interior();
            if interior {
                for index in 0..page.cells() {
                    named.push((page.child(index)?, Point::Branch));
                }
                named.extend(page.right_most().map(|child| (child, Point::Branch)));
            }
            // Every cell that carries a payload names the page its
            // chain begins on, which `ptrmapPutOvflPtr` records: an
            // index page carries one on every cell it has and a table
            // page only on its leaves.
            for index in 0..page.cells() {
                let head = page.overflow(index)?.map(|(head, _)| head);
                named.extend(head.map(|head| (head, Point::Head)));
            }
        }
        for (child, kind) in named {
            self.point(child, kind, number)?;
        }
        Ok(())
    }

    /// The first page from `number` on that no pointer map lies at,
    /// which is the page `fillInCell` asks for when it takes the next
    /// page of a chain.
    #[must_use]
    pub fn after_maps(&self, number: u32) -> u32 {
        let mut number = number.saturating_add(1);
        while self.is_map(number) {
            number = number.saturating_add(1);
        }
        number
    }

    /// Writes what the pointer map says about `number`, which is
    /// `ptrmapPut`: the page is left alone where it already says this,
    /// so a transaction that changes nothing opens no map page.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the map page lies outside the file.
    pub fn point(&mut self, number: u32, kind: Point, parent: u32) -> Result<(), Error> {
        let map = self.map_of(number);
        if map == 0 {
            return Ok(());
        }
        let at =
            size(u64::from(number.saturating_sub(map).saturating_sub(1))).saturating_mul(ENTRY);
        let [one, two, three, four] = parent.to_be_bytes();
        let entry = [kind.byte(), one, two, three, four];
        let held = self
            .bytes(map)?
            .get(at..at.saturating_add(ENTRY))
            .ok_or(Error::Page(map))?;
        if held == entry {
            return Ok(());
        }
        self.keep(map);
        self.put(map, at, &entry);
        Ok(())
    }

    /// How many pages the database holds.
    #[must_use]
    pub fn count(&self) -> u32 {
        u32::try_from(self.held.len()).unwrap_or(0)
    }

    /// The most pages the file may hold from here on, which is what
    /// `PRAGMA max_page_count` was set to.
    pub const fn capped(&mut self, most: u32) {
        self.most = most;
    }

    /// The first page of the free list and how many pages lie on it,
    /// which is what the header of the file says.
    #[must_use]
    pub const fn freelist(&self) -> (u32, u32) {
        (self.freelist, self.freelist_count)
    }

    /// Bytes of every page the b-tree may use.
    #[must_use]
    pub const fn usable(&self) -> usize {
        self.usable
    }

    /// A page of `kind`, taken off the free list where the list holds
    /// one and added at the end of the file where it does not, and the
    /// number the page has.
    ///
    /// The body of a page off the free list keeps the bytes it held,
    /// because `zeroPage` writes the header and nothing else.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the free list names a page the database
    /// does not hold, and [`Error::Overrun`] where the page is too small
    /// for the kind's own header.
    pub fn add(&mut self, kind: Kind, nearby: u32) -> Result<u32, Error> {
        let number = self.plain(nearby)?;
        self.keep(number);
        let usable = u32::try_from(self.usable).unwrap_or(0);
        let at = size(u64::from(number)).saturating_sub(1);
        let bytes = self.held.get_mut(at).ok_or(Error::Page(number))?;
        Writer::fresh(bytes, number, usable, kind).zero(kind);
        Ok(number)
    }

    /// The page a new root takes over a file that keeps pointer maps,
    /// left as an empty page of `kind`.
    ///
    /// This is `sqlite3BtreeCreateTable` of
    /// `research/sqlite/src/btree.c`: a root takes the page after the
    /// largest root the file holds, stepping over a pointer-map page, and
    /// the page that lies there is moved to the page the free list gave
    /// back or to the end of the file. The roots of a file that vacuums
    /// itself therefore run from page three up with no gap, which is what
    /// lets the vacuum of a commit move every page above them.
    ///
    /// Taking the page costs O(n) in the pages of the free list and O(m)
    /// in the cells of the page it moves.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the page the root is to take holds a root
    /// already or lies on the free list, and whatever reading or writing
    /// a page refuses.
    pub fn root_after(&mut self, largest: u32, kind: Kind) -> Result<u32, Error> {
        let mut wanted = largest.saturating_add(1);
        while self.is_map(wanted) {
            wanted = wanted.saturating_add(1);
        }
        // `allocateBtreePage` under `BTALLOC_EXACT` takes the page the
        // root is to lie on where the free list holds it, and the page
        // nearest it where it does not.
        let taken = self.plain(wanted)?;
        if taken != wanted {
            let (point, parent) = self.point_of(wanted)?;
            // `sqlite3BtreeCreateTable` refuses a page the map names a
            // root or a free page, because no tree holds the root it is
            // to take yet and the free list gave that page back already.
            let moves = !matches!(point, Point::Root | Point::Free);
            moves.then_some(()).ok_or(Error::Page(wanted))?;
            self.relocate(wanted, point, parent, taken)?;
        }
        self.blank(wanted, kind)?;
        self.point(wanted, Point::Root, 0)?;
        Ok(wanted)
    }

    /// The page `number` written over as an empty page of `kind`, which
    /// is what the root of a cleared tree is left as.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the file does not hold the page.
    pub fn blank(&mut self, number: u32, kind: Kind) -> Result<(), Error> {
        self.keep(number);
        let usable = u32::try_from(self.usable).unwrap_or(0);
        let at = size(u64::from(number)).saturating_sub(1);
        let bytes = self.held.get_mut(at).ok_or(Error::Page(number))?;
        Writer::fresh(bytes, number, usable, kind).zero(kind);
        Ok(())
    }

    /// A page of no kind at all, which is what an overflow page is, and
    /// the number it has.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the free list names a page the database
    /// does not hold.
    pub fn add_plain(&mut self, nearby: u32) -> Result<u32, Error> {
        self.plain(nearby)
    }

    /// One page with nothing written on it, which is `allocateBtreePage`
    /// for a caller that asks for no page in particular: the first leaf
    /// of the first trunk of the free list, the trunk itself where the
    /// trunk holds no leaf, and a page at the end of the file where the
    /// list is empty.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the free list names a page the database
    /// does not hold.
    fn plain(&mut self, nearby: u32) -> Result<u32, Error> {
        let Some(number) = self.take(nearby)? else {
            // `allocateBtreePage` refuses a file that would grow past
            // the count the connection set.
            if self.count() >= self.most {
                return Err(Error::Full);
            }
            // `allocateBtreePage`: page one holds how many pages the
            // database has, so growing the file writes page one, which
            // is where the journal takes it.
            self.keep(1);
            // A page at the end of the file that falls where a pointer
            // map lies makes that map page and the caller takes the
            // page after it.
            let mut number = self.grow();
            if self.is_map(number) {
                self.keep(number);
                number = self.grow();
            }
            return Ok(number);
        };
        self.keep(number);
        // `PAGER_GET_NOCONTENT`: a page the free list gives back is
        // noughts, because nothing of what it held is needed, unless
        // this transaction is the one that freed it, which the pager
        // has the bytes of in hand.
        let at = size(u64::from(number)).saturating_sub(1);
        if !self.freed.get(at).copied().unwrap_or(false) {
            for page in self.held.iter_mut().skip(at).take(1) {
                page.fill(0);
            }
        }
        Ok(number)
    }

    /// Moves the pages at the end of the file into the free pages below
    /// them and shortens the file, which is `autoVacuumCommit` for a
    /// file that does not vacuum a step at a time.
    ///
    /// The free list is dropped whole, because every page on it either
    /// took a page from the end or lies past the end the file is cut
    /// back to.
    ///
    /// Each page is moved once and its parent written once, so the whole
    /// of it is O(n) in the pages the file gives up.
    ///
    /// # Errors
    ///
    /// [`Error::Balance`] where the free list runs out before the pages
    /// past the end do, and whatever reading or writing a page refuses.
    pub fn vacuum_commit(&mut self) -> Result<(), Error> {
        let origin = self.count();
        let free = self.freelist_count;
        if !self.vacuum || free == 0 {
            return Ok(());
        }
        let last = final_size(self.usable, origin, free);
        for number in (last.saturating_add(1)..=origin).rev() {
            self.vacuum_step(last, number)?;
        }
        self.keep(1);
        self.freelist = 0;
        self.freelist_count = 0;
        self.shorten(last);
        Ok(())
    }

    /// One page at the end of the file moved into a free page below it,
    /// with the file shortened by that page, and whether the file gave
    /// one up at all, which is `incrVacuumStep` of
    /// `research/sqlite/src/btree.c:4038` with no commit under way.
    ///
    /// The free list keeps the pages it holds, so one call gives up one
    /// page. `sqlite3BtreeIncrVacuum` of
    /// `research/sqlite/src/btree.c:4165` answers `SQLITE_DONE` for a
    /// file that vacuums itself whole or holds no free page, which is
    /// `false` here. One call costs O(n) in the pages of the free list,
    /// which is walked trunk by trunk for the page to move into.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] where the map names the last page a root page or
    /// the free list holds more pages than the file, [`Error::Balance`]
    /// where the list holds no page at or below the end, and whatever
    /// reading or writing a page refuses.
    pub fn vacuum_incremental(&mut self) -> Result<bool, Error> {
        let origin = self.count();
        let free = self.freelist_count;
        if !self.vacuum || free == 0 {
            return Ok(false);
        }
        // `sqlite3BtreeIncrVacuum` refuses a file whose free list holds
        // more pages than the file has. Its other test, for an end past
        // the pages the file holds, is what `final_size` cannot answer
        // here, because the count it takes away saturates at nought.
        if free >= origin {
            return Err(Error::Page(origin));
        }
        let last = final_size(self.usable, origin, free);
        if !self.is_map(origin) {
            let (kind, parent) = self.point_of(origin)?;
            match kind {
                Point::Root => return Err(Error::Page(origin)),
                // The last page is on the free list already, so it comes
                // off the list and is given up with the end of the file.
                Point::Free => {
                    self.searched(origin, true)?.ok_or(Error::Balance)?;
                }
                Point::Head | Point::Tail | Point::Branch => {
                    let into = self.searched(last, false)?.ok_or(Error::Balance)?;
                    self.relocate(origin, kind, parent, into)?;
                }
            }
        }
        // A pointer-map page at the end is given up with the pages it
        // carried the entries of, which `incrVacuumStep` steps past.
        let mut end = origin.saturating_sub(1);
        while self.is_map(end) {
            end = end.saturating_sub(1);
        }
        self.keep(1);
        self.shorten(end);
        Ok(true)
    }

    /// The free page `wanted` names where the list holds it, or the
    /// first page the list holds at or below it, taken off the list.
    ///
    /// This is `allocateBtreePage` of `research/sqlite/src/btree.c:6514`
    /// under `BTALLOC_EXACT` for `exact` and under `BTALLOC_LE` for the
    /// other, each of which walks every trunk of the list rather than
    /// the first alone, so one call costs O(n) in the pages of the list.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where a trunk names a word outside the page,
    /// and [`Error::Page`] where it names a page the file does not hold.
    fn searched(&mut self, wanted: u32, exact: bool) -> Result<Option<u32>, Error> {
        let mut before = 0;
        let mut trunk = self.freelist;
        for _ in 0..self.freelist_count {
            if trunk == 0 {
                break;
            }
            let next = self.word(trunk, 0)?;
            let leaves = self.word(trunk, 4)?;
            if fits(trunk, wanted, exact) {
                self.took_trunk(before, trunk, (next, leaves))?;
                return Ok(Some(trunk));
            }
            if let Some(leaf) = self.took_leaf(trunk, leaves, (wanted, exact))? {
                return Ok(Some(leaf));
            }
            before = trunk;
            trunk = next;
        }
        Ok(None)
    }

    /// The trunk page itself taken off the free list, with `before` for
    /// the trunk that names it and nought where page one names it.
    ///
    /// The first leaf of the trunk becomes the trunk in its place and
    /// carries the leaves after it, which `allocateBtreePage` writes for
    /// a trunk the caller asked for by number.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where the trunk names a word outside the page.
    fn took_trunk(&mut self, before: u32, trunk: u32, list: (u32, u32)) -> Result<(), Error> {
        let (next, leaves) = list;
        self.keep(1);
        self.freelist_count = self.freelist_count.saturating_sub(1);
        let into = if leaves == 0 {
            next
        } else {
            let fresh = self.word(trunk, slot(0))?;
            let mut rest = Vec::new();
            for index in 1..leaves {
                rest.extend(self.word(trunk, slot(index))?.to_be_bytes());
            }
            self.put(fresh, 0, &next.to_be_bytes());
            self.put(fresh, 4, &leaves.saturating_sub(1).to_be_bytes());
            self.put(fresh, slot(0), &rest);
            fresh
        };
        if before == 0 {
            self.freelist = into;
        } else {
            self.put(before, 0, &into.to_be_bytes());
        }
        Ok(())
    }

    /// The first leaf of `trunk` that `wanted` names taken off the free
    /// list, and nothing where the trunk holds no such leaf.
    ///
    /// The last leaf takes the place of the one that was taken, which
    /// `allocateBtreePage` does to leave the array whole.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where the trunk names a word outside the page.
    fn took_leaf(
        &mut self,
        trunk: u32,
        leaves: u32,
        asked: (u32, bool),
    ) -> Result<Option<u32>, Error> {
        let (wanted, exact) = asked;
        for index in 0..leaves {
            let leaf = self.word(trunk, slot(index))?;
            if !fits(leaf, wanted, exact) {
                continue;
            }
            let last = self.word(trunk, slot(leaves.saturating_sub(1)))?;
            self.put(trunk, slot(index), &last.to_be_bytes());
            self.put(trunk, 4, &leaves.saturating_sub(1).to_be_bytes());
            self.keep(1);
            self.freelist_count = self.freelist_count.saturating_sub(1);
            return Ok(Some(leaf));
        }
        Ok(None)
    }

    /// The file cut back to `last` pages.
    fn shorten(&mut self, last: u32) {
        // What a page held when the transaction began is kept for every
        // page the file held then, because a rollback writes the file
        // back to that length and every one of those pages with it:
        // `sqlite3PagerRollback` plays the journal back over a file
        // `bDoTruncate` has not cut yet.
        for number in last.saturating_add(1)..=self.count() {
            self.keep(number);
        }
        self.held.truncate(size(u64::from(last)));
        let held = size(u64::from(last.max(self.origin)));
        self.before.truncate(held);
        self.started.truncate(held);
        self.skipped.truncate(held);
        self.freed.truncate(held);
    }

    /// One page past the end the file is cut back to moved into a free
    /// page below it, which is `incrVacuumStep` at a commit.
    fn vacuum_step(&mut self, last: u32, number: u32) -> Result<(), Error> {
        if self.is_map(number) || self.freelist_count == 0 {
            return Ok(());
        }
        let (kind, parent) = self.point_of(number)?;
        match kind {
            // A free page past the end is dropped with the end, and the
            // free list is dropped whole afterwards.
            Point::Free => return Ok(()),
            Point::Root => return Err(Error::Page(number)),
            Point::Head | Point::Tail | Point::Branch => {}
        }
        // The free pages are taken in turn until one lies below the end
        // the file is cut back to. A page past that end is one the file
        // gives up, so the free list runs out only where the end was
        // counted wrong.
        let mut into = last.saturating_add(1);
        while into > last {
            if self.freelist_count == 0 {
                return Err(Error::Balance);
            }
            into = self.plain(0)?;
        }
        self.relocate(number, kind, parent, into)
    }

    /// One page written where another lay, with every page that names it
    /// and every page it names written again, which is `relocatePage`.
    fn relocate(&mut self, from: u32, kind: Point, parent: u32, into: u32) -> Result<(), Error> {
        let bytes = self.bytes(from)?.to_vec();
        self.keep(into);
        self.put_page(into, &bytes)?;
        if matches!(kind, Point::Branch | Point::Root) {
            self.point_cells(into)?;
        } else {
            let next = crate::bytes::u32_at(&bytes, 0).ok_or(Error::Overrun)?;
            if next != 0 {
                self.point(next, Point::Tail, into)?;
            }
        }
        // No page names a root, which is what `relocatePage` leaves the
        // pointer of `iPtrPage` alone for.
        if kind != Point::Root {
            self.repoint(parent, from, into, kind)?;
        }
        self.point(into, kind, parent)
    }

    /// The root page `from` written where `into` lay, with the pages its
    /// cells name written to say so, which is `relocatePage` of
    /// `research/sqlite/src/btree.c` for a `PTRMAP_ROOTPAGE`.
    ///
    /// Moving one root costs O(n) in the cells of it.
    ///
    /// # Errors
    ///
    /// [`Error`] names a page the file does not hold.
    pub fn move_root(&mut self, from: u32, into: u32) -> Result<(), Error> {
        self.relocate(from, Point::Root, 0, into)
    }

    /// Whether the file keeps pointer maps, which is what `PRAGMA
    /// auto_vacuum` turns on.
    #[must_use]
    pub const fn vacuuming(&self) -> bool {
        self.vacuum
    }

    /// The pointer on `parent` that named `from` written to name `into`,
    /// which is `modifyPagePointer`.
    fn repoint(&mut self, parent: u32, from: u32, into: u32, kind: Point) -> Result<(), Error> {
        if kind == Point::Tail {
            self.put(parent, 0, &into.to_be_bytes());
            return Ok(());
        }
        let at = {
            let page = self.page(parent)?;
            let mut found = None;
            for index in 0..page.cells() {
                if kind == Point::Head {
                    // A cell of an index page carries a chain as a cell
                    // of a table leaf does, so the page the chain begins
                    // on is read from the cell and not from the row.
                    if let Some((head, at)) = page.overflow(index)?
                        && head == from
                    {
                        found = Some(at);
                        break;
                    }
                    continue;
                }
                if page.child(index)? == from {
                    found = Some(page.cell_offset(index)?);
                    break;
                }
            }
            match found {
                Some(at) => at,
                // No cell names it, so the page to the right does.
                None => page.kind().header_len().saturating_sub(4),
            }
        };
        self.put(parent, at, &into.to_be_bytes());
        Ok(())
    }

    /// One more page at the end of the file, and the number it has.
    fn grow(&mut self) -> u32 {
        self.held.push(alloc::vec![0u8; self.page_size]);
        self.before.push(None);
        self.started.push(None);
        self.skipped.push(None);
        self.freed.push(false);
        self.count()
    }

    /// Whether the transaction has opened any page to write, which is
    /// what says the commit has anything to write at all.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.before.iter().any(Option::is_some)
    }

    /// Commits what the transaction wrote and begins the next: a page
    /// the transaction freed onto a trunk keeps in the file what it held
    /// when the transaction began.
    pub fn begin(&mut self) {
        self.before.fill(None);
        self.started.fill(None);
        self.freed.fill(false);
        self.journalled.clear();
        self.spills.clear();
        self.origin = self.count();
        self.list = (self.freelist, self.freelist_count);
    }

    /// Every page the transaction opened put back as it was, the file
    /// back to the length it had, and the free list back to the pages it
    /// named, which is what playing the rollback journal back does.
    ///
    /// A rollback costs O(n) in the pages the transaction opened.
    pub fn rollback(&mut self) {
        // A transaction that gave pages up at the end of the file left
        // it shorter than it was, and the rollback writes it back to the
        // length it had before every page of it is written back.
        self.held
            .resize(size(u64::from(self.origin)), alloc::vec![0; self.page_size]);
        for (slot, before) in self.held.iter_mut().zip(&self.before) {
            if let Some(bytes) = before {
                slot.clone_from(bytes);
            }
        }
        self.held.truncate(size(u64::from(self.origin)));
        (self.freelist, self.freelist_count) = self.list;
        self.before.fill(None);
        self.started.fill(None);
        self.skipped.fill(None);
        self.freed.fill(false);
        self.journalled.clear();
        self.spills.clear();
    }

    /// Begins one statement of a transaction, which is the statement
    /// journal `sqlite3VdbeOpenStatement` opens: the pages the
    /// statement opens to write are kept as it found them, so that a
    /// statement that refuses leaves the transaction where it stood.
    pub fn mark(&mut self) {
        self.started.fill(None);
        self.statement = (self.count(), self.freelist, self.freelist_count);
    }

    /// Every page the statement opened put back as it was, the file
    /// back to the length it had and the free list back to the pages it
    /// named, which is what playing the statement journal back does.
    ///
    /// Undoing a statement costs O(n) in the pages it opened.
    pub fn undo(&mut self) {
        let started = core::mem::take(&mut self.started);
        for (at, start) in started.iter().enumerate() {
            if let Some(start) = start {
                one(&mut self.held, at, &start.held);
                one(&mut self.skipped, at, &start.skipped);
                one(&mut self.freed, at, &start.freed);
            }
        }
        self.started = started;
        let (count, freelist, freelist_count) = self.statement;
        self.freelist = freelist;
        self.freelist_count = freelist_count;
        let pages = size(u64::from(count));
        // A statement that gave pages up at the end of the file left it
        // shorter than it was, and undoing the statement writes it back
        // to the length it had.
        self.held.resize(pages, alloc::vec![0; self.page_size]);
        self.held.truncate(pages);
        self.before.truncate(pages);
        self.started.truncate(pages);
        self.skipped.truncate(pages);
        self.freed.truncate(pages);
        self.started.fill(None);
    }

    /// Opens page `number` to write, which is `sqlite3PagerWrite`: what
    /// the page holds now is kept, so that freeing it can leave it out of
    /// the commit, and a page an earlier commit left out is written
    /// again.
    fn keep(&mut self, number: u32) {
        let at = size(u64::from(number)).saturating_sub(1);
        let held = self.held.get(at).cloned();
        self.start(at);
        one(&mut self.skipped, at, &None);
        let mut first = false;
        for slot in self.before.iter_mut().skip(at).take(1) {
            if slot.is_none() {
                slot.clone_from(&held);
                first = true;
            }
        }
        // A page the database did not hold when the transaction began
        // is not in the journal, because rolling back shortens the file
        // to the pages it held.
        if first && number <= self.origin {
            self.journalled.push(number);
            self.fills();
        }
    }

    /// Records a spill where the pages the cache holds since the last one
    /// reach the count the cache is held to, which is `pagerStress` of
    /// `research/sqlite/src/pager.c` writing a page out because the cache
    /// is full.
    ///
    /// `syncJournal` holds the journal on the disk before such a page
    /// reaches the file, so one spill stands for the records written
    /// since the spill before it and for the one sync that holds them.
    fn fills(&mut self) {
        let Some(caching) = self.caching else {
            return;
        };
        let held = self.spills.last().copied().unwrap_or(0);
        if self.journalled.len().saturating_sub(held) >= caching {
            self.spills.push(self.journalled.len());
        }
    }

    /// How many pages the cache holds before it writes one out.
    pub const fn caching(&mut self, pages: usize) {
        self.caching = Some(pages);
    }

    /// The pages every spill of this transaction wrote into the file, in
    /// the order their numbers run, which a rollback writes back out of
    /// the journal.
    #[must_use]
    pub fn spilled(&self) -> Vec<u32> {
        let most = self.spills.last().copied().unwrap_or(0);
        let mut out: Vec<u32> = self.journalled.iter().copied().take(most).collect();
        out.sort_unstable();
        out
    }

    /// The pages each spill of this transaction wrote into the file, one
    /// run per spill, in the order the spills ran.
    ///
    /// Reading them costs O(n) in the pages the transaction journalled.
    #[must_use]
    pub fn spilling(&self) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        let mut held = 0_usize;
        for most in &self.spills {
            out.push(
                self.journalled
                    .get(held..*most)
                    .unwrap_or_default()
                    .to_vec(),
            );
            held = *most;
        }
        out
    }

    /// What one page held when the transaction began, and nothing where
    /// the transaction has not opened it.
    #[must_use]
    pub fn before(&self, number: u32) -> Option<&[u8]> {
        let at = size(u64::from(number)).saturating_sub(1);
        self.before.get(at)?.as_deref()
    }

    /// The page at `at` as the statement found it, kept where the
    /// statement has not opened it yet.
    fn start(&mut self, at: usize) {
        if self.started.get(at).is_some_and(Option::is_some) {
            return;
        }
        let start = Start {
            held: self.held.get(at).cloned().unwrap_or_default(),
            skipped: self.skipped.get(at).and_then(Clone::clone),
            freed: self.freed.get(at).copied().unwrap_or(false),
        };
        one(&mut self.started, at, &Some(start));
    }

    /// The page the free list gives up, or nothing where the list is
    /// empty.
    fn take(&mut self, nearby: u32) -> Result<Option<u32>, Error> {
        let held = self.freelist_count;
        if held == 0 {
            return Ok(None);
        }
        // Page one holds how many pages the free list has, so taking one
        // writes page one.
        self.keep(1);
        self.freelist_count = held.saturating_sub(1);
        let trunk = self.freelist;
        let leaves = self.word(trunk, 4)?;
        if leaves == 0 {
            // A trunk with no leaf of its own is the page.
            self.freelist = self.word(trunk, 0)?;
            return Ok(Some(trunk));
        }
        // The leaf nearest the page the caller named, which keeps a
        // tree it grows out of pages that lie near each other.
        let mut closest = 0;
        if nearby > 0 {
            let mut dist = self.word(trunk, slot(0))?.abs_diff(nearby);
            for index in 1..leaves {
                let other = self.word(trunk, slot(index))?.abs_diff(nearby);
                if other < dist {
                    closest = index;
                    dist = other;
                }
            }
        }
        let number = self.word(trunk, slot(closest))?;
        // The last leaf takes the place of the one that was taken, which
        // is what `allocateBtreePage` does to leave the array whole.
        let last = self.word(trunk, slot(leaves.saturating_sub(1)))?;
        self.put(trunk, slot(closest), &last.to_be_bytes());
        self.put(trunk, 4, &leaves.saturating_sub(1).to_be_bytes());
        Ok(Some(number))
    }

    /// One page put on the free list, which is `freePage2`: a leaf of
    /// the first trunk where that trunk has room, and the new first
    /// trunk where it has not.
    ///
    /// Nothing of the page itself is written where the page becomes a
    /// leaf, so a page the free list holds keeps the bytes it held.
    ///
    /// The list this walks is one this crate wrote, so the checks the C
    /// library makes against a corrupt one are not written here, which
    /// D-164 records the reason for.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for page one, which no free list holds, and for a
    /// page the database does not hold.
    pub fn release(&mut self, number: u32) -> Result<(), Error> {
        if number < 2 || number > self.count() {
            return Err(Error::Page(number));
        }
        let held = self.freelist_count;
        // Page one holds how many pages the free list has, so freeing
        // one writes page one.
        self.keep(1);
        self.freelist_count = held.saturating_add(1);
        self.point(number, Point::Free, 0)?;
        let trunk = self.freelist;
        if held != 0 {
            let leaves = self.word(trunk, 4)?;
            let most = u32::try_from(self.usable.saturating_div(4)).unwrap_or(0);
            if leaves < most.saturating_sub(8) {
                let at = size(u64::from(leaves)).saturating_mul(4).saturating_add(8);
                self.put(trunk, 4, &leaves.saturating_add(1).to_be_bytes());
                self.put(trunk, at, &number.to_be_bytes());
                self.skip(number);
                one(
                    &mut self.freed,
                    size(u64::from(number)).saturating_sub(1),
                    &true,
                );
                return Ok(());
            }
        }
        // The page becomes the trunk the list begins at, naming the one
        // that began it.
        self.put(number, 0, &trunk.to_be_bytes());
        self.put(number, 4, &0_u32.to_be_bytes());
        self.freelist = number;
        Ok(())
    }

    /// Leaves page `number` out of what the commit writes, which is
    /// `sqlite3PagerDontWrite`: the file keeps what the page held when
    /// the transaction began.
    fn skip(&mut self, number: u32) {
        let at = size(u64::from(number)).saturating_sub(1);
        self.start(at);
        let was = self
            .before
            .get(at)
            .and_then(Clone::clone)
            .or_else(|| self.held.get(at).cloned());
        one(&mut self.skipped, at, &was);
    }

    /// Four bytes of a page, big-endian.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold, and
    /// [`Error::Overrun`] where the four bytes are not on it.
    fn word(&self, number: u32, at: usize) -> Result<u32, Error> {
        crate::bytes::u32_at(self.bytes(number)?, at).ok_or(Error::Overrun)
    }

    /// One page, to read.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold, and
    /// whatever [`Page::parse`] refuses the page with.
    pub fn page(&self, number: u32) -> Result<Page<'_>, Error> {
        let bytes = self.bytes(number)?;
        Page::parse(bytes, number, u32::try_from(self.usable).unwrap_or(0))
    }

    /// One page, to change.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold, and
    /// whatever [`Writer::open`] refuses the page with.
    pub fn writer(&mut self, number: u32) -> Result<Writer<'_>, Error> {
        let usable = u32::try_from(self.usable).unwrap_or(0);
        self.keep(number);
        let at = size(u64::from(number))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        let bytes = self.held.get_mut(at).ok_or(Error::Page(number))?;
        Writer::open(bytes, number, usable)
    }

    /// The bytes of one page.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold.
    pub fn bytes(&self, number: u32) -> Result<&[u8], Error> {
        let at = size(u64::from(number))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        self.held
            .get(at)
            .map(Vec::as_slice)
            .ok_or(Error::Page(number))
    }

    /// Two pages changing places in the file, which is what
    /// `sqlite3PagerRekey` does to keep the pages of a balance in the
    /// order their numbers run. What is on them changes with them.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold.
    pub fn swap(&mut self, one: u32, other: u32) -> Result<(), Error> {
        let one = size(u64::from(one)).checked_sub(1).ok_or(Error::Page(0))?;
        let other = size(u64::from(other))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        if one >= self.held.len() || other >= self.held.len() {
            return Err(Error::Page(0));
        }
        self.held.swap(one, other);
        Ok(())
    }

    /// Opens page `number` to write without writing anything of it,
    /// which is `sqlite3PagerWrite`: the journal of the transaction
    /// holds the page from here on.
    pub fn open(&mut self, number: u32) {
        self.keep(number);
    }

    /// Puts `cell` at `at` of page `number` and answers whether the page
    /// had room for it, which is `insertCell`: a page with no room is
    /// not opened to write, so the journal does not hold it.
    ///
    /// # Errors
    ///
    /// Whatever reading or writing the page refuses.
    pub fn put_cell(&mut self, number: u32, at: usize, cell: &[u8]) -> Result<bool, Error> {
        if self.page(number)?.free()? < crate::page::cell_room(cell.len()).saturating_add(2) {
            return Ok(false);
        }
        let put = self.writer(number)?.insert(at, cell)?;
        self.point_cells(number)?;
        Ok(put)
    }

    /// Writes a whole page, which is what a caller that built one out of
    /// its own bytes hands back.
    ///
    /// # Errors
    ///
    /// [`Error::Page`] for a page the database does not hold.
    pub fn put_page(&mut self, number: u32, bytes: &[u8]) -> Result<(), Error> {
        let at = size(u64::from(number))
            .checked_sub(1)
            .ok_or(Error::Page(0))?;
        let page = self.held.get_mut(at).ok_or(Error::Page(number))?;
        for (slot, byte) in page.iter_mut().zip(bytes) {
            *slot = *byte;
        }
        Ok(())
    }

    /// Writes `bytes` into page `number` at `at`. The callers write into
    /// a page they have just added, so the number is one the database
    /// holds.
    fn put(&mut self, number: u32, at: usize, bytes: &[u8]) {
        self.keep(number);
        let index = size(u64::from(number)).saturating_sub(1);
        for page in self.held.iter_mut().skip(index).take(1) {
            for (slot, byte) in page.iter_mut().skip(at).zip(bytes) {
                *slot = *byte;
            }
        }
    }

    /// The rollback journal the commit of this transaction writes, which
    /// holds every page the transaction changed as it was before it.
    ///
    /// `was` is the header the last commit wrote, because the record of
    /// page one holds the header as it was and this crate keeps that
    /// header beside the pages rather than on page one.
    ///
    /// Page one is in every journal: the commit writes the change
    /// counter into it, which is `pager_incr_changecounter`, so a
    /// transaction that never touched page one still holds it last.
    ///
    /// `nonce` and `sector` are what [`crate::journal::write`] takes.
    #[must_use]
    pub fn journal(&self, was: &Header, nonce: u32, sector: u32) -> Vec<u8> {
        let content = |number: u32| -> Vec<u8> {
            let at = size(u64::from(number)).saturating_sub(1);
            let mut page = self
                .before
                .get(at)
                .and_then(Clone::clone)
                .or_else(|| self.held.get(at).cloned())
                .unwrap_or_default();
            if number == 1 {
                for (slot, byte) in page.iter_mut().zip(was.written()) {
                    *slot = byte;
                }
            }
            page
        };
        let mut numbers = self.journalled.clone();
        if !numbers.contains(&1) {
            numbers.push(1);
        }
        let pages: Vec<Vec<u8>> = numbers.iter().copied().map(content).collect();
        let records: Vec<(u32, &[u8])> = numbers
            .iter()
            .copied()
            .zip(pages.iter().map(Vec::as_slice))
            .collect();
        crate::journal::write(&records, self.origin, nonce, sector)
    }

    /// The frames the commit of this transaction writes into the
    /// write-ahead log, which is `sqlite3PcacheDirtyList` and
    /// `pagerWalFrames`: every page the transaction wrote, in the order
    /// their numbers run.
    ///
    /// Page one is among them where the transaction changed how many
    /// pages the database has, because growing the file opens page one
    /// to write. `now` is the header the commit writes, which this crate
    /// keeps beside the pages rather than on page one; the page count
    /// and the free list of it are the ones the pages hold, whatever
    /// the caller's header says.
    #[must_use]
    pub fn frames(&self, now: &Header) -> Vec<(u32, Vec<u8>)> {
        let mut now = *now;
        now.pages = self.count();
        now.freelist = self.freelist;
        now.freelist_pages = self.freelist_count;
        let numbers: Vec<u32> = (1..=self.count())
            .filter(|number| {
                let at = size(u64::from(*number)).saturating_sub(1);
                self.before.get(at).is_some_and(Option::is_some)
            })
            .collect();
        numbers
            .into_iter()
            .map(|number| {
                let at = size(u64::from(number)).saturating_sub(1);
                let mut page = self.held.get(at).cloned().unwrap_or_default();
                if number == 1 {
                    for (slot, byte) in page.iter_mut().zip(now.written()) {
                        *slot = byte;
                    }
                }
                (number, page)
            })
            .collect()
    }

    /// The file the pages make, with the header written into the first
    /// hundred bytes of page one. The free list of the header is the one
    /// the pages hold, whatever the caller's header says.
    #[must_use]
    pub fn written(&self, header: &Header) -> Vec<u8> {
        let mut header = *header;
        header.freelist = self.freelist;
        header.freelist_pages = self.freelist_count;
        let pages: Vec<Vec<u8>> = self
            .held
            .iter()
            .enumerate()
            .map(
                |(at, page)| match self.skipped.get(at).and_then(Option::as_ref) {
                    Some(bytes) => bytes.clone(),
                    None => page.clone(),
                },
            )
            .collect();
        crate::image::write(&header, &pages)
    }

    /// The file as it stood when the transaction began: the pages the
    /// transaction opened to write as it found them, the length the file
    /// had, and the header page one held then.
    ///
    /// `sqlite3PagerSharedLock` answers a reader the pages of the file,
    /// so a connection that did not begin the transaction reads these
    /// and not the pages the transaction has written.
    ///
    /// Building the image costs O(n) in the pages.
    #[must_use]
    pub fn committed(&self, header: &Header) -> Vec<u8> {
        let pages: Vec<Vec<u8>> = (0..size(u64::from(self.origin)))
            .map(|at| match self.before.get(at).and_then(Option::as_ref) {
                Some(bytes) => bytes.clone(),
                None => self.held.get(at).cloned().unwrap_or_default(),
            })
            .collect();
        let mut held = pages
            .first()
            .and_then(|page| Header::parse(page).ok())
            .unwrap_or(*header);
        (held.freelist, held.freelist_pages) = self.list;
        crate::image::write(&held, &pages)
    }
}

/// Where a trunk of the free list holds the leaf at `index`, which is
/// the word after the two the trunk begins with.
fn slot(index: u32) -> usize {
    size(u64::from(index)).saturating_mul(4).saturating_add(8)
}

/// Whether the free page `page` is the one `wanted` names, which for
/// `exact` is that page alone and for the other is every page at or
/// below it.
const fn fits(page: u32, wanted: u32, exact: bool) -> bool {
    if exact {
        page == wanted
    } else {
        page <= wanted
    }
}

/// How many pages a file of `origin` pages with `free` of them free is
/// cut back to, which is `finalDbSize`.
///
/// The count of map pages the file gives up is taken with the wrapping
/// the C library takes it with, because `free` is fewer than `origin`
/// and the subtraction is over unsigned numbers there as here.
///
/// `finalDbSize` steps the answer back past a pointer-map page it lands
/// on. The count of map pages given up is what makes it land past the
/// last of them, so no page size, no file length and no free list
/// reaches that step, and D4 has an unreachable branch deleted rather
/// than exempted.
pub(crate) fn final_size(usable: usize, origin: u32, free: u32) -> u32 {
    let entries = u32::try_from(usable.saturating_div(ENTRY))
        .unwrap_or(1)
        .max(1);
    let maps = free
        .wrapping_sub(origin)
        .wrapping_add(map_page(usable, origin))
        .wrapping_add(entries)
        .checked_div(entries)
        .unwrap_or(0);
    origin.saturating_sub(free).saturating_sub(maps)
}

/// The pointer-map page that carries the entry for `number` over pages
/// of `usable` bytes, which is `ptrmapPageno` of `src/btree.c`.
///
/// One map page carries one entry per five of its usable bytes and one
/// more for itself, so the map pages lie at page two and every that many
/// pages after it. A file of a gibibyte or more holds the lock page as
/// well, which this crate does not write and so does not step over.
pub(crate) fn map_page(usable: usize, number: u32) -> u32 {
    let span = u32::try_from(usable.saturating_div(ENTRY)).unwrap_or(0);
    let span = span.saturating_add(1);
    let which = number.saturating_sub(2).checked_div(span).unwrap_or(0);
    which.saturating_mul(span).saturating_add(2)
}

/// What of a payload stays on the page, with the rest written onto
/// overflow pages.
fn spilled<'a>(pages: &mut Pages, payload: &'a [u8]) -> Result<(&'a [u8], Option<u32>), Error> {
    spilled_as(pages, payload, Kind::LeafTable)
}

/// The same for an entry of an index, which keeps less of its payload
/// on the page it lies on.
fn spilled_entry<'a>(
    pages: &mut Pages,
    payload: &'a [u8],
) -> Result<(&'a [u8], Option<u32>), Error> {
    spilled_as(pages, payload, Kind::LeafIndex)
}

/// What of a payload stays on a page of `kind`, with the rest written
/// onto overflow pages.
fn spilled_as<'a>(
    pages: &mut Pages,
    payload: &'a [u8],
    kind: Kind,
) -> Result<(&'a [u8], Option<u32>), Error> {
    let local = local_len(payload.len(), pages.usable(), kind);
    let held = payload.get(..local).unwrap_or_default();
    let mut rest = payload.get(local..).unwrap_or_default();
    if rest.is_empty() {
        return Ok((held, None));
    }
    let span = pages.usable().saturating_sub(4);
    // Each page of the chain is taken near the one before it, which is
    // the page `fillInCell` names when it asks for the next, stepped
    // past every pointer map on the way.
    let first = pages.add_plain(pages.after_maps(0))?;
    // The first page of a chain is written into the map with no page
    // naming it, because the cell it belongs to is not placed yet;
    // `insertCell` writes the page that holds the cell over it.
    pages.point(first, Point::Head, 0)?;
    let mut number = first;
    // The chain is walked once, and every page but the last is filled,
    // so the whole of it is O(n) in the bytes that run on.
    loop {
        let taken = rest.get(..span.min(rest.len())).unwrap_or_default();
        pages.put(number, 4, taken);
        rest = rest.get(taken.len()..).unwrap_or_default();
        if rest.is_empty() {
            // `fillInCell` writes nought over the pointer of every page
            // it takes, and the page the chain ends on keeps it. A page
            // the free list gave back holds what it held before, so the
            // pointer is written and not left.
            pages.put(number, 0, &0_u32.to_be_bytes());
            break;
        }
        let next = pages.add_plain(pages.after_maps(number))?;
        pages.point(next, Point::Tail, number)?;
        pages.put(number, 0, &next.to_be_bytes());
        number = next;
    }
    Ok((held, Some(first)))
}

/// How an index tree orders its entries.
///
/// The collation of each column says how two values of that column
/// compare, and the encoding says what bytes the file holds text in,
/// which is what `sqlite3VdbeRecordCompare` compares.
#[derive(Clone, Copy)]
pub struct Ordering<'a> {
    /// How each place of the index compares, in its order.
    pub collations: &'a [crate::value::Placing],
    /// The encoding the file writes its text in.
    pub encoding: crate::header::Encoding,
}

/// Puts one entry in the index tree that begins at `root`.
///
/// `key` is what `record` holds, decoded, because an index tree is
/// ordered by the values of its entries and not by a number: the walk
/// compares `key` against the entry each cell holds, under `collations`
/// column by column, and the key of the row the entry points at settles
/// two entries that agree on every column before it.
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, and
/// whatever reading or writing a page of it refuses.
/// `bulk` says the entries arrive in the order they run, which is what
/// a `CREATE INDEX` does through its sorter: the balance then fills the
/// left page and leaves the right one to the entries still to come.
pub fn insert_entry(
    pages: &mut Pages,
    root: u32,
    record: &[u8],
    key: &[Value],
    order: Ordering<'_>,
    bulk: bool,
) -> Result<(), Error> {
    let key = &crate::value::written(key, order.encoding);
    let collations = order.collations;
    let path = place_entry(pages, root, key, collations)?;
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    let (local, overflow) = spilled_entry(pages, record)?;
    let cell = write_cell(&Cell::IndexLeaf {
        payload: Payload {
            local,
            total: record.len(),
            overflow,
        },
    });
    if pages.put_cell(leaf, at, &cell)? {
        return Ok(());
    }
    balance(pages, &path, alloc::vec![(at, cell)], None, bulk).map(|_| ())
}

/// Where an entry of `key` belongs in the index tree at `root`: one
/// page and one place per level, the leaf last.
///
/// The walk is O(log n) pages, and one page costs O(k log k) comparisons
/// over its `k` cells where the search is a scan, which this is.
fn place_entry(
    pages: &Pages,
    root: u32,
    key: &[Value],
    collations: &[crate::value::Placing],
) -> Result<Vec<(u32, usize)>, Error> {
    let mut path = Vec::new();
    let mut number = root;
    for _ in 0..MAX_DEPTH {
        let cells = pages.page(number)?.cells();
        let mut at = cells;
        for index in 0..cells {
            if order_of_entry(pages, number, index, key, collations)?.is_ge() {
                at = index;
                break;
            }
        }
        path.push((number, at));
        let page = pages.page(number)?;
        if !page.kind().is_interior() {
            return Ok(path);
        }
        number = if at == cells {
            page.right_most().ok_or(Error::Overrun)?
        } else {
            page.child(at)?
        };
    }
    Err(Error::Depth)
}

/// Where the entry of cell `at` of page `number` stands against `key`.
fn order_of_entry(
    pages: &Pages,
    number: u32,
    at: usize,
    key: &[Value],
    collations: &[crate::value::Placing],
) -> Result<core::cmp::Ordering, Error> {
    let page = pages.page(number)?;
    let payload = page.entry(at)?;
    // An entry that runs onto a chain is read whole before it is
    // compared, because a column of the key may be the part that ran
    // on.
    let mut held = Vec::new();
    let bytes = if payload.is_whole() {
        payload.local
    } else {
        read_chain(pages, &payload, &mut held)?;
        &held
    };
    let record = crate::record::Record::parse(bytes)?;
    let mut orders = Vec::new();
    for (at, wanted) in key.iter().enumerate() {
        let mine = record.value(at)?.map_or(Value::Null, held_value);
        let placing = collations.get(at).copied().unwrap_or_default();
        orders.push(crate::value::compare_placed(&mine, wanted, placing));
    }
    Ok(orders
        .into_iter()
        .find(|order| *order != core::cmp::Ordering::Equal)
        .unwrap_or(core::cmp::Ordering::Equal))
}

/// One value of a record as a value a comparison takes, which is the
/// bytes as they are stored: an index compares what it holds and not
/// what an encoding makes of it.
pub(crate) fn held_value(value: crate::record::Value<'_>) -> Value {
    match value {
        crate::record::Value::Null => Value::Null,
        crate::record::Value::Int(number) => Value::Int(number),
        crate::record::Value::Real(number) => Value::Real(number),
        crate::record::Value::Text(bytes) => Value::Text(bytes.to_vec()),
        crate::record::Value::Blob(bytes) => Value::Blob(bytes.to_vec()),
    }
}

/// The whole of a payload that runs onto a chain, read into `into`.
fn read_chain(pages: &Pages, payload: &Payload<'_>, into: &mut Vec<u8>) -> Result<(), Error> {
    into.clear();
    into.extend_from_slice(payload.local);
    let span = pages.usable().saturating_sub(4);
    let mut number = payload.overflow;
    while let Some(page) = number {
        let bytes = pages.bytes(page)?;
        let rest = payload.total.saturating_sub(into.len());
        into.extend_from_slice(
            bytes
                .get(4..span.min(rest).saturating_add(4))
                .ok_or(Error::Overrun)?,
        );
        let next = crate::bytes::u32_at(bytes, 0).ok_or(Error::Overrun)?;
        number = (next != 0).then_some(next);
    }
    Ok(())
}

/// Takes the entry of `key` out of the index tree that begins at
/// `root`, and answers whether the tree held one.
///
/// This is `sqlite3BtreeDelete` over an index cursor: an entry on a
/// leaf is dropped and the leaf is balanced; an entry on an interior
/// page is replaced by the entry before it, which lies at the end of
/// the right-most leaf under the page the entry names, and both the
/// leaf and the interior page are balanced.
///
/// The descent is O(log n) pages and the balance O(log n) more.
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, and
/// whatever reading, balancing or freeing a page of it refuses.
pub fn remove_entry(
    pages: &mut Pages,
    root: u32,
    key: &[Value],
    order: Ordering<'_>,
) -> Result<bool, Error> {
    let key = &crate::value::written(key, order.encoding);
    let collations = order.collations;
    let Some(path) = find_entry(pages, root, key, collations)? else {
        return Ok(false);
    };
    let (holder, at) = *path.last().ok_or(Error::Depth)?;
    let interior = pages.page(holder)?.kind().is_interior();
    let under = if interior {
        let child = pages.page(holder)?.child(at)?;
        last_under(pages, child)?
    } else {
        Vec::new()
    };
    let overflow = pages.page(holder)?.entry(at)?.overflow;
    pages.open(holder);
    clear(pages, overflow)?;
    pages.writer(holder)?.remove(at)?;
    if !interior {
        balance(pages, &path, Vec::new(), None, false)?;
        return Ok(true);
    }
    replace_entry(pages, &path, &under)
}

/// The entry an interior page lost given the entry before it, and both
/// pages balanced.
///
/// `path` ends on the interior page and `under` runs from the page the
/// lost entry named down to the leaf that holds the entry before it.
fn replace_entry(
    pages: &mut Pages,
    path: &[(u32, usize)],
    under: &[(u32, usize)],
) -> Result<bool, Error> {
    let (holder, at) = *path.last().ok_or(Error::Depth)?;
    let (below, _) = *under.first().ok_or(Error::Depth)?;
    let (leaf, last) = *under.last().ok_or(Error::Depth)?;
    // The entry moves up as a divider, which names the page the lost
    // entry named in its first four bytes.
    let mut divider = below.to_be_bytes().to_vec();
    divider.extend_from_slice(&write_cell(&pages.page(leaf)?.cell(last)?));
    pages.open(leaf);
    let mut spill: Vec<Spill> = Vec::new();
    if !pages.put_cell(holder, at, &divider)? {
        spill.push((at, divider));
    }
    pages.writer(leaf)?.remove(last)?;
    let mut down = path.to_vec();
    down.extend_from_slice(under);
    let held = (!spill.is_empty()).then(|| (holder, spill.clone()));
    let left = balance(pages, &down, Vec::new(), held, false)?;
    // The balance of the leaf stops where it has no more to do, so the
    // interior page is balanced on its own where the balance stopped
    // below it.
    if left > path.len() {
        balance(pages, path, spill, None, false)?;
    }
    Ok(true)
}

/// The key of the row an entry of `key` belongs to, where the index
/// tree at `root` holds one, which is the `width` values after the
/// columns.
///
/// `key` is the columns of the index and not the key of the row after
/// them, so an entry whose columns are these answers whatever row it
/// belongs to, which is what a `UNIQUE` is held to. The key of a row
/// is one value where the table has a rowid and the columns of the
/// `PRIMARY KEY` where the table keeps its rows in the key's own tree.
/// The walk is O(log n) pages.
///
/// # Errors
///
/// [`Error`] names whatever reading a page refuses.
pub(crate) fn entry_tail_at(
    pages: &Pages,
    root: u32,
    key: &[Value],
    order: Ordering<'_>,
    width: usize,
) -> Result<Option<Vec<Value>>, Error> {
    let key = &crate::value::written(key, order.encoding);
    let collations = order.collations;
    let Some(path) = find_entry(pages, root, key, collations)? else {
        return Ok(None);
    };
    let (number, at) = *path.last().ok_or(Error::Depth)?;
    let page = pages.page(number)?;
    let payload = page.entry(at)?;
    let mut held = Vec::new();
    let bytes = if payload.is_whole() {
        payload.local
    } else {
        read_chain(pages, &payload, &mut held)?;
        &held
    };
    let record = crate::record::Record::parse(bytes)?;
    let mut out = Vec::new();
    for step in 0..width {
        let place = key.len().saturating_add(step);
        out.push(record.value(place)?.map_or(Value::Null, held_value));
    }
    Ok(Some(out))
}

/// Where the entry of `key` lies in the index tree at `root`: one page
/// and one place per level, the page that holds the entry last.
/// Answers nothing where the tree holds no entry of `key`.
///
/// The descent is O(log n) pages, and one page costs O(k) comparisons
/// over its `k` cells.
pub(crate) fn find_entry(
    pages: &Pages,
    root: u32,
    key: &[Value],
    collations: &[crate::value::Placing],
) -> Result<Option<Vec<(u32, usize)>>, Error> {
    let mut path = Vec::new();
    let mut number = root;
    for _ in 0..MAX_DEPTH {
        let cells = pages.page(number)?.cells();
        let mut at = cells;
        let mut same = false;
        for index in 0..cells {
            let order = order_of_entry(pages, number, index, key, collations)?;
            if order.is_ge() {
                at = index;
                same = order == core::cmp::Ordering::Equal;
                break;
            }
        }
        path.push((number, at));
        if same {
            return Ok(Some(path));
        }
        let page = pages.page(number)?;
        if !page.kind().is_interior() {
            return Ok(None);
        }
        number = if at == cells {
            page.right_most().ok_or(Error::Overrun)?
        } else {
            page.child(at)?
        };
    }
    Err(Error::Depth)
}

/// The path from `child` down to the leaf that holds the last entry
/// under it, which is the entry before the one the divider above
/// `child` holds. The leaf stands at its last cell.
fn last_under(pages: &Pages, child: u32) -> Result<Vec<(u32, usize)>, Error> {
    let mut path = Vec::new();
    let mut number = child;
    for _ in 0..MAX_DEPTH {
        let page = pages.page(number)?;
        let cells = page.cells();
        if !page.kind().is_interior() {
            path.push((number, cells.saturating_sub(1)));
            return Ok(path);
        }
        path.push((number, cells));
        number = page.right_most().ok_or(Error::Overrun)?;
    }
    Err(Error::Depth)
}

/// Puts one row in the table tree that begins at `root`.
///
/// A leaf that will not hold the cell is balanced, and so is every page
/// above it that the balance leaves a divider it will not hold.
///
/// # Errors
///
/// [`Error::Balance`] for a balance this crate does not write, which is
/// one that frees a page; [`Error::Depth`] for a tree deeper than this
/// crate walks; and whatever reading a page of it refuses.
pub fn insert(pages: &mut Pages, root: u32, rowid: i64, record: &[u8]) -> Result<(), Error> {
    let (local, overflow) = spilled(pages, record)?;
    let cell = write_cell(&Cell::TableLeaf {
        rowid,
        payload: Payload {
            local,
            total: record.len(),
            overflow,
        },
    });
    let path = place(pages, root, rowid)?;
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    if pages.put_cell(leaf, at, &cell)? {
        return Ok(());
    }
    balance(pages, &path, alloc::vec![(at, cell)], None, false).map(|_| ())
}

/// Puts a row that the tree already holds there again, and answers
/// whether the tree held one.
///
/// This is `sqlite3BtreeInsert` over a cursor that stands on the row: a
/// payload of the length the old one was is written where the old one
/// lies, chain and all, and anything else frees the chain the row ran
/// onto and writes the cell again, over the old one where the two are
/// the same size and in its place where they are not.
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, and
/// whatever reading or writing a page of it refuses.
pub fn update(pages: &mut Pages, root: u32, rowid: i64, record: &[u8]) -> Result<bool, Error> {
    let path = place(pages, root, rowid)?;
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    let (key, total, local_len, overflow, offset) = {
        let page = pages.page(leaf)?;
        if at >= page.cells() {
            return Ok(false);
        }
        let (key, payload) = page.row(at)?;
        (
            key,
            payload.total,
            payload.local.len(),
            payload.overflow,
            page.cell_offset(at)?,
        )
    };
    if key != rowid {
        return Ok(false);
    }
    let head = crate::bytes::varint_len(u64::try_from(total).unwrap_or(u64::MAX))
        .saturating_add(crate::bytes::varint_len(rowid.cast_unsigned()));
    if total == record.len() {
        // `btreeOverwriteCell`: the payload is the length the old one
        // was, so the new bytes go where the old ones lie and the chain
        // the row runs onto is the chain it keeps.
        let local = record.get(..local_len).unwrap_or_default();
        pages
            .writer(leaf)?
            .overwrite(offset.saturating_add(head), local);
        let mut rest = record.get(local_len..).unwrap_or_default();
        let mut number = overflow;
        let span = pages.usable().saturating_sub(4);
        while let Some(page) = number {
            let taken = rest.get(..span.min(rest.len())).unwrap_or_default();
            pages.put(page, 4, taken);
            rest = rest.get(taken.len()..).unwrap_or_default();
            let next = crate::bytes::u32_at(pages.bytes(page)?, 0).ok_or(Error::Overrun)?;
            number = (next != 0).then_some(next);
        }
        return Ok(true);
    }
    // A payload of another length: the chain the row ran onto goes back
    // on the free list and the cell is written again.
    clear(pages, overflow)?;
    let (local, spill) = spilled(pages, record)?;
    let cell = write_cell(&Cell::TableLeaf {
        rowid,
        payload: Payload {
            local,
            total: record.len(),
            overflow: spill,
        },
    });
    // `sqlite3BtreeInsert`: a cell the length of the old one, where the
    // old one held its whole payload, goes where the old one lies.
    if overflow.is_none() && cell.len() == head.saturating_add(local_len) {
        pages.writer(leaf)?.overwrite(offset, &cell);
        return Ok(true);
    }
    pages.writer(leaf)?.remove(at)?;
    if pages.put_cell(leaf, at, &cell)? {
        return Ok(true);
    }
    balance(pages, &path, alloc::vec![(at, cell)], None, false)?;
    Ok(true)
}

/// Takes the row of `rowid` out of the table tree that begins at `root`,
/// and answers whether the tree held one.
///
/// The overflow pages of the row go on the free list, and a leaf left
/// more than two thirds empty is balanced against its siblings, which is
/// what joins the pages a delete has emptied.
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, and
/// whatever reading or writing a page of it refuses.
pub fn remove(pages: &mut Pages, root: u32, rowid: i64) -> Result<bool, Error> {
    let path = place(pages, root, rowid)?;
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    let page = pages.page(leaf)?;
    if at >= page.cells() {
        return Ok(false);
    }
    let (key, payload) = page.row(at)?;
    if key != rowid {
        return Ok(false);
    }
    let overflow = payload.overflow;
    // `sqlite3PagerWrite` on the leaf comes first, then `clearCell` and
    // `dropCell`, which is the order the pages come onto the free list
    // in and the order the journal holds them in.
    pages.open(leaf);
    clear(pages, overflow)?;
    pages.writer(leaf)?.remove(at)?;
    balance(pages, &path, Vec::new(), None, false)?;
    Ok(true)
}

/// The overflow chain a cell ran onto put back on the free list, which
/// is `clearCell`. One chain is O(n) in the pages it holds.
///
/// # Errors
///
/// [`Error::Overflow`] for a chain that reaches more pages than the
/// database holds, and whatever freeing a page of it refuses.
fn clear(pages: &mut Pages, first: Option<u32>) -> Result<(), Error> {
    let mut number = first;
    let mut read = 0_u32;
    while let Some(page) = number {
        read = read.saturating_add(1);
        if read > pages.count() {
            return Err(Error::Overflow(page));
        }
        let next = crate::bytes::u32_at(pages.bytes(page)?, 0).ok_or(Error::Overrun)?;
        pages.release(page)?;
        number = (next != 0).then_some(next);
    }
    Ok(())
}

/// Every page of the tree at `root` put on the free list, which is
/// `sqlite3BtreeDropTable` for a file that does not vacuum itself.
///
/// `clearDatabasePage` frees what lies under a page before the page
/// itself: the child of a cell, then the chain the cell runs onto, then
/// the page under the right pointer, and the page last. The root goes
/// after all of them.
///
/// Freeing a tree of `n` pages is O(n), and the walk is O(d) deep for a
/// tree of depth `d`.
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, which is
/// what a page that names itself answers, and whatever reading or
/// freeing a page of it refuses.
pub fn destroy(pages: &mut Pages, root: u32) -> Result<(), Error> {
    clear_page(pages, root, false, 0)?;
    pages.release(root)
}

/// The tree at `root` taken out of a file that keeps pointer maps, with
/// the root that carries the largest page number moved into the page the
/// root leaves, and the page that root lay on given up.
///
/// This is `btreeDropTable` of `research/sqlite/src/btree.c:10328`: a
/// root at the end of the file is one the vacuum of the commit cannot
/// move, so the drop moves it while the schema still says where it is.
/// The answer is the page the move took a root off, and nothing where
/// the root dropped was the largest one, so the caller writes the row of
/// the schema that named the moved root.
///
/// Dropping one tree costs O(n) in its pages and O(m) in the cells of
/// the moved root.
///
/// # Errors
///
/// [`Error`] names a page the tree names and the file does not hold.
pub fn destroy_moving(pages: &mut Pages, root: u32, largest: u32) -> Result<Option<u32>, Error> {
    clear_page(pages, root, false, 0)?;
    if root >= largest {
        pages.release(root)?;
        return Ok(None);
    }
    pages.move_root(largest, root)?;
    pages.release(largest)?;
    Ok(Some(largest))
}

/// Every row of the tree at `root` taken out, with the root left as an
/// empty page of `kind`, which is `sqlite3BtreeClearTable`.
///
/// Clearing costs O(n) in the pages of the tree.
///
/// # Errors
///
/// [`Error`] names a page the tree names and the file does not hold.
pub fn clear_tree(pages: &mut Pages, root: u32, kind: Kind) -> Result<(), Error> {
    clear_page(pages, root, false, 0)?;
    pages.blank(root, kind)
}

/// One page of a tree cleared, and freed where `free` says so.
fn clear_page(pages: &mut Pages, number: u32, free: bool, depth: usize) -> Result<(), Error> {
    if depth >= MAX_DEPTH {
        return Err(Error::Depth);
    }
    let (children, chains, right) = {
        let page = pages.page(number)?;
        let interior = page.kind().is_interior();
        let mut children = Vec::new();
        let mut chains = Vec::new();
        for at in 0..page.cells() {
            if interior {
                children.push(page.child(at)?);
            }
            chains.push(match page.cell(at)? {
                Cell::TableLeaf { payload, .. }
                | Cell::IndexLeaf { payload }
                | Cell::IndexInterior { payload, .. } => payload.overflow,
                Cell::TableInterior { .. } => None,
            });
        }
        (children, chains, page.right_most())
    };
    for (at, chain) in chains.iter().enumerate() {
        if let Some(child) = children.get(at) {
            clear_page(pages, *child, true, depth.saturating_add(1))?;
        }
        clear(pages, *chain)?;
    }
    if let Some(right) = right {
        clear_page(pages, right, true, depth.saturating_add(1))?;
    }
    if free {
        pages.release(number)?;
    }
    Ok(())
}

/// A cell a page holds that does not lie on it, which is `apOvfl` and
/// `aiOvfl`: where among the cells of the page the cell belongs, and its
/// bytes. Every one of them is next to the one before, because a page
/// gains them only from the dividers of a balance.
type Spill = (usize, Vec<u8>);

/// A page that will not hold a cell, balanced so that it does, and then
/// its parent where the balance left the parent a divider it will not
/// hold, up to the root.
///
/// This is `balance`: the root of a tree of one page grows a child under
/// it, a cell at the end of the right-most page grows a sibling beside
/// it, and every other cell is the general balance.
fn balance(
    pages: &mut Pages,
    path: &[(u32, usize)],
    mut spill: Vec<Spill>,
    mut above: Option<(u32, Vec<Spill>)>,
    bulk: bool,
) -> Result<usize, Error> {
    let mut stack = path.to_vec();
    // A page more than two thirds empty is balanced as well, so that a
    // delete joins pages rather than leaving a tree of empty ones.
    let least = pages.usable().saturating_mul(2).saturating_div(3);
    // One turn per level, from the leaf to the root, so a tree of depth
    // d costs O(d) balances.
    loop {
        let (page, at) = *stack.last().ok_or(Error::Depth)?;
        if spill.is_empty() && pages.page(page)?.free()? <= least {
            return Ok(stack.len());
        }
        let over = stack
            .len()
            .checked_sub(2)
            .and_then(|level| stack.get(level));
        let Some(&(parent, at_above)) = over else {
            if spill.is_empty() {
                // A root has no sibling to even out against, however
                // empty it is.
                return Ok(stack.len());
            }
            // A tree of one page: the root keeps its place in the file,
            // so the cells of the root move to a child and the root
            // becomes the interior page above it. The root holds no cell
            // after that, so the cell belongs on the child.
            let child = deepen(pages, page)?;
            stack = alloc::vec![(page, 0), (child, at)];
            continue;
        };
        // `balance_quick` is the C library's own routine for one cell at
        // the end of a leaf of a table that the parent's right pointer
        // names: it writes a page of that one cell and hangs it from the
        // parent, which holds only where nothing of the leaf lies to the
        // right of it. A parent on page one is left to the balance
        // proper, because that page begins a hundred bytes in.
        let first = spill.first().map(|(at, _)| *at);
        if pages.page(page)?.kind() == Kind::LeafTable
            && spill.len() == 1
            && first == Some(pages.page(page)?.cells())
            && parent != crate::image::SCHEMA_ROOT
            && pages.page(parent)?.cells() == at_above
        {
            let (_, bytes) = spill.first().ok_or(Error::Balance)?;
            if quick(pages, parent, page, bytes)? {
                return Ok(stack.len());
            }
        }
        pages.open(parent);
        // The cell the parent has no room for is one of the dividers
        // this balance reads, so the balance that reaches the parent
        // takes it and no later one does.
        let mine = above
            .take_if(|(number, _)| *number == parent)
            .map(|(_, cells)| cells)
            .unwrap_or_default();
        spill = balance_nonroot(
            pages,
            parent,
            at_above,
            &spill,
            &mine,
            stack.len() == 2,
            bulk,
        )?;
        stack.pop();
    }
}

/// A root of one page grown into a root over one child, which is
/// `balance_deeper`: the child takes the root's content and the root
/// becomes the interior page above it.
pub(crate) fn deepen(pages: &mut Pages, root: u32) -> Result<u32, Error> {
    let kind = pages.page(root)?.kind();
    let child = pages.add(kind, root)?;
    // `copyNodeContent`: the child takes the content area of the root
    // as it lies, which it can because a cell of a root on page one
    // lies above the hundredth byte already, and then as many bytes
    // from the root's own header as the root's cell pointer array ends
    // at. That count is the array's own offset and not its length, so a
    // root on page one hands the child a hundred bytes of what lies
    // after its array as well.
    let usable = pages.usable();
    let page = pages.page(root)?;
    let header =
        usize::from(root == crate::image::SCHEMA_ROOT).saturating_mul(crate::header::HEADER_LEN);
    let array_end = header
        .saturating_add(kind.header_len())
        .saturating_add(page.cells().saturating_mul(2));
    let data = page.content_start();
    let bytes = pages.bytes(root)?.to_vec();
    pages.put(child, data, bytes.get(data..usable).ok_or(Error::Overrun)?);
    pages.put(
        child,
        0,
        bytes
            .get(header..header.saturating_add(array_end))
            .ok_or(Error::Overrun)?,
    );
    let mut above = pages.writer(root)?;
    above.zero(kind.interior());
    above.point(child)?;
    pages.point(child, Point::Branch, root)?;
    pages.point_cells(child)?;
    Ok(child)
}

/// A full right-most leaf grown a sibling beside it, which is
/// `balance_quick`: the cell that did not fit is the whole of the new
/// page, and the parent gains a divider naming the page that was full
/// and the largest key on it.
///
/// Answers whether the sibling was written. The parent gains the
/// divider, so a parent that will not hold one leaves the leaf to the
/// balance proper, which the C library reaches by letting the parent
/// overflow and balancing it in the next turn of its own loop. The room
/// is counted before a page is taken, so a leaf this answers `false`
/// for leaves the file as it found it.
pub(crate) fn quick(pages: &mut Pages, parent: u32, page: u32, cell: &[u8]) -> Result<bool, Error> {
    let last = pages
        .page(page)?
        .cells()
        .checked_sub(1)
        .ok_or(Error::Balance)?;
    let Cell::TableLeaf { rowid, .. } = pages.page(page)?.cell(last)? else {
        return Err(Error::Balance);
    };
    let divider = write_cell(&Cell::TableInterior { child: page, rowid });
    let room = crate::page::cell_room(divider.len()).saturating_add(2);
    if pages.page(parent)?.free()? < room {
        return Ok(false);
    }
    let sibling = pages.add(Kind::LeafTable, 0)?;
    if !pages.put_cell(sibling, 0, cell)? {
        return Err(Error::Balance);
    }
    let cells = pages.page(parent)?.cells();
    // The room the divider takes was counted above, so the parent holds
    // it and the answer says so.
    pages.put_cell(parent, cells, &divider)?;
    pages.writer(parent)?.point(sibling)?;
    pages.point(sibling, Point::Branch, parent)?;
    pages.point_cells(sibling)?;
    Ok(true)
}

/// Writes `bytes` over the payload of the row `rowid` from `at`, which
/// is `sqlite3BtreePutData` of `research/sqlite/src/btree.c:11090`: the
/// bytes the cell holds on its own page and then the pages of its
/// chain, with nothing moved and no page freed, because the payload
/// keeps the length it had.
///
/// The descent to the row is O(log n) in the rows and the write is O(n)
/// in the bytes it writes.
///
/// # Errors
///
/// [`Error::Overrun`] where the cell the descent reaches carries no
/// payload, and whatever reading or writing a page refuses.
pub fn put_payload(
    pages: &mut Pages,
    root: u32,
    rowid: i64,
    at: usize,
    bytes: &[u8],
) -> Result<(), Error> {
    let path = place(pages, root, rowid)?;
    let (leaf, index) = *path.last().ok_or(Error::Depth)?;
    // The caller read the row out of the same tree, so the descent
    // reaches the cell that holds it.
    let (start, local, chain) = {
        let page = pages.page(leaf)?;
        page.payload_place(index)?.ok_or(Error::Overrun)?
    };
    let mut written = 0;
    let mut skipped = at;
    if skipped < local {
        let take = local.saturating_sub(skipped).min(bytes.len());
        pages.put(
            leaf,
            start.saturating_add(skipped),
            bytes.get(..take).unwrap_or_default(),
        );
        written = take;
        skipped = 0;
    } else {
        skipped = skipped.saturating_sub(local);
    }
    // Every page of the chain carries the number of the next one and
    // then the bytes of the payload. The payload was read before the
    // write, so its chain holds every byte the write reaches and the
    // walk ends where the bytes do.
    let held = pages.usable.saturating_sub(4);
    let mut number = chain.unwrap_or(0);
    while written < bytes.len() {
        let after = pages.word(number, 0)?;
        if skipped >= held {
            skipped = skipped.saturating_sub(held);
            number = after;
            continue;
        }
        let take = held
            .saturating_sub(skipped)
            .min(bytes.len().saturating_sub(written));
        pages.put(
            number,
            skipped.saturating_add(4),
            bytes
                .get(written..written.saturating_add(take))
                .unwrap_or_default(),
        );
        written = written.saturating_add(take);
        skipped = 0;
        number = after;
    }
    Ok(())
}

/// Whether the table tree at `root` holds a row under `rowid`, which is
/// what `sqlite3BtreeTableMoveto` answers for the key of a row about to
/// be written. One walk is O(log n) in the rows.
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, and
/// whatever reading a page of it refuses.
pub fn holds(pages: &Pages, root: u32, rowid: i64) -> Result<bool, Error> {
    let path = place(pages, root, rowid)?;
    let (leaf, at) = *path.last().ok_or(Error::Depth)?;
    let page = pages.page(leaf)?;
    if at >= page.cells() {
        return Ok(false);
    }
    Ok(page.row(at)?.0 == rowid)
}

/// The largest key the table tree at `root` holds, or nothing where the
/// tree holds no row.
///
/// The walk follows the right-most pointer of every page, which is
/// O(log n).
///
/// # Errors
///
/// [`Error::Depth`] for a tree deeper than this crate walks, and
/// whatever reading a page of it refuses.
pub fn largest(pages: &Pages, root: u32) -> Result<Option<i64>, Error> {
    let mut number = root;
    for _ in 0..MAX_DEPTH {
        let page = pages.page(number)?;
        if !page.kind().is_interior() {
            let Some(last) = page.cells().checked_sub(1) else {
                return Ok(None);
            };
            return page.row(last).map(|(rowid, _)| Some(rowid));
        }
        number = page.right_most().ok_or(Error::Overrun)?;
    }
    Err(Error::Depth)
}

/// The pages the descent to a key passes through, root first, each with
/// where the key belongs on it: the same descent a reader makes, and
/// then the first cell of the leaf whose key is above it.
fn place(pages: &Pages, root: u32, rowid: i64) -> Result<Vec<(u32, usize)>, Error> {
    let mut path = Vec::new();
    let mut number = root;
    for _ in 0..MAX_DEPTH {
        let page = pages.page(number)?;
        let cells = page.cells();
        let mut at = cells;
        for index in 0..cells {
            let above = match page.cell(index)? {
                Cell::TableInterior { rowid: key, .. } | Cell::TableLeaf { rowid: key, .. } => {
                    key >= rowid
                }
                _ => return Err(Error::PageKind(page.kind().byte())),
            };
            if above {
                at = index;
                break;
            }
        }
        path.push((number, at));
        if !page.kind().is_interior() {
            return Ok(path);
        }
        number = if at == cells {
            page.right_most().ok_or(Error::Overrun)?
        } else {
            page.child(at)?
        };
    }
    Err(Error::Depth)
}

/// One cell of the pages being balanced: its bytes, and where it lies
/// now, which is nothing for a divider taken off the parent and for the
/// cell that did not fit.
struct Held {
    /// The bytes of the cell.
    bytes: Vec<u8>,
    /// The page it lies on, and where on it.
    from: Option<(u32, usize)>,
}

/// The most pages a balance writes, which is `NB+2`.
const MOST: usize = 5;

/// Five numbers, one for every page a balance may write, read and
/// written by the place of the page among them.
#[derive(Clone, Copy, Default)]
struct Row<T: Copy + Default>([T; MOST]);

impl<T: Copy + Default> Row<T> {
    /// The number at `at`, which is nought past the end.
    fn get(&self, at: usize) -> T {
        self.0.get(at).copied().unwrap_or_default()
    }

    /// Writes `value` at `at`, which writes nothing past the end.
    fn set(&mut self, at: usize, value: T) {
        for slot in self.0.iter_mut().skip(at).take(1) {
            *slot = value;
        }
    }
}

/// The siblings a balance writes again.
struct Siblings {
    /// The pages, left to right.
    old: Vec<u32>,
    /// The dividers that were between them, in the same order, taken off
    /// the parent.
    dividers: Vec<Vec<u8>>,
    /// Where among the cells of the parent the first divider was.
    next: usize,
    /// Which of the siblings would not hold the cell.
    full: usize,
    /// Where the pointer the last sibling hangs from is, which is the
    /// parent's own where it equals the parent's cell count.
    right_at: usize,
}

/// The siblings of the child at `at`: one on either side of it where the
/// parent has one, and two from one side where the child is the first or
/// the last. The dividers between them come off the parent.
///
/// # Errors
///
/// [`Error::Overrun`] for a parent with no right-most pointer, and
/// whatever reading or writing the parent refuses.
/// The divider at `index` of a parent that holds at most one cell it
/// has no room for: its bytes, the page under it, and where it lies
/// among the cells of the page, which is nothing for the cell the
/// parent has no room for.
///
/// The dividers are read from the last to the first, so a divider after
/// the one the parent has no room for lies one place earlier on the
/// page than its index says.
fn divider_at(
    pages: &Pages,
    parent: u32,
    index: usize,
    above: &[Spill],
) -> Result<(Vec<u8>, u32, Option<usize>), Error> {
    let over = above.first().map(|(place, _)| *place);
    if over == Some(index) {
        let (_, bytes) = above.first().ok_or(Error::Balance)?;
        let child = crate::bytes::u32_at(bytes, 0).ok_or(Error::Overrun)?;
        return Ok((bytes.clone(), child, None));
    }
    let on = index.saturating_sub(usize::from(over.is_some_and(|place| index > place)));
    let page = pages.page(parent)?;
    Ok((write_cell(&page.cell(on)?), page.child(on)?, Some(on)))
}

fn siblings(pages: &mut Pages, parent: u32, at: usize, above: &[Spill]) -> Result<Siblings, Error> {
    let held_by_parent = pages.page(parent)?.cells().saturating_add(above.len());
    let (mut i, next) = if held_by_parent < 2 {
        (held_by_parent, 0)
    } else if at == 0 {
        (2, 0)
    } else if at == held_by_parent {
        (2, held_by_parent.saturating_sub(2))
    } else {
        (2, at.saturating_sub(1))
    };
    let right_at = i.saturating_add(next);
    let mut number = if right_at == held_by_parent {
        pages.page(parent)?.right_most().ok_or(Error::Overrun)?
    } else {
        divider_at(pages, parent, right_at, above)?.1
    };
    // The siblings are read right to left, because the divider that
    // names one of them lies to its left.
    let mut old: Vec<u32> = Vec::new();
    let mut dividers: Vec<Vec<u8>> = Vec::new();
    loop {
        old.push(number);
        if i == 0 {
            break;
        }
        i = i.saturating_sub(1);
        let index = i.saturating_add(next);
        let (bytes, child, on) = divider_at(pages, parent, index, above)?;
        dividers.push(bytes);
        number = child;
        // A divider the parent has no room for lies on no page, so the
        // balance reads it and drops nothing.
        if let Some(on) = on {
            pages.writer(parent)?.remove(on)?;
        }
    }
    old.reverse();
    dividers.reverse();
    Ok(Siblings {
        old,
        dividers,
        next,
        full: at.saturating_sub(next),
        right_at,
    })
}

/// The cells of the siblings in key order, with the dividers between
/// them where a divider is a cell of its own, and where each sibling
/// ends among them.
///
/// # Errors
///
/// [`Error::Balance`] where a sibling is not of the kind the first one
/// is, and whatever reading a sibling refuses.
fn gather(
    pages: &Pages,
    taken: &Siblings,
    spill: &[Spill],
    kind: Kind,
    leaf_data: bool,
) -> Result<(Vec<Held>, Row<usize>), Error> {
    let last = taken.old.len().saturating_sub(1);
    let mut cells: Vec<Held> = Vec::new();
    let mut old_at: Row<usize> = Row::default();
    for (index, number) in taken.old.iter().copied().enumerate() {
        let page = pages.page(number)?;
        if page.kind() != kind {
            return Err(Error::Balance);
        }
        let mut own: Vec<Held> = Vec::new();
        for slot in 0..page.cells() {
            own.push(Held {
                bytes: write_cell(&page.cell(slot)?),
                from: Some((number, page.cell_offset(slot)?)),
            });
        }
        // The cells a page holds that lie on no page come before the
        // cell they were put in front of.
        let spilled = if index == taken.full { spill } else { &[] };
        let first = spilled
            .first()
            .map_or(own.len(), |(at, _)| (*at).min(own.len()));
        own.splice(
            first..first,
            spilled.iter().map(|(_, bytes)| Held {
                bytes: bytes.clone(),
                from: None,
            }),
        );
        cells.append(&mut own);
        old_at.set(index, cells.len());
        if index < last && !leaf_data {
            let mut bytes = taken.dividers.get(index).cloned().unwrap_or_default();
            match page.right_most() {
                // The right pointer of the page under the divider
                // becomes the left pointer of the divider itself.
                Some(child) => {
                    for (slot, byte) in bytes.iter_mut().zip(child.to_be_bytes()) {
                        *slot = byte;
                    }
                }
                // A leaf has no page under it, so the divider joins the
                // cells of the leaves without the four bytes that named
                // one, which is the `leafCorrection` of the C library.
                None => bytes.drain(..4.min(bytes.len())).for_each(drop),
            }
            cells.push(Held { bytes, from: None });
        }
    }
    Ok((cells, old_at))
}

/// What the cell at `at` costs the page it lies on: its bytes and the
/// two of its pointer, and nought where the cells end before it.
fn cost_of(cells: &[Held], at: usize) -> i64 {
    cells.get(at).map_or(0, |cell| {
        count_of(crate::page::cell_room(cell.bytes.len())).saturating_add(2)
    })
}

/// How many pages the cells take and where each of them ends among the
/// cells, which is the packing loop and the evening out after it.
fn packing(
    cells: &[Held],
    mut sizes: Row<i64>,
    mut counts: Row<usize>,
    old_count: usize,
    room: i64,
    leaf_data: bool,
    bulk: bool,
) -> (usize, Row<usize>) {
    let mut k = old_count;
    let mut index = 0;
    while index < k {
        let right = index.saturating_add(1);
        while sizes.get(index) > room {
            if right >= k {
                k = index.saturating_add(2);
                sizes.set(k.saturating_sub(1), 0);
                counts.set(k.saturating_sub(1), cells.len());
            }
            let moved = cost_of(cells, counts.get(index).saturating_sub(1));
            sizes.set(index, sizes.get(index).saturating_sub(moved));
            let after = if leaf_data {
                moved
            } else {
                cost_of(cells, counts.get(index))
            };
            sizes.set(right, sizes.get(right).saturating_add(after));
            counts.set(index, counts.get(index).saturating_sub(1));
        }
        while counts.get(index) < cells.len() {
            let taken = cost_of(cells, counts.get(index));
            if sizes.get(index).saturating_add(taken) > room {
                break;
            }
            sizes.set(index, sizes.get(index).saturating_add(taken));
            counts.set(index, counts.get(index).saturating_add(1));
            let after = if leaf_data {
                taken
            } else {
                cost_of(cells, counts.get(index))
            };
            sizes.set(right, sizes.get(right).saturating_sub(after));
        }
        if counts.get(index) >= cells.len() {
            k = index.saturating_add(1);
        }
        index = index.saturating_add(1);
    }
    // The packing above fills the left pages and may leave the right one
    // nearly empty, which is not only slow but may be no page at all, so
    // the cells move back until the two sides are even.
    for index in (1..k).rev() {
        let left_at = index.saturating_sub(1);
        let mut right = sizes.get(index);
        let mut left = sizes.get(left_at);
        for r in (0..counts.get(left_at)).rev() {
            let d = r.saturating_add(1).saturating_sub(usize::from(leaf_data));
            let size_r = cost_of(cells, r).saturating_sub(2);
            let size_d = cost_of(cells, d).saturating_sub(2);
            let keeps = if index == k.saturating_sub(1) { 0 } else { 2 };
            // A bulk load fills the left page and leaves the right one
            // to the cells still to come, because they all run after
            // what is there: `BTREE_BULKLOAD` is what a `CREATE INDEX`
            // asks for.
            if right != 0
                && (bulk
                    || right.saturating_add(size_d).saturating_add(2)
                        > left.saturating_sub(size_r.saturating_add(keeps)))
            {
                break;
            }
            right = right.saturating_add(size_d).saturating_add(2);
            left = left.saturating_sub(size_r.saturating_add(2));
            counts.set(left_at, r);
        }
        sizes.set(index, right);
        sizes.set(left_at, left);
    }
    (k, counts)
}

/// The pages a balance writes: the siblings it read again, and one more
/// for every page the cells now take, in the order their numbers run.
///
/// A page that changes place in the file takes what is on it with it, so
/// where every cell of the two lies changes as well.
///
/// # Errors
///
/// Whatever adding a page or moving one refuses.
fn allocate(
    pages: &mut Pages,
    taken: &Siblings,
    cells: &mut [Held],
    old_at: &mut Row<usize>,
    k: usize,
    kind: Kind,
) -> Result<Vec<u32>, Error> {
    let mut new: Vec<u32> = Vec::new();
    // A page the balance adds is taken near the one before it, and the
    // first near the left-most of the siblings.
    let mut near = taken.old.first().copied().unwrap_or(0);
    for index in 0..k {
        let number = if let Some(number) = taken.old.get(index) {
            // `sqlite3PagerWrite`: a sibling the balance writes again is
            // opened here, which is where the journal takes it.
            pages.open(*number);
            *number
        } else {
            old_at.set(index, cells.len());
            near = pages.add(kind, near)?;
            near
        };
        new.push(number);
    }
    for index in 0..k.saturating_sub(1) {
        let mut least = index;
        for other in index.saturating_add(1)..k {
            if number_at(&new, other) < number_at(&new, least) {
                least = other;
            }
        }
        if least != index {
            let (one, other) = (number_at(&new, index), number_at(&new, least));
            pages.swap(one, other)?;
            for cell in cells.iter_mut() {
                if let Some((number, _)) = &mut cell.from {
                    if *number == one {
                        *number = other;
                    } else if *number == other {
                        *number = one;
                    }
                }
            }
            new.swap(index, least);
        }
    }
    Ok(new)
}

/// The page number at `index` among the pages a balance writes.
fn number_at(new: &[u32], index: usize) -> u32 {
    new.get(index).copied().unwrap_or(0)
}

/// The dividers the parent needs after a balance, one between each two
/// pages, with the right pointer of each page written as it goes.
///
/// # Errors
///
/// [`Error::Balance`] where a divider names no key, and whatever writing
/// a page refuses.
fn dividers_for(
    pages: &mut Pages,
    cells: &[Held],
    counts: Row<usize>,
    new: &[u32],
    next: usize,
    leaf_data: bool,
    leaves: bool,
) -> Result<Vec<Spill>, Error> {
    let mut built: Vec<Spill> = Vec::new();
    for index in 0..new.len().saturating_sub(1) {
        let j = counts.get(index);
        let page = number_at(new, index);
        let divider = if leaf_data {
            let held = cells.get(j.saturating_sub(1)).ok_or(Error::Balance)?;
            let rowid = key_of(&held.bytes)?;
            write_cell(&Cell::TableInterior { child: page, rowid })
        } else if leaves {
            // The divider of a tree that holds a key on every page is
            // the entry itself, which no leaf keeps a copy of. A cell
            // that came off a leaf carries no pointer, so the parent's
            // copy gains the four bytes `leafCorrection` counts.
            let held = cells.get(j).ok_or(Error::Balance)?;
            let mut bytes = page.to_be_bytes().to_vec();
            bytes.extend_from_slice(&held.bytes);
            bytes
        } else {
            let held = cells.get(j).ok_or(Error::Balance)?;
            let child = u32::from_be_bytes(
                held.bytes
                    .get(..4)
                    .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                    .ok_or(Error::Balance)?,
            );
            pages.writer(page)?.point(child)?;
            let mut bytes = held.bytes.clone();
            for (slot, byte) in bytes.iter_mut().zip(page.to_be_bytes()) {
                *slot = byte;
            }
            bytes
        };
        built.push((next.saturating_add(index), divider));
    }
    Ok(built)
}

/// What a balance decided, which is what writing its pages needs.
#[derive(Clone, Copy)]
struct Plan<'a> {
    /// Where each page the balance writes ends among the cells.
    counts: Row<usize>,
    /// Where each sibling ended among them before the balance.
    old_at: Row<usize>,
    /// The pages the balance writes, in the order their numbers run.
    new: &'a [u32],
    /// The cells the full sibling holds that lie on no page.
    spill: &'a [Spill],
    /// Which of the pages that sibling is.
    full: usize,
    /// How many siblings the balance read.
    old_count: usize,
    /// Whether a divider between two of the pages is one of the cells.
    leaf_data: bool,
}

/// The pages of a balance written, each of them once: a page that gives
/// cells to its left is written after that page, and a page that gives
/// them to its right after the page on its right, so the pass goes down
/// and then up.
///
/// # Errors
///
/// Whatever writing one of the pages refuses.
fn write_pages(pages: &mut Pages, cells: &[Held], plan: &Plan<'_>) -> Result<(), Error> {
    let Plan {
        counts,
        old_at,
        new,
        spill,
        full,
        old_count,
        leaf_data,
    } = *plan;
    let k = new.len();
    let mut done = [false; MOST];
    let sources: Vec<Source<'_>> = cells
        .iter()
        .map(|cell| Source {
            bytes: &cell.bytes,
            from: cell.from,
        })
        .collect();
    let holds: Vec<usize> = spill.iter().map(|(at, _)| *at).collect();
    for step in 0..k.saturating_mul(2).saturating_sub(1) {
        let index = if step < k.saturating_sub(1) {
            k.saturating_sub(1).saturating_sub(step)
        } else {
            step.saturating_sub(k.saturating_sub(1))
        };
        let upwards = step >= k.saturating_sub(1);
        if done.get(index).copied().unwrap_or(false) {
            continue;
        }
        let left_at = index.saturating_sub(1);
        if !upwards && old_at.get(left_at) < counts.get(left_at) {
            continue;
        }
        let (old_first, new_first, count) = if index == 0 {
            (0, 0, counts.get(0))
        } else {
            let old_first = if index < old_count {
                old_at.get(left_at).saturating_add(usize::from(!leaf_data))
            } else {
                cells.len()
            };
            let new_first = counts.get(left_at).saturating_add(usize::from(!leaf_data));
            (
                old_first,
                new_first,
                counts.get(index).saturating_sub(new_first),
            )
        };
        let held = if index == full { holds.as_slice() } else { &[] };
        pages
            .writer(number_at(new, index))?
            .edit(old_first, new_first, count, &sources, held)?;
        for slot in done.iter_mut().skip(index).take(1) {
            *slot = true;
        }
    }
    Ok(())
}

/// The page that would not hold a cell, up to one sibling on either side
/// of it, and the dividers between them, written again so that every
/// cell fits and the parent names them, which is `balance_nonroot`.
///
/// `topmost` says the parent is the root of its tree, which is the one
/// parent that may be left with no cell at all and then takes the
/// content of its only child.
///
/// Answers the dividers the parent would not hold, which the caller
/// balances the parent for.
///
/// # Errors
///
/// [`Error::Balance`] where a sibling is not of the kind the first one
/// is, and whatever reading or writing one of the pages refuses.
fn balance_nonroot(
    pages: &mut Pages,
    parent: u32,
    at: usize,
    spill: &[Spill],
    above: &[Spill],
    topmost: bool,
    bulk: bool,
) -> Result<Vec<Spill>, Error> {
    let usable = pages.usable();
    let held_by_parent = pages.page(parent)?.cells().saturating_add(above.len());
    let taken = siblings(pages, parent, at, above)?;
    let old_count = taken.old.len();
    let kind = pages
        .page(*taken.old.first().ok_or(Error::Balance)?)?
        .kind();
    let leaf = !kind.is_interior();
    // A tree of rows keeps its keys in the dividers and its rows in the
    // leaves, so a divider between two leaves is not one of the cells.
    let leaf_data = kind == Kind::LeafTable;
    let (mut cells, mut old_at) = gather(pages, &taken, spill, kind, leaf_data)?;
    // The space a page takes is counted as a signed number, because the
    // packing hands a cell from one page to the next before it knows the
    // next page holds it, which leaves that page owing space.
    let room = count_of(
        usable
            .saturating_sub(12)
            .saturating_add(usize::from(leaf).saturating_mul(4)),
    );
    let mut sizes: Row<i64> = Row::default();
    let mut counts: Row<usize> = Row::default();
    for (index, number) in taken.old.iter().copied().enumerate() {
        let mut used = room.saturating_sub(count_of(pages.page(number)?.free()?));
        if index == taken.full {
            for (_, bytes) in spill {
                used = used.saturating_add(
                    count_of(crate::page::cell_room(bytes.len())).saturating_add(2),
                );
            }
        }
        sizes.set(index, used);
        counts.set(index, old_at.get(index));
    }
    let (k, counts) = packing(&cells, sizes, counts, old_count, room, leaf_data, bulk);
    let new = allocate(pages, &taken, &mut cells, &mut old_at, k, kind)?;
    let last = number_at(&new, k.saturating_sub(1));
    // The pointer the last of the pages hangs from, which is written
    // before the dividers are, because inserting a cell moves no other
    // cell's bytes.
    if taken.right_at == held_by_parent {
        pages.writer(parent)?.point(last)?;
    } else {
        pages.writer(parent)?.point_child(taken.next, last)?;
    }
    // The right pointer of the last of the old pages is the right
    // pointer of the last of the new ones. Where the balance writes
    // more pages than it read, that page is one of the new ones, whose
    // number the sort above may have changed; where it writes fewer, it
    // is one of the pages the balance is about to free.
    if !leaf && old_count != k {
        let from = if k > old_count {
            number_at(&new, old_count.saturating_sub(1))
        } else {
            *taken
                .old
                .get(old_count.saturating_sub(1))
                .ok_or(Error::Balance)?
        };
        let child = pages.page(from)?.right_most().ok_or(Error::Balance)?;
        pages.writer(last)?.point(child)?;
    }
    let built = dividers_for(pages, &cells, counts, &new, taken.next, leaf_data, leaf)?;
    // A parent that holds one cell it has no room for holds every cell
    // after that one the same way, which is `insertCell`, and which is
    // why the dividers of a balance lie next to each other.
    let mut out: Vec<Spill> = Vec::new();
    let mut rest = built.into_iter();
    for (place, divider) in rest.by_ref() {
        if !pages.put_cell(parent, place, &divider)? {
            out.push((place, divider));
            break;
        }
    }
    out.extend(rest);
    let plan = Plan {
        counts,
        old_at,
        new: &new,
        spill,
        full: taken.full,
        old_count,
        leaf_data,
    };
    write_pages(pages, &cells, &plan)?;
    // Every page the balance wrote names other pages again, so the
    // pointer maps are written again for the parent and for each of the
    // siblings.
    pages.point_cells(parent)?;
    for number in &new {
        pages.point_cells(*number)?;
    }
    // A root left with no cell at all takes the content of its only
    // child, which is the balance that makes a tree one level shorter.
    // A root on page one begins a hundred bytes in, so it takes the
    // child only where the child has that many bytes free; the tree
    // stays a level taller where it has fewer.
    let first = number_at(&new, 0);
    let header =
        usize::from(parent == crate::image::SCHEMA_ROOT).saturating_mul(crate::header::HEADER_LEN);
    if topmost && pages.page(parent)?.cells() == 0 && pages.page(first)?.free()? >= header {
        shallower(pages, parent, first)?;
    }
    // The pages the balance no longer needs go on the free list.
    for number in taken.old.iter().copied().skip(k) {
        pages.release(number)?;
    }
    Ok(out)
}

/// A root that holds no cell given the content of its only child, which
/// is `balance-shallower`: the child is moved together first, because a
/// root on page one has a hundred bytes fewer and the space has to be in
/// one piece at the front.
///
/// # Errors
///
/// Whatever moving the child together or freeing it refuses.
fn shallower(pages: &mut Pages, root: u32, child: u32) -> Result<(), Error> {
    pages.writer(child)?.defragment(None)?;
    let usable = pages.usable();
    let page = pages.page(child)?;
    let kind = page.kind();
    let array_end = kind
        .header_len()
        .saturating_add(page.cells().saturating_mul(2));
    let data = page.content_start();
    let bytes = pages.bytes(child)?.to_vec();
    let header =
        usize::from(root == crate::image::SCHEMA_ROOT).saturating_mul(crate::header::HEADER_LEN);
    pages.put(root, header, bytes.get(..array_end).ok_or(Error::Overrun)?);
    pages.put(root, data, bytes.get(data..usable).ok_or(Error::Overrun)?);
    pages.point_cells(root)?;
    pages.release(child)
}

/// A count of bytes as the signed number the packing loops keep.
fn count_of(bytes: usize) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

/// The key of a row, which is the varint after the length of its
/// payload.
fn key_of(cell: &[u8]) -> Result<i64, Error> {
    let (_, read) = crate::bytes::varint(cell)?;
    let (key, _) = crate::bytes::varint(cell.get(read..).unwrap_or_default())?;
    Ok(crate::bytes::signed(key))
}
