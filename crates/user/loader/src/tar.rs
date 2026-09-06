// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The ustar reader: the archive of the boot image, as bytes somebody else
//! owns.
//!
//! A ustar archive is a sequence of 512-byte headers, each followed by the
//! bytes of its file padded up to the next multiple of 512, and ended by
//! two blocks of zeros. Everything the format says about a file stands in
//! its header as text: the size in octal, the checksum in octal, and the
//! name in two fields that are joined with a slash between them.
//!
//! The reader is strict, because the archive is input the root task did not
//! write. A name that begins with a slash or that walks upwards through
//! `..` names something outside the archive and is refused where it is
//! read, not where it is used. A header whose checksum does not match is
//! refused. What follows the end marker is not read at all.
//!
//! Invariants: every entry the reader hands out lies inside the bytes it
//! was given; the walk advances by at least one block per step, so it ends
//! on every input; an entry that was refused ends the walk rather than
//! being skipped, because the next header is found by the size field of
//! this one and a header that does not parse says nothing about where the
//! next one is.

/// The size of a header and of every block of the archive.
pub const BLOCK: usize = 512;

/// Offset of the name field.
pub(crate) const NAME: usize = 0;

/// Length of the name field.
const NAME_LEN: usize = 100;

/// Offset of the size field.
pub(crate) const SIZE: usize = 124;

/// Length of the size field.
const SIZE_LEN: usize = 12;

/// Offset of the checksum field.
pub(crate) const CHECKSUM: usize = 148;

/// Length of the checksum field.
const CHECKSUM_LEN: usize = 8;

/// Offset of the type flag.
const TYPE_FLAG: usize = 156;

/// Offset of the magic.
pub(crate) const MAGIC: usize = 257;

/// The magic of a ustar archive.
const USTAR: [u8; 6] = *b"ustar\0";

/// Offset of the prefix field.
pub(crate) const PREFIX: usize = 345;

/// Length of the prefix field.
const PREFIX_LEN: usize = 155;

/// How many bytes a joined name has at most: both fields and the slash
/// between them.
pub const MAX_PATH: usize = PREFIX_LEN + 1 + NAME_LEN;

/// Why an archive could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TarError {
    /// The bytes end inside a header or inside the file it describes.
    Truncated {
        /// Where the reader stood.
        at: usize,
    },
    /// The magic at offset 257 is not `ustar\0`.
    BadMagic,
    /// The checksum in the header is not the sum of the header.
    BadChecksum {
        /// What the header claims.
        found: u32,
        /// What the bytes add up to.
        computed: u32,
    },
    /// A field that should be octal digits is not.
    BadOctal {
        /// Where the field starts in the header.
        at: usize,
    },
    /// A name that begins with a slash.
    AbsolutePath,
    /// A name with a `..` component.
    ParentComponent,
    /// A header whose name field is empty.
    EmptyName,
    /// A name field with bytes behind its terminating zero.
    NameNotTerminated {
        /// Where the field starts in the header.
        at: usize,
    },
}

impl core::fmt::Display for TarError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TarError::Truncated { at } => write!(f, "the archive ends at {at}"),
            TarError::BadMagic => f.write_str("the header is no ustar header"),
            TarError::BadChecksum { found, computed } => {
                write!(
                    f,
                    "the checksum is {found}, the header adds up to {computed}"
                )
            }
            TarError::BadOctal { at } => write!(f, "the field at {at} is not octal"),
            TarError::AbsolutePath => f.write_str("an absolute path"),
            TarError::ParentComponent => f.write_str("a path with a `..` component"),
            TarError::EmptyName => f.write_str("a header without a name"),
            TarError::NameNotTerminated { at } => {
                write!(f, "the name at {at} has bytes behind its zero")
            }
        }
    }
}

/// What an entry of the archive is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A regular file. Type flag `0` or a zero byte, which older writers
    /// used for the same thing.
    File,
    /// A directory. Type flag `5`.
    Directory,
    /// Something this system has no use for: a link, a device, a name that
    /// belongs to the next entry. The reader hands it out rather than
    /// refusing it, and whoever walks the archive passes over it.
    Other(u8),
}

/// A name, joined out of the prefix and the name field.
#[derive(Clone, Copy, Debug)]
pub struct Path {
    bytes: [u8; MAX_PATH],
    len: usize,
}

