// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    TextError,
    unicode::{self as u, GeneralCategory as G, LineBreak as L},
};

/// Whether a line may or must end at a byte boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Break {
    /// No line break at this position.
    #[default]
    Prohibited,
    /// Optional line break opportunity.
    Allowed,
    /// Required line break, including end of text.
    Mandatory,
}
/// A UTF-8 byte boundary and its line break classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineBoundary {
    /// Byte offset in the original text.
    pub byte: usize,
    /// Line break classification before this byte.
    pub kind: Break,
}
/// Caller-owned scratch record; provision one per Unicode scalar.
#[derive(Clone, Copy, Debug)]
pub struct LineUnit {
    byte: usize,
    code: u32,
    raw: L,
    class: L,
    base: usize,
    previous: Option<usize>,
    next: Option<usize>,
    nonspace: Option<usize>,
    ri_odd: bool,
    numeric: bool,
}
impl Default for LineUnit {
    fn default() -> Self {
        Self {
            byte: 0,
            code: 0,
            raw: L::Xx,
            class: L::Xx,
            base: 0,
            previous: None,
            next: None,
            nonspace: None,
            ri_odd: false,
            numeric: false,
        }
    }
}
fn get(units: &[LineUnit], index: Option<usize>) -> Option<&LineUnit> {
    index.and_then(|i| units.get(i))
}
fn class(unit: Option<&LineUnit>) -> Option<L> {
    unit.map(|u| u.class)
}
const fn hard(value: L) -> bool {
    matches!(value, L::Bk | L::Cr | L::Lf | L::Nl)
}
fn east_asian(unit: &LineUnit) -> bool {
    matches!(
        u::east_asian_width(unit.code),
        u::EastAsianWidth::F | u::EastAsianWidth::W | u::EastAsianWidth::H
    )
}
const fn alpha(value: L) -> bool {
    matches!(value, L::Al | L::Hl)
}
const fn hangul(value: L) -> bool {
    matches!(value, L::Jl | L::Jv | L::Jt | L::H2 | L::H3)
}
const fn aksara(unit: &LineUnit) -> bool {
    matches!(unit.class, L::Ak | L::As) || unit.code == 0x25cc
}
fn resolve(code: u32) -> L {
    match u::line_break(code) {
        L::Ai | L::Sg | L::Xx => L::Al,
        L::Sa => {
            if matches!(u::general_category(code), G::Mn | G::Mc) {
                L::Cm
            } else {
                L::Al
            }
        }
        L::Cj => L::Ns,
        value => value,
    }
}
fn prepare(text: &str, units: &mut [LineUnit]) -> Result<usize, TextError> {
    let count = text.chars().count();
    let units = units.get_mut(..count).ok_or(TextError::BufferTooSmall)?;
    let mut previous = None;
    for (i, (byte, ch)) in text.char_indices().enumerate() {
        let code = u32::from(ch);
        let raw = resolve(code);
        let before = get(units, previous).copied();
        let combining = matches!(raw, L::Cm | L::Zwj);
        let attached = combining
            && before.is_some_and(|b| !hard(b.class) && !matches!(b.class, L::Sp | L::Zw));
        let class = if combining { L::Al } else { raw };
        let unit = LineUnit {
            byte,
            code: if combining && !attached { 0x41 } else { code },
            raw,
            class,
            base: if attached {
                previous.ok_or(TextError::Overflow)?
            } else {
                i
            },
            previous,
            next: None,
            nonspace: if class == L::Sp {
                before.and_then(|b| b.nonspace)
            } else {
                Some(i)
            },
            ri_odd: class == L::Ri && before.is_none_or(|b| b.class != L::Ri || !b.ri_odd),
            numeric: class == L::Nu
                || (matches!(class, L::Sy | L::Is) && before.is_some_and(|b| b.numeric)),
        };
        *units.get_mut(i).ok_or(TextError::BufferTooSmall)? = unit;
        if !attached {
            if let Some(p) = previous {
                units.get_mut(p).ok_or(TextError::BufferTooSmall)?.next = Some(i);
            }
            previous = Some(i);
        }
    }
    Ok(count)
}

