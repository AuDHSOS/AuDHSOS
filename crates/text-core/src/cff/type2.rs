// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::blend::Blend;
use super::{
    Index,
    dict::{Cursor, integer, offset},
};
use crate::{
    Fixed, FontError,
    read::{add, sub},
};

/// A cubic-path coordinate in font design units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    /// Horizontal coordinate.
    pub x: Fixed,
    /// Vertical coordinate, positive upward.
    pub y: Fixed,
}
impl Position {
    fn relative(self, x: Fixed, y: Fixed) -> Result<Self, FontError> {
        Ok(Self {
            x: self.x.checked_add(x)?,
            y: self.y.checked_add(y)?,
        })
    }
}

/// An unhinted cubic path command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Command {
    /// Begin a contour at this point.
    Move(Position),
    /// Draw a straight segment to this point.
    Line(Position),
    /// Cubic control point one, control point two, and endpoint.
    Curve(Position, Position, Position),
    /// Close the current contour.
    #[default]
    Close,
}
impl Command {
    pub(super) fn transform(
        &mut self,
        mut f: impl FnMut(Position) -> Result<Position, FontError>,
    ) -> Result<(), FontError> {
        match self {
            Self::Move(p) | Self::Line(p) => *p = f(*p)?,
            Self::Curve(a, b, c) => {
                *a = f(*a)?;
                *b = f(*b)?;
                *c = f(*c)?;
            }
            Self::Close => (),
        }
        Ok(())
    }
}

pub(super) fn decode(
    program: &[u8],
    local: Index<'_>,
    global: Index<'_>,
    cff2: bool,
    commands: &mut [Command],
    blend: Blend<'_>,
) -> Result<(usize, Option<[Fixed; 4]>), FontError> {
    let mut m = Machine {
        stack: [Fixed::ZERO; 513],
        len: 0,
        transient: [None; 32],
        position: Position::default(),
        open: false,
        width_seen: cff2,
        stems: 0,
        output: commands,
        count: 0,
        operations: 0,
        path: [(false, 0); 10],
        random: 1,
        cff2,
        seac: None,
        blend,
    };
    let ended = m.run(program, local, global, 0)?;
    if !cff2 && !ended {
        return Err(FontError::InvalidTable);
    }
    if m.len != 0 {
        return Err(FontError::InvalidTable);
    }
    m.close()?;
    Ok((m.count, m.seac))
}

struct Machine<'a, 'b> {
    stack: [Fixed; 513],
    len: usize,
    transient: [Option<Fixed>; 32],
    position: Position,
    open: bool,
    width_seen: bool,
    stems: usize,
    output: &'a mut [Command],
    count: usize,
    operations: usize,
    path: [(bool, usize); 10],
    random: u32,
    cff2: bool,
    seac: Option<[Fixed; 4]>,
    blend: Blend<'b>,
}
impl Machine<'_, '_> {
    fn push(&mut self, value: Fixed) -> Result<(), FontError> {
        if self.len >= if self.cff2 { 513 } else { 48 } {
            return Err(FontError::LimitExceeded);
        }
        *self
            .stack
            .get_mut(self.len)
            .ok_or(FontError::LimitExceeded)? = value;
        self.len = add(self.len, 1)?;
        Ok(())
    }
    fn pop(&mut self) -> Result<Fixed, FontError> {
        self.len = sub(self.len, 1)?;
        self.arg(self.len)
    }
    fn arg(&self, index: usize) -> Result<Fixed, FontError> {
        self.stack
            .get(index)
            .copied()
            .ok_or(FontError::InvalidTable)
    }
    fn emit(&mut self, command: Command) -> Result<(), FontError> {
        *self
            .output
            .get_mut(self.count)
            .ok_or(FontError::BufferTooSmall)? = command;
        self.count = add(self.count, 1)?;
        Ok(())
    }
    fn close(&mut self) -> Result<(), FontError> {
        if self.open {
            self.emit(Command::Close)?;
            self.open = false;
        }
        Ok(())
    }
    fn line(&mut self, x: Fixed, y: Fixed) -> Result<(), FontError> {
        if !self.open {
            return Err(FontError::InvalidTable);
        }
        self.position = self.position.relative(x, y)?;
        self.emit(Command::Line(self.position))
    }
    fn curve(&mut self, d: [Fixed; 6]) -> Result<(), FontError> {
        if !self.open {
            return Err(FontError::InvalidTable);
        }
        let [x1, y1, x2, y2, x3, y3] = d;
        let a = self.position.relative(x1, y1)?;
        let b = a.relative(x2, y2)?;
        let c = b.relative(x3, y3)?;
        self.position = c;
        self.emit(Command::Curve(a, b, c))
    }
    fn width(&mut self, expected: usize) -> Result<(), FontError> {
        if !self.width_seen {
            if self.len > expected {
                self.stack.copy_within(1..self.len, 0);
                self.len = sub(self.len, 1)?;
            }
            self.width_seen = true;
        }
        Ok(())
    }
    fn stems(&mut self) -> Result<(), FontError> {
        if !self.width_seen && !self.len.is_multiple_of(2) {
            self.width(sub(self.len, 1)?)?;
        }
        self.width_seen = true;
        if !self.len.is_multiple_of(2) {
            return Err(FontError::InvalidTable);
        }
        self.stems = add(self.stems, self.len / 2)?;
        if self.stems > 96 {
            return Err(FontError::LimitExceeded);
        }
        self.len = 0;
        Ok(())
    }