impl Path {
    /// The bytes of the name.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// How many bytes the name has.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` for a name of no bytes, which no entry has.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Path {}

/// One entry of the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// The name, prefix and name field joined.
    pub path: Path,
    /// What the entry is.
    pub kind: Kind,
    /// The bytes of the file, without the padding behind them.
    pub data: &'a [u8],
}

/// A ustar archive over borrowed bytes.
#[derive(Clone, Copy, Debug)]
pub struct Archive<'a> {
    bytes: &'a [u8],
}

impl<'a> Archive<'a> {
    /// The archive in `bytes`. Nothing is read yet; a header is checked
    /// when the walk reaches it.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Archive { bytes }
    }

    /// The bytes behind the archive.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The entries, in the order they stand in.
    ///
    /// The walk ends at the end marker, at the end of the bytes, or at the
    /// first entry that does not read — and in that last case the error is
    /// the last item the iterator hands out.
    #[must_use]
    pub const fn entries(&self) -> Entries<'a> {
        Entries {
            bytes: self.bytes,
            at: 0,
            done: false,
        }
    }

    /// The first entry whose name is `path`, if the archive holds one.
    ///
    /// # Errors
    ///
    /// The first [`TarError`] the walk meets, which ends it.
    pub fn find(&self, path: &[u8]) -> Result<Option<Entry<'a>>, TarError> {
        for entry in self.entries() {
            let entry = entry?;
            if entry.path.as_bytes() == path {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }
}

/// The walk over the entries of an archive.
///
/// It is not `Copy`: a walk that could be duplicated by being read would
/// be one where a caller loses its place by accident.
#[derive(Clone, Debug)]
pub struct Entries<'a> {
    bytes: &'a [u8],
    at: usize,
    done: bool,
}

impl<'a> Iterator for Entries<'a> {
    type Item = Result<Entry<'a>, TarError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(entry) => {
                if entry.is_none() {
                    self.done = true;
                }
                entry.map(Ok)
            }
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

impl<'a> Entries<'a> {
    /// Reads the header at the cursor and moves the cursor behind the file
    /// it describes. `None` at the end marker and at the end of the bytes.
    fn step(&mut self) -> Result<Option<Entry<'a>>, TarError> {
        let Some(header) = block(self.bytes, self.at) else {
            // Fewer than 512 bytes left. An archive whose last block is
            // partial is over; one that stopped inside a header is
            // truncated, and there is no way to tell the two apart, so the
            // reader takes the reading that ends the walk.
            return Ok(None);
        };
        if is_zero(header) {
            return Ok(None);
        }
        check_magic(header)?;
        check_checksum(header)?;
        let path = path_of(header)?;
        let size = octal(header, SIZE, SIZE_LEN)?;
        // A size that does not fit the index type of the machine cannot fit
        // the archive either, so the largest index stands for it and the
        // bound below refuses it as a truncation.
        let len = usize::try_from(size).unwrap_or(usize::MAX);

        // The cursor is inside the bytes, so a block behind it cannot
        // overflow; the file behind that can.
        let start = self.at.saturating_add(BLOCK);
        let end = start
            .checked_add(len)
            .ok_or(TarError::Truncated { at: start })?;
        let data = self
            .bytes
            .get(start..end)
            .ok_or(TarError::Truncated { at: start })?;

        // The next header stands at the next multiple of the block size,
        // which is what pads a file whose length is not one.
        let padded = len.next_multiple_of(BLOCK);
        self.at = start
            .checked_add(padded)
            .ok_or(TarError::Truncated { at: start })?;
        Ok(Some(Entry {
            path,
            kind: kind_of(header),
            data,
        }))
    }
}

/// The block of `bytes` at `at`, or `None` when fewer than a block is left.
fn block(bytes: &[u8], at: usize) -> Option<&[u8; BLOCK]> {
    let end = at.checked_add(BLOCK)?;
    bytes.get(at..end)?.first_chunk::<BLOCK>()
}

/// `true` for a block of nothing but zeros, which is what the end marker is
/// made of.
fn is_zero(header: &[u8; BLOCK]) -> bool {
    header.iter().all(|byte| *byte == 0)
}

/// Refuses a header whose magic is not `ustar\0`.
fn check_magic(header: &[u8; BLOCK]) -> Result<(), TarError> {
    let end = MAGIC.wrapping_add(USTAR.len());
    if header.get(MAGIC..end) == Some(USTAR.as_slice()) {
        Ok(())
    } else {
        Err(TarError::BadMagic)
    }
}

