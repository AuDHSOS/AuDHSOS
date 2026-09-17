// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{B, TextError, Unit, add, at, at_mut, isolate, sub};
use crate::unicode::{BracketKind, paired_bracket};

fn kind(units: &[Unit], seq: &[usize], pos: usize) -> Result<B, TextError> {
    Ok(at(units, *seq.get(pos).ok_or(TextError::BufferTooSmall)?)?.kind)
}
fn set(units: &mut [Unit], seq: &[usize], pos: usize, value: B) -> Result<(), TextError> {
    at_mut(units, *seq.get(pos).ok_or(TextError::BufferTooSmall)?)?.kind = value;
    Ok(())
}
const fn strong(kind: B) -> Option<B> {
    match kind {
        B::L => Some(B::L),
        B::R | B::En | B::An => Some(B::R),
        _ => None,
    }
}

pub(super) fn resolve(
    units: &mut [Unit],
    seq: &[usize],
    sos: B,
    eos: B,
    embedding: B,
) -> Result<(), TextError> {
    weak(units, seq, sos)?;
    brackets(units, seq, sos, embedding)?;
    neutral(units, seq, sos, eos, embedding)?;
    for i in seq {
        let u = at_mut(units, *i)?;
        let increment = if u.embedding & 1 == 0 {
            match u.kind {
                B::R => 1,
                B::En | B::An => 2,
                _ => 0,
            }
        } else {
            u8::from(u.kind != B::R)
        };
        u.level = u
            .embedding
            .checked_add(increment)
            .ok_or(TextError::Overflow)?;
    }
    Ok(())
}
fn weak(units: &mut [Unit], seq: &[usize], sos: B) -> Result<(), TextError> {
    let mut previous = sos;
    // W1.
    for i in seq {
        let u = at_mut(units, *i)?;
        if u.kind == B::Nsm {
            u.kind = if isolate(previous) || previous == B::Pdi {
                B::On
            } else {
                previous
            };
        }
        previous = u.kind;
    }
    // W2/W3 retain the last original strong type until the next one.
    let mut last_strong = sos;
    for i in seq {
        let u = at_mut(units, *i)?;
        if matches!(u.kind, B::L | B::R | B::Al) {
            last_strong = u.kind;
        }
        if u.kind == B::En && last_strong == B::Al {
            u.kind = B::An;
        }
        if u.kind == B::Al {
            u.kind = B::R;
        }
    }
    // W4.
    for triple in seq.windows(3) {
        let triple: [usize; 3] = triple.try_into().map_err(|_| TextError::Overflow)?;
        let a = at(units, triple[0])?.kind;
        let b = at(units, triple[1])?.kind;
        let c = at(units, triple[2])?.kind;
        if a == c && ((b == B::Es && a == B::En) || (b == B::Cs && matches!(a, B::En | B::An))) {
            at_mut(units, triple[1])?.kind = a;
        }
    }
    // W5.
    let mut i = 0;
    while i < seq.len() {
        if kind(units, seq, i)? != B::Et {
            i = add(i, 1)?;
            continue;
        }
        let start = i;
        while i < seq.len() && kind(units, seq, i)? == B::Et {
            i = add(i, 1)?;
        }
        let before = if start == 0 {
            sos
        } else {
            kind(units, seq, sub(start, 1)?)?
        };
        let after = if i == seq.len() {
            B::On
        } else {
            kind(units, seq, i)?
        };
        if before == B::En || after == B::En {
            for pos in start..i {
                set(units, seq, pos, B::En)?;
            }
        }
    }
    // W6/W7.
    last_strong = sos;
    for i in seq {
        let u = at_mut(units, *i)?;
        if matches!(u.kind, B::Es | B::Et | B::Cs) {
            u.kind = B::On;
        }
        if matches!(u.kind, B::L | B::R) {
            last_strong = u.kind;
        }
        if u.kind == B::En && last_strong == B::L {
            u.kind = B::L;
        }
    }
    Ok(())
}
const fn bracket_code(code: u32) -> u32 {
    if code == 0x232a { 0x3009 } else { code }
}
fn find_pairs(units: &mut [Unit], seq: &[usize]) -> Result<bool, TextError> {
    let mut stack = [(0_u32, 0_usize); 63];
    let mut len = 0;
    for (pos, i) in seq.iter().enumerate() {
        let unit = *at(units, *i)?;
        if unit.kind != B::On {
            continue;
        }
        if let Some((pair, kind)) = paired_bracket(unit.code) {
            if kind == BracketKind::Open {
                if len == 63 {
                    return Ok(false);
                }
                *stack.get_mut(len).ok_or(TextError::Overflow)? = (bracket_code(pair), pos);
                len = add(len, 1)?;
            } else {
                let found = stack
                    .get(..len)
                    .ok_or(TextError::Overflow)?
                    .iter()
                    .rposition(|(code, _)| *code == bracket_code(unit.code));
                if let Some(index) = found {
                    let (_, open) = *stack.get(index).ok_or(TextError::Overflow)?;
                    at_mut(units, *seq.get(open).ok_or(TextError::BufferTooSmall)?)?.bracket =
                        Some(pos);
                    len = index;
                }
            }
        }
    }
    Ok(true)
}
fn brackets(units: &mut [Unit], seq: &[usize], sos: B, embedding: B) -> Result<(), TextError> {
    if !find_pairs(units, seq)? {
        return Ok(());
    }
    for (open, i) in seq.iter().enumerate() {
        let Some(close) = at(units, *i)?.bracket else {
            continue;
        };
        let mut matching = false;
        let mut opposite = false;
        for pos in add(open, 1)?..close {
            if let Some(s) = strong(kind(units, seq, pos)?) {
                matching |= s == embedding;
                opposite |= s != embedding;
            }
        }
        let resolved = if matching {
            Some(embedding)
        } else if opposite {
            let mut context = sos;
            for pos in (0..open).rev() {
                if let Some(s) = strong(kind(units, seq, pos)?) {
                    context = s;
                    break;
                }
            }
            Some(context)
        } else {
            None
        };
        if let Some(resolved) = resolved {
            for pos in [open, close] {
                set(units, seq, pos, resolved)?;
                let mut after = add(pos, 1)?;
                while after < seq.len()
                    && at(units, *seq.get(after).ok_or(TextError::BufferTooSmall)?)?.original
                        == B::Nsm
                {
                    set(units, seq, after, resolved)?;
                    after = add(after, 1)?;
                }
            }
        }
    }
    Ok(())
}
fn neutral(
    units: &mut [Unit],
    seq: &[usize],
    sos: B,
    eos: B,
    embedding: B,
) -> Result<(), TextError> {
    let mut previous = sos;
    let mut i = 0;
    while i < seq.len() {
        if let Some(s) = strong(kind(units, seq, i)?) {
            previous = s;
            i = add(i, 1)?;
            continue;
        }
        let start = i;
        while i < seq.len() && strong(kind(units, seq, i)?).is_none() {
            i = add(i, 1)?;
        }
        let after = if i == seq.len() {
            eos
        } else {
            strong(kind(units, seq, i)?).ok_or(TextError::Overflow)?
        };
        let value = if previous == after {
            previous
        } else {
            embedding
        };
        for pos in start..i {
            set(units, seq, pos, value)?;
        }
    }
    Ok(())
}