    fn run(
        &mut self,
        program: &[u8],
        local: Index<'_>,
        global: Index<'_>,
        depth: usize,
    ) -> Result<bool, FontError> {
        if program.len() > 65535 {
            return Err(FontError::LimitExceeded);
        }
        let mut c = Cursor {
            data: program,
            at: 0,
        };
        while c.at < program.len() {
            self.operations = add(self.operations, 1)?;
            if self.operations > 65536 {
                return Err(FontError::LimitExceeded);
            }
            let op = c.byte()?;
            if let Some(n) = c.number(op, false)? {
                self.push(n)?;
                continue;
            }
            match op {
                1 | 3 | 18 | 23 => self.stems()?,
                19 | 20 => {
                    self.stems()?;
                    c.skip(add(self.stems, 7)? / 8)?;
                }
                10 | 29 => {
                    let index = if op == 10 { local } else { global };
                    let bias = if index.len() < 1240 {
                        107
                    } else if index.len() < 33900 {
                        1131
                    } else {
                        32768
                    };
                    let id = usize::try_from(
                        integer(self.pop()?)?
                            .checked_add(bias)
                            .ok_or(FontError::Overflow)?,
                    )
                    .map_err(|_| FontError::InvalidTable)?;
                    let key = (op == 29, id);
                    if self
                        .path
                        .get(..depth)
                        .ok_or(FontError::LimitExceeded)?
                        .contains(&key)
                    {
                        return Err(FontError::Cycle);
                    }
                    *self.path.get_mut(depth).ok_or(FontError::LimitExceeded)? = key;
                    if self.run(index.get(id)?, local, global, add(depth, 1)?)? {
                        return Ok(true);
                    }
                }
                11 if !self.cff2 => {
                    if depth == 0 {
                        return Err(FontError::InvalidTable);
                    }
                    return Ok(false);
                }
                14 if !self.cff2 => {
                    if !self.width_seen && (self.len == 1 || self.len == 5) {
                        self.width(sub(self.len, 1)?)?;
                    }
                    if self.len == 4 {
                        self.seac = Some([self.arg(0)?, self.arg(1)?, self.arg(2)?, self.arg(3)?]);
                        self.len = 0;
                    } else if self.len != 0 {
                        return Err(FontError::InvalidTable);
                    }
                    return Ok(true);
                }
                12 => {
                    let escaped = c.byte()?;
                    if matches!(escaped, 34..=37) {
                        self.flex(escaped)?;
                    } else if !self.cff2 {
                        self.arithmetic(escaped)?;
                    } else {
                        self.len = 0;
                    }
                }
                4 | 5 | 6 | 7 | 8 | 21 | 22 | 24..=27 | 30 | 31 => self.draw(op)?,
                15 if self.cff2 => {
                    if self.len != 1 {
                        return Err(FontError::InvalidTable);
                    }
                    let index = offset(self.pop()?)?;
                    self.blend.select(index)?;
                }
                16 if self.cff2 => self.blend.apply(&mut self.stack, &mut self.len)?,
                _ if self.cff2 => self.len = 0,
                _ => return Err(FontError::InvalidTable),
            }
        }
        if !self.cff2 {
            return Err(FontError::Truncated);
        }
        Ok(false)
    }