/// Refuses a header whose checksum field does not match its bytes.
///
/// The field itself counts as eight spaces, which is how the checksum can
/// stand in the block it covers.
fn check_checksum(header: &[u8; BLOCK]) -> Result<(), TarError> {
    let found = octal(header, CHECKSUM, CHECKSUM_LEN)?;
    let end = CHECKSUM.wrapping_add(CHECKSUM_LEN);
    let computed = header
        .iter()
        .enumerate()
        .fold(0u32, |total, (index, byte)| {
            let counted = if (CHECKSUM..end).contains(&index) {
                u32::from(b' ')
            } else {
                u32::from(*byte)
            };
            total.wrapping_add(counted)
        });
    let found = u32::try_from(found).unwrap_or(u32::MAX);
    if found == computed {
        Ok(())
    } else {
        Err(TarError::BadChecksum { found, computed })
    }
}

/// The `len` bytes of `header` at `at`. Every caller passes a field of the
/// header, whose bounds are constants of the format, so the empty slice is
/// a value this never returns.
fn slice(header: &[u8; BLOCK], at: usize, len: usize) -> &[u8] {
    let end = at.saturating_add(len);
    header.get(at..end).unwrap_or(&[])
}

/// Reads the octal number in the field at `at`.
///
/// A field of this format is octal digits, then a zero or a space, then
/// padding that is not read. A field of nothing but padding is zero, which
/// is what a size field of a directory looks like.
fn octal(header: &[u8; BLOCK], at: usize, len: usize) -> Result<u64, TarError> {
    // Every caller passes a field of the header, so the range lies inside
    // the block; an empty slice would be no octal field and is refused
    // below like any other.
    let field = slice(header, at, len);
    let digits = field
        .iter()
        .copied()
        .skip_while(|byte| *byte == b' ')
        .take_while(|byte| (b'0'..=b'7').contains(byte));
    // A field of this format is at most twelve characters, so at most
    // twelve octal digits, which is thirty-six bits: the saturating forms
    // here cannot saturate.
    let mut value = 0u64;
    let mut seen = false;
    for digit in digits {
        seen = true;
        value = value
            .saturating_mul(8)
            .saturating_add(u64::from(digit.wrapping_sub(b'0')));
    }
    if seen {
        return Ok(value);
    }
    // No digits at all: a field of zeros, spaces, or both is zero; a field
    // that begins with anything else is no octal field.
    let padded = field.iter().all(|byte| *byte == 0 || *byte == b' ');
    if padded {
        Ok(0)
    } else {
        Err(TarError::BadOctal { at })
    }
}

/// What the type flag says the entry is.
fn kind_of(header: &[u8; BLOCK]) -> Kind {
    match header.get(TYPE_FLAG).copied().unwrap_or(0) {
        b'0' | 0 => Kind::File,
        b'5' => Kind::Directory,
        other => Kind::Other(other),
    }
}

/// The name of the entry, prefix and name field joined with a slash, and
/// checked.
fn path_of(header: &[u8; BLOCK]) -> Result<Path, TarError> {
    let prefix = field(header, PREFIX, PREFIX_LEN)?;
    let name = field(header, NAME, NAME_LEN)?;
    let mut path = Path {
        bytes: [0; MAX_PATH],
        len: 0,
    };
    append(&mut path, prefix);
    if !prefix.is_empty() && !name.is_empty() {
        append(&mut path, b"/");
    }
    append(&mut path, name);
    if path.len == 0 {
        return Err(TarError::EmptyName);
    }
    check_path(path.as_bytes())?;
    Ok(path)
}

/// The bytes of a text field up to its terminating zero.
///
/// Everything behind the zero has to be zero as well. A field with bytes
/// behind its terminator is a header two readers would disagree about, and
/// the name of a file the root task is about to start is not a place to
/// disagree.
fn field(header: &[u8; BLOCK], at: usize, len: usize) -> Result<&[u8], TarError> {
    let end = at
        .checked_add(len)
        .ok_or(TarError::NameNotTerminated { at })?;
    let bytes = header
        .get(at..end)
        .ok_or(TarError::NameNotTerminated { at })?;
    let used = bytes.iter().position(|byte| *byte == 0).unwrap_or(len);
    let text = bytes
        .get(..used)
        .ok_or(TarError::NameNotTerminated { at })?;
    let rest = bytes.get(used..).unwrap_or(&[]);
    if rest.iter().any(|byte| *byte != 0) {
        return Err(TarError::NameNotTerminated { at });
    }
    Ok(text)
}

