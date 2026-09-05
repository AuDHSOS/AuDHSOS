// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The DWARF line program: which file and line an address belongs to.
//!
//! Invariants: the state machine runs once per lookup and keeps only the
//! row it needs, so that the crate allocates nothing; a row counts for an
//! address only inside the sequence that ends above it, which is what
//! keeps the rows of one unit from answering for another; every read is
//! bounded by the unit it belongs to, so a truncated program is an error
//! and never a panic.

use crate::cursor::Cursor;
use crate::error::SymbolError;

/// The versions of the line program this crate reads.
const SUPPORTED: [u16; 2] = [4, 5];

/// The `unit_length` that says the offsets of the unit are 64 bits wide.
const DWARF64: u32 = 0xFFFF_FFFF;

/// The opcode that introduces an extended one.
const EXTENDED: u8 = 0;

/// Standard opcodes.
const DW_LNS_COPY: u8 = 1;
const DW_LNS_ADVANCE_PC: u8 = 2;
const DW_LNS_ADVANCE_LINE: u8 = 3;
const DW_LNS_SET_FILE: u8 = 4;
const DW_LNS_SET_COLUMN: u8 = 5;
const DW_LNS_NEGATE_STMT: u8 = 6;
const DW_LNS_SET_BASIC_BLOCK: u8 = 7;
const DW_LNS_CONST_ADD_PC: u8 = 8;
const DW_LNS_FIXED_ADVANCE_PC: u8 = 9;
const DW_LNS_SET_PROLOGUE_END: u8 = 10;
const DW_LNS_SET_EPILOGUE_BEGIN: u8 = 11;
const DW_LNS_SET_ISA: u8 = 12;

/// Extended opcodes.
const DW_LNE_END_SEQUENCE: u8 = 1;
const DW_LNE_SET_ADDRESS: u8 = 2;
const DW_LNE_SET_DISCRIMINATOR: u8 = 4;

/// Content types of a version 5 file or directory entry.
const DW_LNCT_PATH: u64 = 1;
const DW_LNCT_DIRECTORY_INDEX: u64 = 2;

/// Forms a version 5 file or directory entry may use.
const DW_FORM_BLOCK: u64 = 0x09;
const DW_FORM_DATA1: u64 = 0x0B;
const DW_FORM_DATA2: u64 = 0x05;
const DW_FORM_DATA4: u64 = 0x06;
const DW_FORM_DATA8: u64 = 0x07;
const DW_FORM_DATA16: u64 = 0x1E;
const DW_FORM_LINE_STRP: u64 = 0x1F;
const DW_FORM_STRING: u64 = 0x08;
const DW_FORM_STRP: u64 = 0x0E;
const DW_FORM_UDATA: u64 = 0x0F;

/// The string sections a version 5 file table may point into.
#[derive(Clone, Copy, Debug, Default)]
pub struct Strings<'a> {
    /// The content of `.debug_str`.
    pub debug_str: &'a [u8],
    /// The content of `.debug_line_str`.
    pub debug_line_str: &'a [u8],
}

impl<'a> Strings<'a> {
    /// The string at `offset` in the section `form` names.
    fn at(&self, form: u64, offset: u64) -> Option<&'a str> {
        let section = if form == DW_FORM_LINE_STRP {
            self.debug_line_str
        } else {
            self.debug_str
        };
        let start = usize::try_from(offset).ok()?;
        let rest = section.get(start..)?;
        let end = rest.iter().position(|byte| *byte == 0)?;
        core::str::from_utf8(rest.get(..end)?).ok()
    }
}

/// Where an address falls in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row<'a> {
    /// The file, as the file table spells it.
    pub file: Option<&'a str>,
    /// The directory of that file, when the table names one.
    pub directory: Option<&'a str>,
    /// The line, or zero when the program names none.
    pub line: u32,
    /// The column, or zero when the program names none.
    pub column: u32,
}

/// The header of one line program unit.
#[derive(Clone, Copy, Debug)]
struct Header {
    version: u16,
    address_size: usize,
    offset_size: usize,
    minimum_instruction_length: u64,
    maximum_operations_per_instruction: u64,
    line_base: i64,
    line_range: u64,
    opcode_base: u8,
    /// Where the standard opcode lengths start in the unit.
    lengths_at: usize,
    /// Where the file table starts in the unit.
    files_at: usize,
    /// Where the program starts in the unit.
    program_at: usize,
    /// Where the unit ends.
    end: usize,
}