    fn draw(&mut self, op: u8) -> Result<(), FontError> {
        let z = Fixed::ZERO;
        match op {
            4 | 21 | 22 => {
                let expected = if op == 21 { 2 } else { 1 };
                self.width(expected)?;
                if self.len != expected {
                    return Err(FontError::InvalidTable);
                }
                let (x, y) = match op {
                    4 => (z, self.arg(0)?),
                    22 => (self.arg(0)?, z),
                    _ => (self.arg(0)?, self.arg(1)?),
                };
                self.close()?;
                self.position = self.position.relative(x, y)?;
                self.emit(Command::Move(self.position))?;
                self.open = true;
            }
            5 => {
                if self.len < 2 || !self.len.is_multiple_of(2) {
                    return Err(FontError::InvalidTable);
                }
                for i in (0..self.len).step_by(2) {
                    self.line(self.arg(i)?, self.arg(add(i, 1)?)?)?;
                }
            }
            6 | 7 => {
                if self.len == 0 {
                    return Err(FontError::InvalidTable);
                }
                for i in 0..self.len {
                    let v = self.arg(i)?;
                    if i.is_multiple_of(2) == (op == 6) {
                        self.line(v, z)?;
                    } else {
                        self.line(z, v)?;
                    }
                }
            }
            8 | 24 | 25 => self.mixed(op)?,
            26 | 27 => self.axis_curves(op)?,
            30 | 31 => self.alternate_curves(op)?,
            _ => return Err(FontError::InvalidTable),
        }
        self.len = 0;
        Ok(())
    }
    fn mixed(&mut self, op: u8) -> Result<(), FontError> {
        let (lines, curves, last_line) = match op {
            8 if self.len >= 6 && self.len.is_multiple_of(6) => (0, self.len / 6, false),
            24 if self.len >= 8 && sub(self.len, 2)?.is_multiple_of(6) => {
                (0, sub(self.len, 2)? / 6, true)
            }
            25 if self.len >= 8 && sub(self.len, 6)?.is_multiple_of(2) => {
                (sub(self.len, 6)? / 2, 1, false)
            }
            _ => return Err(FontError::InvalidTable),
        };
        let mut at = 0;
        for _ in 0..lines {
            self.line(self.arg(at)?, self.arg(add(at, 1)?)?)?;
            at = add(at, 2)?;
        }
        for _ in 0..curves {
            let d = self
                .stack
                .get(at..add(at, 6)?)
                .ok_or(FontError::InvalidTable)?
                .try_into()
                .map_err(|_| FontError::InvalidTable)?;
            self.curve(d)?;
            at = add(at, 6)?;
        }
        if last_line {
            self.line(self.arg(at)?, self.arg(add(at, 1)?)?)?;
        }
        Ok(())
    }
    fn axis_curves(&mut self, op: u8) -> Result<(), FontError> {
        if self.len < 4 || self.len % 4 > 1 {
            return Err(FontError::InvalidTable);
        }
        let mut at = self.len % 4;
        let mut first = if at == 1 { self.arg(0)? } else { Fixed::ZERO };
        while at < self.len {
            let a = self.arg(at)?;
            let b = self.arg(add(at, 1)?)?;
            let c = self.arg(add(at, 2)?)?;
            let d = self.arg(add(at, 3)?)?;
            self.curve(if op == 26 {
                [first, a, b, c, Fixed::ZERO, d]
            } else {
                [a, first, b, c, d, Fixed::ZERO]
            })?;
            first = Fixed::ZERO;
            at = add(at, 4)?;
        }
        Ok(())
    }
    #[expect(
        clippy::many_single_char_names,
        reason = "four curve operands in specification order"
    )]
    fn alternate_curves(&mut self, op: u8) -> Result<(), FontError> {
        if self.len < 4 || self.len % 4 > 1 {
            return Err(FontError::InvalidTable);
        }
        let mut at = 0;
        let mut horizontal = op == 31;
        while at < self.len {
            let a = self.arg(at)?;
            let b = self.arg(add(at, 1)?)?;
            let c = self.arg(add(at, 2)?)?;
            let d = self.arg(add(at, 3)?)?;
            at = add(at, 4)?;
            let last = if sub(self.len, at)? == 1 {
                let v = self.arg(at)?;
                at = add(at, 1)?;
                v
            } else {
                Fixed::ZERO
            };
            self.curve(if horizontal {
                [a, Fixed::ZERO, b, c, last, d]
            } else {
                [Fixed::ZERO, a, b, c, d, last]
            })?;
            horizontal = !horizontal;
        }
        Ok(())
    }
    fn flex(&mut self, op: u8) -> Result<(), FontError> {
        let expected = match op {
            34 => 7,
            35 => 13,
            36 => 9,
            37 => 11,
            _ => return Err(FontError::InvalidTable),
        };
        if self.len != expected {
            return Err(FontError::InvalidTable);
        }
        let z = Fixed::ZERO;
        let mut values = [z; 12];
        match op {
            34 => {
                values = [
                    self.arg(0)?,
                    z,
                    self.arg(1)?,
                    self.arg(2)?,
                    self.arg(3)?,
                    z,
                    self.arg(4)?,
                    z,
                    self.arg(5)?,
                    self.arg(2)?.checked_neg()?,
                    self.arg(6)?,
                    z,
                ];
            }
            35 => values.copy_from_slice(self.stack.get(..12).ok_or(FontError::InvalidTable)?),
            36 => {
                let last = self
                    .arg(1)?
                    .checked_add(self.arg(3)?)?
                    .checked_add(self.arg(7)?)?
                    .checked_neg()?;
                values = [
                    self.arg(0)?,
                    self.arg(1)?,
                    self.arg(2)?,
                    self.arg(3)?,
                    self.arg(4)?,
                    z,
                    self.arg(5)?,
                    z,
                    self.arg(6)?,
                    self.arg(7)?,
                    self.arg(8)?,
                    last,
                ];
            }
            37 => {
                values
                    .get_mut(..10)
                    .ok_or(FontError::InvalidTable)?
                    .copy_from_slice(self.stack.get(..10).ok_or(FontError::InvalidTable)?);
                let mut dx = z;
                let mut dy = z;
                for i in (0..10).step_by(2) {
                    dx = dx.checked_add(self.arg(i)?)?;
                    dy = dy.checked_add(self.arg(add(i, 1)?)?)?;
                }
                let (x, y) = if dx.bits().unsigned_abs() > dy.bits().unsigned_abs() {
                    (self.arg(10)?, dy.checked_neg()?)
                } else {
                    (dx.checked_neg()?, self.arg(10)?)
                };
                *values.get_mut(10).ok_or(FontError::InvalidTable)? = x;
                *values.get_mut(11).ok_or(FontError::InvalidTable)? = y;
            }
            _ => return Err(FontError::InvalidTable),
        }
        for chunk in values.as_chunks::<6>().0 {
            self.curve(*chunk)?;
        }
        self.len = 0;
        Ok(())
    }

    fn roll(&mut self) -> Result<(), FontError> {
        let shift = integer(self.pop()?)?;
        let count = offset(self.pop()?)?;
        if count != 0 {
            let n = i64::try_from(count).map_err(|_| FontError::Overflow)?;
            let rotation = usize::try_from(shift.rem_euclid(n)).map_err(|_| FontError::Overflow)?;
            self.stack
                .get_mut(sub(self.len, count)?..self.len)
                .ok_or(FontError::InvalidTable)?
                .rotate_right(rotation);
        }
        Ok(())
    }

    fn arithmetic(&mut self, op: u8) -> Result<(), FontError> {
        match op {
            0 => {
                if self.len != 0 {
                    return Err(FontError::InvalidTable);
                }
            }
            3 | 4 | 10 | 11 | 12 | 15 | 24 => {
                let b = self.pop()?;
                let a = self.pop()?;
                let v = match op {
                    3 => boolean(a != Fixed::ZERO && b != Fixed::ZERO),
                    4 => boolean(a != Fixed::ZERO || b != Fixed::ZERO),
                    10 => a.checked_add(b)?,
                    11 => a.checked_sub(b)?,
                    12 => a.checked_div(b)?,
                    15 => boolean(a == b),
                    _ => a.checked_mul(b)?,
                };
                self.push(v)?;
            }
            5 | 9 | 14 | 26 => {
                let a = self.pop()?;
                let v = match op {
                    5 => boolean(a == Fixed::ZERO),
                    9 => {
                        if a < Fixed::ZERO {
                            a.checked_neg()?
                        } else {
                            a
                        }
                    }
                    14 => a.checked_neg()?,
                    _ => sqrt(a)?,
                };
                self.push(v)?;
            }
            18 => {
                self.pop()?;
            }
            20 => {
                let i = offset(self.pop()?)?;
                let v = self.pop()?;
                *self.transient.get_mut(i).ok_or(FontError::InvalidTable)? = Some(v);
            }
            21 => {
                let i = offset(self.pop()?)?;
                let v = self
                    .transient
                    .get(i)
                    .copied()
                    .flatten()
                    .ok_or(FontError::InvalidTable)?;
                self.push(v)?;
            }
            22 => {
                let v2 = self.pop()?;
                let v1 = self.pop()?;
                let s2 = self.pop()?;
                let s1 = self.pop()?;
                self.push(if v1 <= v2 { s1 } else { s2 })?;
            }
            23 => {
                self.random ^= self.random << 13;
                self.random ^= self.random >> 17;
                self.random ^= self.random << 5;
                self.push(Fixed::from_bits(
                    i64::from(self.random)
                        .checked_add(1)
                        .ok_or(FontError::Overflow)?,
                ))?;
            }
            27 => {
                let a = self.pop()?;
                self.push(a)?;
                self.push(a)?;
            }
            28 => {
                let a = self.pop()?;
                let b = self.pop()?;
                self.push(a)?;
                self.push(b)?;
            }
            29 => {
                let i = usize::try_from(integer(self.pop()?)?.max(0))
                    .map_err(|_| FontError::InvalidTable)?;
                let a = self.arg(sub(sub(self.len, 1)?, i)?)?;
                self.push(a)?;
            }
            30 => self.roll()?,
            _ => return Err(FontError::InvalidTable),
        }
        Ok(())
    }
}
fn boolean(value: bool) -> Fixed {
    Fixed::from_i32(i32::from(value))
}

fn sqrt(value: Fixed) -> Result<Fixed, FontError> {
    let n = u128::try_from(value.bits()).map_err(|_| FontError::InvalidTable)? << 32;
    if n == 0 {
        return Ok(Fixed::ZERO);
    }
    let mut x = n;
    let mut y = n.checked_add(1).ok_or(FontError::Overflow)? / 2;
    while y < x {
        x = y;
        y = x
            .checked_add(n.checked_div(x).ok_or(FontError::InvalidTable)?)
            .ok_or(FontError::Overflow)?
            / 2;
    }
    let low = n
        .checked_sub(x.checked_mul(x).ok_or(FontError::Overflow)?)
        .ok_or(FontError::Overflow)?;
    let upper = x.checked_add(1).ok_or(FontError::Overflow)?;
    let high = upper
        .checked_mul(upper)
        .and_then(|v| v.checked_sub(n))
        .ok_or(FontError::Overflow)?;
    let x = if low > high || (low == high && x & 1 != 0) {
        upper
    } else {
        x
    };
    Ok(Fixed::from_bits(
        i64::try_from(x).map_err(|_| FontError::Overflow)?,
    ))
}
