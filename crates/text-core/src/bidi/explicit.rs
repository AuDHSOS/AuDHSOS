// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{B, TextError, Unit, add, at, at_mut, isolate, sub};

pub(super) fn matching(units: &mut [Unit]) -> Result<(), TextError> {
    let mut pending = None;
    for i in 0..units.len() {
        let original = at(units, i)?.original;
        if isolate(original) {
            at_mut(units, i)?.parent = pending;
            pending = Some(i);
        } else if original == B::Pdi
            && let Some(open) = pending
        {
            at_mut(units, i)?.matching = Some(open);
            at_mut(units, open)?.matching = Some(i);
            pending = at(units, open)?.parent;
        }
    }
    Ok(())
}
pub(super) fn first_strong(units: &[Unit], mut i: usize, end: usize) -> Result<u8, TextError> {
    while i < end {
        let u = at(units, i)?;
        match u.original {
            B::L => return Ok(0),
            B::R | B::Al => return Ok(1),
            k if isolate(k) => i = u.matching.unwrap_or(end),
            _ => (),
        }
        i = add(i, 1)?;
    }
    Ok(0)
}
#[derive(Clone, Copy)]
struct Status {
    level: u8,
    override_kind: Option<B>,
    isolate: bool,
}
struct Stack {
    values: [Status; 126],
    top: usize,
    overflow_isolate: usize,
    overflow_embedding: usize,
    valid_isolate: usize,
}
impl Stack {
    fn current(&self) -> Result<Status, TextError> {
        self.values
            .get(self.top)
            .copied()
            .ok_or(TextError::Overflow)
    }
    fn push(&mut self, status: Status) -> Result<(), TextError> {
        self.top = add(self.top, 1)?;
        *self.values.get_mut(self.top).ok_or(TextError::Overflow)? = status;
        Ok(())
    }
    fn pop(&mut self) -> Result<(), TextError> {
        self.top = sub(self.top, 1)?;
        Ok(())
    }
    fn initiate(&mut self, kind: B) -> Result<(), TextError> {
        let isolating = isolate(kind);
        let odd = matches!(kind, B::Rle | B::Rlo | B::Rli);
        let old = self.current()?.level;
        let level = old
            .checked_add(if odd { 1 } else { 2 })
            .ok_or(TextError::Overflow)?;
        let level = if odd { level | 1 } else { level & !1 };
        if level <= 125 && self.overflow_isolate == 0 && self.overflow_embedding == 0 {
            let override_kind = match kind {
                B::Lro => Some(B::L),
                B::Rlo => Some(B::R),
                _ => None,
            };
            self.push(Status {
                level,
                override_kind,
                isolate: isolating,
            })?;
            if isolating {
                self.valid_isolate = add(self.valid_isolate, 1)?;
            }
        } else if isolating {
            self.overflow_isolate = add(self.overflow_isolate, 1)?;
        } else if self.overflow_isolate == 0 {
            self.overflow_embedding = add(self.overflow_embedding, 1)?;
        }
        Ok(())
    }
    fn pdi(&mut self) -> Result<(), TextError> {
        if self.overflow_isolate > 0 {
            self.overflow_isolate = sub(self.overflow_isolate, 1)?;
        } else if self.valid_isolate > 0 {
            self.overflow_embedding = 0;
            while !self.current()?.isolate {
                self.pop()?;
            }
            self.pop()?;
            self.valid_isolate = sub(self.valid_isolate, 1)?;
        }
        Ok(())
    }
    fn pdf(&mut self) -> Result<(), TextError> {
        if self.overflow_isolate == 0 {
            if self.overflow_embedding > 0 {
                self.overflow_embedding = sub(self.overflow_embedding, 1)?;
            } else if self.top > 0 && !self.current()?.isolate {
                self.pop()?;
            }
        }
        Ok(())
    }
}
pub(super) fn levels(units: &mut [Unit], base: u8) -> Result<(), TextError> {
    let mut stack = Stack {
        values: [Status {
            level: base,
            override_kind: None,
            isolate: false,
        }; 126],
        top: 0,
        overflow_isolate: 0,
        overflow_embedding: 0,
        valid_isolate: 0,
    };
    for i in 0..units.len() {
        let original = at(units, i)?.original;
        let status = stack.current()?;
        {
            let u = at_mut(units, i)?;
            u.embedding = status.level;
            u.base = base;
        }
        match original {
            B::Rle | B::Lre | B::Rlo | B::Lro => stack.initiate(original)?,
            B::Rli | B::Lri | B::Fsi => {
                at_mut(units, i)?.kind = status.override_kind.unwrap_or(original);
                let kind = if original == B::Fsi {
                    if first_strong(
                        units,
                        add(i, 1)?,
                        at(units, i)?.matching.unwrap_or(units.len()),
                    )? == 1
                    {
                        B::Rli
                    } else {
                        B::Lri
                    }
                } else {
                    original
                };
                stack.initiate(kind)?;
            }
            B::Pdi => {
                stack.pdi()?;
                let status = stack.current()?;
                let u = at_mut(units, i)?;
                u.embedding = status.level;
                u.kind = status.override_kind.unwrap_or(original);
            }
            B::Pdf => stack.pdf()?,
            B::B => at_mut(units, i)?.embedding = base,
            B::Bn => (),
            _ => at_mut(units, i)?.kind = status.override_kind.unwrap_or(original),
        }
        let u = at_mut(units, i)?;
        u.level = u.embedding;
    }
    Ok(())
}