/// Appends what fits of `bytes` to `path`. Nothing is cut in practice: the
/// two fields and the slash are exactly [`MAX_PATH`] bytes.
fn append(path: &mut Path, bytes: &[u8]) {
    let end = path.len.saturating_add(bytes.len()).min(MAX_PATH);
    let slot = path.bytes.get_mut(path.len..end).unwrap_or_default();
    let source = bytes.get(..slot.len()).unwrap_or(&[]);
    slot.copy_from_slice(source);
    path.len = end;
}

/// Refuses a name that reaches outside the archive.
fn check_path(path: &[u8]) -> Result<(), TarError> {
    if path.first() == Some(&b'/') {
        return Err(TarError::AbsolutePath);
    }
    if path.split(|byte| *byte == b'/').any(|part| part == b"..") {
        return Err(TarError::ParentComponent);
    }
    Ok(())
}

/// Offset of the mode field.
const MODE: usize = 100;

/// Offset of the owner field.
const UID: usize = 108;

/// Offset of the group field.
const GID: usize = 116;

/// Offset of the modification time.
const MTIME: usize = 136;

/// Offset of the version field.
const VERSION: usize = 263;

/// The version a ustar writer puts there.
const VERSION_BYTES: [u8; 2] = *b"00";

/// The mode every file of a boot archive gets: readable by everyone,
/// writable by nobody. Nothing of this system looks at it; it is written
/// because a header of this format has the field.
const MODE_BYTES: u64 = 0o444;

/// Why an archive could not be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WriteError {
    /// The buffer has no room for another block.
    Full,
    /// A name that does not fit the name field and the prefix field, even
    /// split at a slash.
    NameTooLong(usize),
}

impl core::fmt::Display for WriteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WriteError::Full => f.write_str("the buffer holds no further block"),
            WriteError::NameTooLong(len) => write!(f, "a name of {len} bytes"),
        }
    }
}

/// How many bytes an archive of these files occupies: a header and the
/// padded content per file, and the two blocks of the end marker.
#[must_use]
pub fn archive_len(files: &[(&[u8], &[u8])]) -> usize {
    let content = files.iter().fold(0usize, |total, (_, data)| {
        total
            .saturating_add(BLOCK)
            .saturating_add(data.len().next_multiple_of(BLOCK))
    });
    content.saturating_add(BLOCK.saturating_mul(2))
}

/// Writes a ustar archive into bytes the caller owns.
///
/// The writer exists so that the archive of the boot image and the archives
/// the tests read are made by the same code, and so that what this crate
/// reads can be checked against what it writes.
#[derive(Debug)]
pub struct Builder<'a> {
    bytes: &'a mut [u8],
    at: usize,
}

impl<'a> Builder<'a> {
    /// A writer over `bytes`, which has to hold what will be written; see
    /// [`archive_len`].
    #[must_use]
    pub const fn new(bytes: &'a mut [u8]) -> Self {
        Builder { bytes, at: 0 }
    }