/// The registers of the line state machine that a lookup cares about.
#[derive(Clone, Copy, Debug)]
struct State {
    address: u64,
    op_index: u64,
    file: u64,
    line: i64,
    column: u64,
}

impl State {
    /// The state a sequence starts in. Both versions start at file one:
    /// version 5 numbers its table from zero, but the register still
    /// starts at one.
    const fn new() -> Self {
        State {
            address: 0,
            op_index: 0,
            file: 1,
            line: 1,
            column: 0,
        }
    }
}

/// What one opcode did.
#[derive(Clone, Copy, Debug)]
struct Step {
    /// The opcode emitted a row.
    emitted: bool,
    /// The row ended the sequence.
    ended: bool,
}

impl Step {
    /// An opcode that emitted nothing.
    const fn none() -> Self {
        Step {
            emitted: false,
            ended: false,
        }
    }

    /// An opcode that emitted a row.
    const fn row() -> Self {
        Step {
            emitted: true,
            ended: false,
        }
    }

    /// An opcode that ended the sequence.
    const fn end() -> Self {
        Step {
            emitted: true,
            ended: true,
        }
    }
}

/// What a lookup has found so far in the sequence it is walking.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    address: u64,
    file: u64,
    line: i64,
    column: u64,
}

/// Reads `.debug_line` and answers where an address falls.
#[derive(Clone, Copy, Debug)]
pub struct LineProgram<'a> {
    bytes: &'a [u8],
    strings: Strings<'a>,
}

impl<'a> LineProgram<'a> {
    /// A program over the content of `.debug_line`, with the string
    /// sections a version 5 file table may point into.
    #[must_use]
    pub const fn new(bytes: &'a [u8], strings: Strings<'a>) -> Self {
        LineProgram { bytes, strings }
    }