/// Default UAX #14 boundaries, in O(N log R) time and O(N) caller storage.
///
/// Scratch needs one record per scalar; output needs one more. Empty text
/// has one mandatory end boundary. Errors invalidate output and scratch.
/// # Errors
/// Returns capacity or index overflow errors.
pub fn line_breaks(
    text: &str,
    workspace: &mut [LineUnit],
    output: &mut [LineBoundary],
) -> Result<usize, TextError> {
    let count = prepare(text, workspace)?;
    let len = count.checked_add(1).ok_or(TextError::Overflow)?;
    let out = output.get_mut(..len).ok_or(TextError::BufferTooSmall)?;
    let units = workspace.get(..count).ok_or(TextError::BufferTooSmall)?;
    for (i, boundary) in out.iter_mut().enumerate() {
        *boundary = if i == count {
            LineBoundary {
                byte: text.len(),
                kind: Break::Mandatory,
            }
        } else if i == 0 {
            LineBoundary {
                byte: 0,
                kind: Break::Prohibited,
            }
        } else {
            let right = units.get(i).ok_or(TextError::BufferTooSmall)?;
            let left = units
                .get(i.checked_sub(1).ok_or(TextError::Overflow)?)
                .ok_or(TextError::BufferTooSmall)?;
            LineBoundary {
                byte: right.byte,
                kind: at_boundary(units, left, right, i)?,
            }
        };
    }
    Ok(len)
}
fn at_boundary(
    units: &[LineUnit],
    raw_left: &LineUnit,
    right: &LineUnit,
    i: usize,
) -> Result<Break, TextError> {
    // UAX #14 rev.57, LB4–LB10.
    if raw_left.raw == L::Cr && right.raw == L::Lf {
        return Ok(Break::Prohibited);
    }
    if hard(raw_left.raw) {
        return Ok(Break::Mandatory);
    }
    if hard(right.raw) || matches!(right.raw, L::Sp | L::Zw) {
        return Ok(Break::Prohibited);
    }
    let left = units.get(raw_left.base).ok_or(TextError::BufferTooSmall)?;
    let nonspace = get(units, left.nonspace);
    if class(nonspace) == Some(L::Zw) {
        return Ok(Break::Allowed);
    }
    if raw_left.raw == L::Zwj || right.base != i {
        return Ok(Break::Prohibited);
    }
    Ok(rules(units, left, right, nonspace))
}
fn rules(
    units: &[LineUnit],
    left: &LineUnit,
    right: &LineUnit,
    nonspace: Option<&LineUnit>,
) -> Break {
    let (a, b) = (left.class, right.class);
    // LB11–LB14.
    if a == L::Wj
        || b == L::Wj
        || a == L::Gl
        || (b == L::Gl && !matches!(a, L::Sp | L::Hy | L::Hh))
        || matches!(b, L::Cl | L::Cp | L::Ex | L::Sy)
        || class(nonspace) == Some(L::Op)
    {
        return Break::Prohibited;
    }
    let before = get(units, left.previous);
    let after = get(units, right.next);
    // LB15a–LB17, before the LB18 space break.
    if initial_quote(units, nonspace)
        || (b == L::Qu
            && u::general_category(right.code) == G::Pf
            && after.is_none_or(|n| {
                matches!(
                    n.class,
                    L::Sp
                        | L::Gl
                        | L::Wj
                        | L::Cl
                        | L::Qu
                        | L::Cp
                        | L::Ex
                        | L::Is
                        | L::Sy
                        | L::Bk
                        | L::Cr
                        | L::Lf
                        | L::Nl
                        | L::Zw
                )
            }))
    {
        return Break::Prohibited;
    }
    if a == L::Sp && b == L::Is && class(after) == Some(L::Nu) {
        return Break::Allowed;
    }
    if b == L::Is
        || (b == L::Ns && matches!(class(nonspace), Some(L::Cl | L::Cp)))
        || (b == L::B2 && class(nonspace) == Some(L::B2))
    {
        return Break::Prohibited;
    }
    if a == L::Sp {
        return Break::Allowed;
    }
    // LB19/LB19a.
    if (b == L::Qu
        && (u::general_category(right.code) != G::Pi
            || !east_asian(left)
            || after.is_none_or(|u| !east_asian(u))))
        || (a == L::Qu
            && (u::general_category(left.code) != G::Pf
                || !east_asian(right)
                || before.is_none_or(|u| !east_asian(u))))
    {
        return Break::Prohibited;
    }
    if a == L::Cb || b == L::Cb {
        return Break::Allowed;
    }
    if words_and_numbers(units, left, right, before, after) {
        return Break::Prohibited;
    }
    // LB26–LB30b.
    if (a == L::Jl && matches!(b, L::Jl | L::Jv | L::H2 | L::H3))
        || (matches!(a, L::Jv | L::H2) && matches!(b, L::Jv | L::Jt))
        || (matches!(a, L::Jt | L::H3) && b == L::Jt)
        || (hangul(a) && b == L::Po)
        || (a == L::Pr && hangul(b))
        || (alpha(a) && alpha(b))
        || brahmic(left, right, before, after)
        || (a == L::Is && alpha(b))
        || ((alpha(a) || a == L::Nu) && b == L::Op && !east_asian(right))
        || (a == L::Cp && !east_asian(left) && (alpha(b) || b == L::Nu))
        || (a == L::Ri && b == L::Ri && left.ri_odd)
        || (b == L::Em
            && (a == L::Eb
                || (u::extended_pictographic(left.code) == u::ExtendedPictographic::Yes
                    && u::general_category(left.code) == G::Cn)))
    {
        return Break::Prohibited;
    }
    Break::Allowed
}
fn initial_quote(units: &[LineUnit], nonspace: Option<&LineUnit>) -> bool {
    nonspace.is_some_and(|q| {
        q.class == L::Qu
            && u::general_category(q.code) == G::Pi
            && get(units, q.previous).is_none_or(|p| {
                matches!(
                    p.class,
                    L::Bk | L::Cr | L::Lf | L::Nl | L::Op | L::Qu | L::Gl | L::Sp | L::Zw
                )
            })
    })
}
fn words_and_numbers(
    units: &[LineUnit],
    left: &LineUnit,
    right: &LineUnit,
    before: Option<&LineUnit>,
    after: Option<&LineUnit>,
) -> bool {
    let (a, b) = (left.class, right.class);
    // LB20a–LB24.
    if (matches!(a, L::Hy | L::Hh)
        && alpha(b)
        && before.is_none_or(|p| {
            matches!(
                p.class,
                L::Bk | L::Cr | L::Lf | L::Nl | L::Sp | L::Zw | L::Cb | L::Gl
            )
        }))
        || matches!(b, L::Ba | L::Hh | L::Hy | L::Ns)
        || a == L::Bb
        || (class(before) == Some(L::Hl) && matches!(a, L::Hy | L::Hh) && b != L::Hl)
        || (a == L::Sy && b == L::Hl)
        || b == L::In
        || (alpha(a) && b == L::Nu)
        || (a == L::Nu && alpha(b))
        || (a == L::Pr && matches!(b, L::Id | L::Eb | L::Em))
        || (matches!(a, L::Id | L::Eb | L::Em) && b == L::Po)
        || (matches!(a, L::Pr | L::Po) && alpha(b))
        || (alpha(a) && matches!(b, L::Pr | L::Po))
    {
        return true;
    }
    // LB25; numeric tracks NU (SY|IS)* with ignored combining marks removed.
    (matches!(b, L::Po | L::Pr)
        && (left.numeric || (matches!(a, L::Cl | L::Cp) && before.is_some_and(|p| p.numeric))))
        || (matches!(a, L::Po | L::Pr)
            && (b == L::Nu
                || (b == L::Op
                    && (class(after) == Some(L::Nu)
                        || (class(after) == Some(L::Is)
                            && class(after.and_then(|n| get(units, n.next))) == Some(L::Nu))))))
        || (matches!(a, L::Hy | L::Is) && b == L::Nu)
        || (left.numeric && b == L::Nu)
}
fn brahmic(
    left: &LineUnit,
    right: &LineUnit,
    before: Option<&LineUnit>,
    after: Option<&LineUnit>,
) -> bool {
    (left.class == L::Ap && aksara(right))
        || (aksara(left) && matches!(right.class, L::Vf | L::Vi))
        || (before.is_some_and(aksara)
            && left.class == L::Vi
            && (right.class == L::Ak || right.code == 0x25cc))
        || (aksara(left) && aksara(right) && class(after) == Some(L::Vf))
}