    /// How many bytes have been written.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.at
    }

    /// `true` before the first entry.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.at == 0
    }

    /// Appends a regular file.
    ///
    /// # Errors
    ///
    /// [`WriteError::Full`] when the buffer has no room;
    /// [`WriteError::NameTooLong`] for a name that does not fit the two
    /// fields.
    pub fn file(&mut self, name: &[u8], data: &[u8]) -> Result<(), WriteError> {
        self.entry(name, Kind::File, data)
    }

    /// Appends an entry of any kind.
    ///
    /// # Errors
    ///
    /// As [`file`](Self::file).
    pub fn entry(&mut self, name: &[u8], kind: Kind, data: &[u8]) -> Result<(), WriteError> {
        // The size field holds eleven octal digits, which is eight
        // gibibytes; the buffer would have to hold that much before a file
        // could reach it, and nothing in this system builds an archive in
        // a buffer of that size.
        let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
        let mut header = [0u8; BLOCK];
        let (prefix, name) = split_name(name)?;
        put(&mut header, NAME, name);
        put(&mut header, PREFIX, prefix);
        put_octal(&mut header, MODE, 8, MODE_BYTES);
        put_octal(&mut header, UID, 8, 0);
        put_octal(&mut header, GID, 8, 0);
        put_octal(&mut header, SIZE, SIZE_LEN, size);
        put_octal(&mut header, MTIME, 12, 0);
        put(&mut header, MAGIC, &USTAR);
        put(&mut header, VERSION, &VERSION_BYTES);
        let flag = match kind {
            Kind::File => b'0',
            Kind::Directory => b'5',
            Kind::Other(flag) => flag,
        };
        put(&mut header, TYPE_FLAG, core::slice::from_ref(&flag));
        // The checksum field counts as spaces while the sum is taken, which
        // is how the number can stand inside the block it covers.
        put(&mut header, CHECKSUM, &[b' '; CHECKSUM_LEN]);
        let sum = header
            .iter()
            .fold(0u64, |total, byte| total.wrapping_add(u64::from(*byte)));
        put_octal(&mut header, CHECKSUM, 7, sum);
        put(&mut header, CHECKSUM.wrapping_add(7), b" ");

        self.put_block(&header)?;
        for chunk in data.chunks(BLOCK) {
            let mut block = [0u8; BLOCK];
            put(&mut block, 0, chunk);
            self.put_block(&block)?;
        }
        Ok(())
    }

    /// Writes the two zero blocks that end an archive and answers with how
    /// many bytes the archive occupies.
    ///
    /// # Errors
    ///
    /// [`WriteError::Full`] when the buffer has no room for them.
    pub fn finish(mut self) -> Result<usize, WriteError> {
        let zero = [0u8; BLOCK];
        self.put_block(&zero)?;
        self.put_block(&zero)?;
        Ok(self.at)
    }

    /// Writes one block at the cursor.
    fn put_block(&mut self, block: &[u8; BLOCK]) -> Result<(), WriteError> {
        // The cursor never leaves the buffer, so a block behind it cannot
        // overflow; whether it fits is what `get_mut` answers.
        let end = self.at.saturating_add(BLOCK);
        let slot = self.bytes.get_mut(self.at..end).ok_or(WriteError::Full)?;
        slot.copy_from_slice(block);
        self.at = end;
        Ok(())
    }
}

/// Splits a name into the prefix field and the name field.
///
/// A name of at most a hundred bytes goes into the name field alone. A
/// longer one is cut at the last slash that leaves at most a hundred bytes
/// behind it, which is where the format says the prefix ends.
fn split_name(name: &[u8]) -> Result<(&[u8], &[u8]), WriteError> {
    if name.len() <= NAME_LEN {
        return Ok((&[], name));
    }
    let cut = name
        .iter()
        .enumerate()
        .filter(|(index, byte)| {
            **byte == b'/'
                && *index <= PREFIX_LEN
                && name.len().saturating_sub(index.saturating_add(1)) <= NAME_LEN
        })
        .map(|(index, _)| index)
        .next()
        .ok_or(WriteError::NameTooLong(name.len()))?;
    // `cut` is an index of `name`, so both halves are there.
    let prefix = name.get(..cut).unwrap_or(&[]);
    let rest = name.get(cut.saturating_add(1)..).unwrap_or(&[]);
    Ok((prefix, rest))
}

/// Copies `bytes` into `header` at `at`, as far as they fit.
fn put(header: &mut [u8; BLOCK], at: usize, bytes: &[u8]) {
    let end = at.saturating_add(bytes.len());
    let slot = header.get_mut(at..end).unwrap_or_default();
    let source = bytes.get(..slot.len()).unwrap_or(&[]);
    slot.copy_from_slice(source);
}

/// Writes `value` as `digits` octal digits followed by a zero byte.
fn put_octal(header: &mut [u8; BLOCK], at: usize, width: usize, value: u64) {
    let digits = width.saturating_sub(1);
    let mut field = [b'0'; SIZE_LEN];
    let mut rest = value;
    // The field is filled from its last digit backwards, which is what
    // puts the number to the right with zeros in front of it.
    for slot in field.iter_mut().take(digits).rev() {
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "the remainder of a division by eight is one octal digit"
        )]
        let digit = (rest.wrapping_rem(8) as u8).wrapping_add(b'0');
        *slot = digit;
        rest = rest.wrapping_div(8);
    }
    let written = field.get(..digits).unwrap_or(&[]);
    put(header, at, written);
    put(header, at.saturating_add(digits), &[0]);
}