    /// `true` if the file carries no line program.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The row that covers `address`, from the first unit that has one.
    ///
    /// # Errors
    ///
    /// [`SymbolError::Truncated`] if a unit reaches beyond the section;
    /// [`SymbolError::LineVersion`] for a version this crate does not
    /// read; the errors of the file table.
    pub fn row_for(&self, address: u64) -> Result<Option<Row<'a>>, SymbolError> {
        let mut unit = 0usize;
        while unit < self.bytes.len() {
            let header = self.header(unit)?;
            if let Some(found) = self.run(&header, address)? {
                return Ok(Some(self.row(&header, found)?));
            }
            unit = header.end;
        }
        Ok(None)
    }

    /// Reads the header of the unit that starts at `start`.
    fn header(&self, start: usize) -> Result<Header, SymbolError> {
        let mut cursor = Cursor::new(self.bytes);
        cursor.seek(start)?;
        let first = cursor.u32()?;
        let (offset_size, length) = if first == DWARF64 {
            (8usize, cursor.u64()?)
        } else {
            (4usize, u64::from(first))
        };
        let length = usize::try_from(length).map_err(|_| SymbolError::Overflow)?;
        let end = cursor
            .position()
            .checked_add(length)
            .ok_or(SymbolError::Truncated)?;
        if end > self.bytes.len() {
            return Err(SymbolError::Truncated);
        }
        let version = cursor.u16()?;
        if !SUPPORTED.contains(&version) {
            return Err(SymbolError::LineVersion(version));
        }
        let mut address_size = 8usize;
        if version >= 5 {
            address_size = usize::from(cursor.u8()?);
            let _segment_selector_size = cursor.u8()?;
        }
        let header_length = cursor.unsigned(offset_size)?;
        let header_length = usize::try_from(header_length).map_err(|_| SymbolError::Overflow)?;
        let program_at = cursor
            .position()
            .checked_add(header_length)
            .ok_or(SymbolError::Truncated)?;
        if program_at > end {
            return Err(SymbolError::Truncated);
        }
        let minimum_instruction_length = u64::from(cursor.u8()?);
        let maximum_operations_per_instruction = if version >= 4 {
            u64::from(cursor.u8()?)
        } else {
            1
        };
        // The default statement flag is a register a lookup does not
        // care about; the byte still has to be read to reach the rest.
        let _default_is_stmt = cursor.u8()?;
        let line_base = i64::from(cursor.i8()?);
        let line_range = u64::from(cursor.u8()?);
        if line_range == 0 {
            return Err(SymbolError::LineRange);
        }
        let opcode_base = cursor.u8()?;
        let lengths_at = cursor.position();
        cursor.skip(usize::from(opcode_base.saturating_sub(1)))?;
        let files_at = cursor.position();
        Ok(Header {
            version,
            address_size,
            offset_size,
            minimum_instruction_length,
            maximum_operations_per_instruction: maximum_operations_per_instruction.max(1),
            line_base,
            line_range,
            opcode_base,
            lengths_at,
            files_at,
            program_at,
            end,
        })
    }

    /// The length of standard opcode `opcode`, in operands.
    fn operands(&self, header: &Header, opcode: u8) -> Result<usize, SymbolError> {
        let index = usize::from(opcode.saturating_sub(1));
        let mut cursor = Cursor::new(self.bytes);
        cursor.seek(
            header
                .lengths_at
                .checked_add(index)
                .ok_or(SymbolError::Truncated)?,
        )?;
        Ok(usize::from(cursor.u8()?))
    }

    /// Runs the state machine of one unit and returns the row that covers
    /// `address`, if the unit has one.
    fn run(&self, header: &Header, address: u64) -> Result<Option<Candidate>, SymbolError> {
        let mut cursor = Cursor::new(self.bytes);
        cursor.seek(header.program_at)?;
        let mut state = State::new();
        let mut candidate: Option<Candidate> = None;
        while cursor.position() < header.end {
            let step = self.step(header, &mut cursor, &mut state)?;
            if !step.emitted {
                continue;
            }
            if step.ended {
                if let Some(found) = candidate
                    && address < state.address
                {
                    return Ok(Some(found));
                }
                candidate = None;
                state = State::new();
            } else if state.address <= address
                && candidate.is_none_or(|found| found.address <= state.address)
            {
                candidate = Some(Candidate {
                    address: state.address,
                    file: state.file,
                    line: state.line,
                    column: state.column,
                });
            }
        }
        Ok(None)
    }

    /// Runs one opcode of the program.
    fn step(
        &self,
        header: &Header,
        cursor: &mut Cursor<'a>,
        state: &mut State,
    ) -> Result<Step, SymbolError> {
        let opcode = cursor.u8()?;
        if opcode >= header.opcode_base {
            let adjusted = u64::from(opcode.wrapping_sub(header.opcode_base));
            advance(header, state, per_line(header, adjusted));
            let step = adjusted.checked_rem(header.line_range).unwrap_or(0);
            state.line = state
                .line
                .saturating_add(header.line_base)
                .saturating_add(i64::try_from(step).unwrap_or(0));
            return Ok(Step::row());
        }
        if opcode == EXTENDED {
            return Self::extended(header, cursor, state);
        }
        self.standard(header, cursor, state, opcode)
    }

    /// Runs one extended opcode, whose length carries the cursor past
    /// whatever this crate does not read.
    fn extended(
        header: &Header,
        cursor: &mut Cursor<'a>,
        state: &mut State,
    ) -> Result<Step, SymbolError> {
        let length = cursor.uleb_usize()?;
        let end = cursor
            .position()
            .checked_add(length)
            .ok_or(SymbolError::Truncated)?;
        if length == 0 {
            return Ok(Step::none());
        }
        let sub = cursor.u8()?;
        let step = match sub {
            DW_LNE_END_SEQUENCE => Step::end(),
            DW_LNE_SET_ADDRESS => {
                state.address = cursor.unsigned(header.address_size)?;
                state.op_index = 0;
                Step::none()
            }
            DW_LNE_SET_DISCRIMINATOR => {
                let _ = cursor.uleb()?;
                Step::none()
            }
            // Version 4 could define a file with `DW_LNE_define_file`,
            // which this crate does not read; the length carries the
            // cursor past it and past every opcode of a later standard,
            // so neither needs an arm of its own.
            _ => Step::none(),
        };
        cursor.seek(end)?;
        Ok(step)
    }

    /// Runs one standard opcode.
    fn standard(
        &self,
        header: &Header,
        cursor: &mut Cursor<'a>,
        state: &mut State,
        opcode: u8,
    ) -> Result<Step, SymbolError> {
        match opcode {
            DW_LNS_COPY => return Ok(Step::row()),
            DW_LNS_ADVANCE_PC => {
                let operation = cursor.uleb()?;
                advance(header, state, operation);
            }
            DW_LNS_ADVANCE_LINE => state.line = state.line.saturating_add(cursor.sleb()?),
            DW_LNS_SET_FILE => state.file = cursor.uleb()?,
            DW_LNS_SET_COLUMN => state.column = cursor.uleb()?,
            DW_LNS_NEGATE_STMT
            | DW_LNS_SET_BASIC_BLOCK
            | DW_LNS_SET_PROLOGUE_END
            | DW_LNS_SET_EPILOGUE_BEGIN => {}
            DW_LNS_CONST_ADD_PC => {
                let adjusted = u64::from(255u8.wrapping_sub(header.opcode_base));
                advance(header, state, per_line(header, adjusted));
            }
            DW_LNS_FIXED_ADVANCE_PC => {
                state.address = state.address.saturating_add(u64::from(cursor.u16()?));
                state.op_index = 0;
            }
            DW_LNS_SET_ISA => {
                let _ = cursor.uleb()?;
            }
            other => {
                for _ in 0..self.operands(header, other)? {
                    let _ = cursor.uleb()?;
                }
            }
        }
        Ok(Step::none())
    }

    /// Turns a candidate into the row a caller sees.
    fn row(&self, header: &Header, found: Candidate) -> Result<Row<'a>, SymbolError> {
        let (file, directory) = self.file(header, found.file)?;
        Ok(Row {
            file,
            directory,
            line: u32::try_from(found.line).unwrap_or(0),
            column: u32::try_from(found.column).unwrap_or(0),
        })
    }

    /// The name and the directory of file `index` of the unit's table.
    fn file(
        &self,
        header: &Header,
        index: u64,
    ) -> Result<(Option<&'a str>, Option<&'a str>), SymbolError> {
        if header.version >= 5 {
            self.file_v5(header, index)
        } else {
            self.file_v4(header, index)
        }
    }

    /// The file table of a version 4 unit: directories first, then files.
    fn file_v4(
        &self,
        header: &Header,
        index: u64,
    ) -> Result<(Option<&'a str>, Option<&'a str>), SymbolError> {
        let mut cursor = Cursor::new(self.bytes);
        cursor.seek(header.files_at)?;
        let directories_at = cursor.position();
        loop {
            let entry = cursor.string()?;
            if entry.is_empty() {
                break;
            }
        }
        let mut number = 1u64;
        loop {
            let name = cursor.string()?;
            if name.is_empty() {
                return Ok((None, None));
            }
            let directory = cursor.uleb()?;
            let _mtime = cursor.uleb()?;
            let _length = cursor.uleb()?;
            if number == index {
                return Ok((Some(name), self.directory_v4(directories_at, directory)?));
            }
            number = number.saturating_add(1);
        }
    }

    /// Directory `index` of a version 4 unit; index zero is the directory
    /// of the compilation, which the table does not name.
    fn directory_v4(&self, start: usize, index: u64) -> Result<Option<&'a str>, SymbolError> {
        if index == 0 {
            return Ok(None);
        }
        let mut cursor = Cursor::new(self.bytes);
        cursor.seek(start)?;
        let mut number = 1u64;
        loop {
            let entry = cursor.string()?;
            if entry.is_empty() {
                return Ok(None);
            }
            if number == index {
                return Ok(Some(entry));
            }
            number = number.saturating_add(1);
        }
    }

    /// The file table of a version 5 unit, where both tables carry the
    /// format of their entries and file numbers start at zero.
    fn file_v5(
        &self,
        header: &Header,
        index: u64,
    ) -> Result<(Option<&'a str>, Option<&'a str>), SymbolError> {
        let mut cursor = Cursor::new(self.bytes);
        cursor.seek(header.files_at)?;
        let directories_at = cursor.position();
        skip_table(&mut cursor, header, &self.strings)?;
        let (name, directory) = self.entry_v5(&mut cursor, header, index)?;
        let Some(directory) = directory else {
            return Ok((name, None));
        };
        let mut directories = Cursor::new(self.bytes);
        directories.seek(directories_at)?;
        let (path, _) = self.entry_v5(&mut directories, header, directory)?;
        Ok((name, path))
    }

    /// Reads entry `index` of a version 5 table the cursor stands on.
    fn entry_v5(
        &self,
        cursor: &mut Cursor<'a>,
        header: &Header,
        index: u64,
    ) -> Result<(Option<&'a str>, Option<u64>), SymbolError> {
        let format = Format::read(cursor)?;
        let count = cursor.uleb()?;
        let mut number = 0u64;
        while number < count {
            let (path, directory) = format.read_entry(cursor, header, &self.strings)?;
            if number == index {
                return Ok((path, directory));
            }
            number = number.saturating_add(1);
        }
        Ok((None, None))
    }
}

