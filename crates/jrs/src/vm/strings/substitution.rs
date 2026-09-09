// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Shared `GetSubstitution` for literal and `RegExp` replacements. Named capture
//! access stays in the VM; scanning and emitted code units consume shared fuel.
use super::{Error, Execution, Value, Vec};

#[derive(Clone, Copy)]
pub(in crate::vm) struct Input<'a> {
    pub(in crate::vm) matched: &'a [u16],
    pub(in crate::vm) text: &'a [u16],
    pub(in crate::vm) position: usize,
    pub(in crate::vm) captures: &'a [Value],
    pub(in crate::vm) named: &'a Value,
}
impl Execution<'_> {
    pub(in crate::vm) fn get_substitution(
        &mut self,
        template: &[u16],
        input: Input<'_>,
    ) -> Result<Vec<u16>, Error> {
        let mut out = Vec::new();
        let mut at = 0usize;
        // An absent closing '>' never needs rescanning the remaining suffix.
        let mut no_closing = false;
        while let Some(unit) = template.get(at) {
            self.charge(1)?;
            let next = template.get(at.saturating_add(1)).copied();
            let mut width = 1usize;
            let owned;
            let piece = if *unit == 36 {
                match next {
                    Some(36) => {
                        width = 2;
                        Some(&[36][..])
                    }
                    Some(38) => {
                        width = 2;
                        Some(input.matched)
                    }
                    Some(96) => {
                        width = 2;
                        Some(
                            input
                                .text
                                .get(..input.position)
                                .ok_or(Error::InvalidBytecode)?,
                        )
                    }
                    Some(39) => {
                        width = 2;
                        Some(
                            input
                                .text
                                .get(
                                    input
                                        .position
                                        .saturating_add(input.matched.len())
                                        .min(input.text.len())..,
                                )
                                .ok_or(Error::InvalidBytecode)?,
                        )
                    }
                    Some(digit @ 48..=57) => {
                        let mut index = usize::from(digit.saturating_sub(48));
                        let mut consumed = 2;
                        if let Some(second @ 48..=57) = template.get(at.saturating_add(2)) {
                            let two = index
                                .saturating_mul(10)
                                .saturating_add(usize::from(second.saturating_sub(48)));
                            if two > 0 && two <= input.captures.len() {
                                index = two;
                                consumed = 3;
                            }
                        }
                        if index > 0 && index <= input.captures.len() {
                            width = consumed;
                            match input.captures.get(index.saturating_sub(1)) {
                                Some(Value::String(s)) => Some(s.as_ref()),
                                Some(Value::Undefined) => Some(&[][..]),
                                _ => return Err(Error::InvalidBytecode),
                            }
                        } else {
                            None
                        }
                    }
                    Some(60) if !matches!(input.named, Value::Undefined) && !no_closing => {
                        let mut end = at.saturating_add(2);
                        while template.get(end).is_some_and(|u| *u != 62) {
                            self.charge(1)?;
                            end = end.saturating_add(1);
                        }
                        if template.get(end).is_some() {
                            width = end.saturating_sub(at).saturating_add(1);
                            let name = template
                                .get(at.saturating_add(2)..end)
                                .ok_or(Error::InvalidBytecode)?;
                            let value = self.get(input.named, name)?;
                            owned = if matches!(value, Value::Undefined) {
                                Value::string("").units()
                            } else {
                                self.string_units(&value)?
                            };
                            Some(owned.as_ref())
                        } else {
                            no_closing = true;
                            None
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };
            self.append_string(&mut out, piece.unwrap_or(core::slice::from_ref(unit)))?;
            at = at.saturating_add(width);
        }
        Ok(out)
    }
}