/// The format of the entries of one version 5 table.
#[derive(Clone, Copy, Debug)]
struct Format {
    pairs: [(u64, u64); MAX_FORMAT_PAIRS],
    count: usize,
}

/// The number of content type and form pairs one table may declare.
const MAX_FORMAT_PAIRS: usize = 8;

impl Format {
    /// Reads the format the cursor stands on.
    fn read(cursor: &mut Cursor<'_>) -> Result<Self, SymbolError> {
        let declared = usize::from(cursor.u8()?);
        let mut pairs = [(0u64, 0u64); MAX_FORMAT_PAIRS];
        let mut count = 0usize;
        for index in 0..declared {
            let kind = cursor.uleb()?;
            let form = cursor.uleb()?;
            if let Some(slot) = pairs.get_mut(index) {
                *slot = (kind, form);
                count = count.saturating_add(1);
            }
        }
        if declared > MAX_FORMAT_PAIRS {
            return Err(SymbolError::Overflow);
        }
        Ok(Format { pairs, count })
    }

    /// Reads one entry and returns its path and its directory index.
    fn read_entry<'a>(
        &self,
        cursor: &mut Cursor<'a>,
        header: &Header,
        strings: &Strings<'a>,
    ) -> Result<(Option<&'a str>, Option<u64>), SymbolError> {
        let mut path = None;
        let mut directory = None;
        for (kind, form) in self.pairs.iter().take(self.count) {
            let value = read_form(cursor, header, strings, *form)?;
            match *kind {
                DW_LNCT_PATH => path = value.text,
                DW_LNCT_DIRECTORY_INDEX => directory = value.number,
                _ => {}
            }
        }
        Ok((path, directory))
    }
}

/// What one form of a version 5 entry carried.
#[derive(Clone, Copy, Debug, Default)]
struct Value<'a> {
    text: Option<&'a str>,
    number: Option<u64>,
}

/// Reads one value of the given form.
fn read_form<'a>(
    cursor: &mut Cursor<'a>,
    header: &Header,
    strings: &Strings<'a>,
    form: u64,
) -> Result<Value<'a>, SymbolError> {
    match form {
        DW_FORM_STRING => Ok(Value {
            text: Some(cursor.string()?),
            number: None,
        }),
        DW_FORM_STRP | DW_FORM_LINE_STRP => {
            let offset = cursor.unsigned(header.offset_size)?;
            Ok(Value {
                text: strings.at(form, offset),
                number: None,
            })
        }
        DW_FORM_UDATA => Ok(Value {
            text: None,
            number: Some(cursor.uleb()?),
        }),
        DW_FORM_DATA1 => Ok(Value {
            text: None,
            number: Some(u64::from(cursor.u8()?)),
        }),
        DW_FORM_DATA2 => Ok(Value {
            text: None,
            number: Some(u64::from(cursor.u16()?)),
        }),
        DW_FORM_DATA4 => Ok(Value {
            text: None,
            number: Some(u64::from(cursor.u32()?)),
        }),
        DW_FORM_DATA8 => Ok(Value {
            text: None,
            number: Some(cursor.u64()?),
        }),
        DW_FORM_DATA16 => {
            cursor.skip(16)?;
            Ok(Value::default())
        }
        DW_FORM_BLOCK => {
            let length = cursor.uleb_usize()?;
            cursor.skip(length)?;
            Ok(Value::default())
        }
        other => Err(SymbolError::UnknownForm(other)),
    }
}

/// Skips a whole version 5 table, format and entries.
fn skip_table<'a>(
    cursor: &mut Cursor<'a>,
    header: &Header,
    strings: &Strings<'a>,
) -> Result<(), SymbolError> {
    let format = Format::read(cursor)?;
    let count = cursor.uleb()?;
    let mut number = 0u64;
    while number < count {
        format.read_entry(cursor, header, strings)?;
        number = number.saturating_add(1);
    }
    Ok(())
}

/// The operation advance an adjusted opcode carries. The header keeps
/// `line_range` above zero, so the division cannot fail.
const fn per_line(header: &Header, adjusted: u64) -> u64 {
    match adjusted.checked_div(header.line_range) {
        Some(advance) => advance,
        None => 0,
    }
}

/// Advances the address by `operations` operations of the machine. The
/// header keeps `maximum_operations_per_instruction` at one or above, so
/// neither division can fail.
const fn advance(header: &Header, state: &mut State, operations: u64) {
    let total = state.op_index.saturating_add(operations);
    let (steps, next) = match (
        total.checked_div(header.maximum_operations_per_instruction),
        total.checked_rem(header.maximum_operations_per_instruction),
    ) {
        (Some(steps), Some(next)) => (steps, next),
        _ => (0, 0),
    };
    state.op_index = next;
    state.address = state
        .address
        .saturating_add(header.minimum_instruction_length.saturating_mul(steps));
}
