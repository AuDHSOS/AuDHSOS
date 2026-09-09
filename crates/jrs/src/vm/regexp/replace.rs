// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Two-phase `RegExp` replacement: retain actual exec result identities, then
//! read mutable captures and run replacers. No matcher fallback for custom hooks.
use super::{Error, Execution, Rc, Value, Vec, intrinsics::require_object};
use crate::vm::strings::substitution::Input;

impl Execution<'_> {
    pub(super) fn regexp_symbol_replace(
        &mut self,
        receiver: &Value,
        input: &Value,
        replacement: &Value,
    ) -> Result<Value, Error> {
        require_object(receiver)?;
        let text = self.string_units(input)?;
        let template = if matches!(replacement, Value::Function(_)) {
            None
        } else {
            Some(self.string_units(replacement)?)
        };
        let results = self.replace_collect(receiver, &text)?;
        let mut output = Vec::new();
        let mut copied = 0usize;
        for result in results {
            self.charge(1)?;
            let roots = self.native_roots.len();
            let step = (|| {
                let (matched, position, substitution) =
                    self.replace_result(&result, &text, replacement, template.as_deref())?;
                // Even backwards/overlapping results execute all conversion and
                // replacement hooks, but their resulting text is discarded.
                if position >= copied {
                    self.append_string(
                        &mut output,
                        text.get(copied..position).ok_or(Error::InvalidBytecode)?,
                    )?;
                    self.append_string(&mut output, &substitution)?;
                    copied = position.saturating_add(matched);
                }
                Ok(())
            })();
            self.native_roots.truncate(roots);
            step?;
        }
        if copied < text.len() {
            self.append_string(
                &mut output,
                text.get(copied..).ok_or(Error::InvalidBytecode)?,
            )?;
        }
        Ok(Value::String(output.into()))
    }
    fn replace_collect(&mut self, receiver: &Value, text: &Rc<[u16]>) -> Result<Vec<Value>, Error> {
        let flags = self.get(receiver, &Value::string("flags").units())?;
        let flags = self.string_units(&flags)?;
        self.charge(u64::try_from(flags.len()).unwrap_or(u64::MAX))?;
        let global = flags.contains(&103);
        let unicode = flags.contains(&117) || flags.contains(&118);
        let key = Value::string("lastIndex").units();
        if global {
            self.set(receiver, key.clone(), Value::Number(0.0), true)?;
        }
        let mut results = Vec::new();
        loop {
            self.charge(1)?;
            let result = self.regexp_exec_value(receiver, text)?;
            if matches!(result, Value::Null) {
                break;
            }
            if results.len() >= self.limits.properties {
                return Err(Error::Limit {
                    resource: "replace matches",
                });
            }
            self.native_roots.push(result.clone());
            results.push(result.clone());
            if !global {
                break;
            }
            let matched = self.get(&result, &Value::string("0").units())?;
            if self.string_units(&matched)?.is_empty() {
                let index = self.get(receiver, &key)?;
                let index = crate::value::integer_or_infinity(self.numeric(&index)?)
                    .clamp(0.0, 9_007_199_254_740_991.0);
                let next = super::dispatch::advance(text, index, unicode);
                self.set(receiver, key.clone(), Value::Number(next), true)?;
            }
        }
        Ok(results)
    }
    fn replace_result(
        &mut self,
        result: &Value,
        text: &Rc<[u16]>,
        replacement: &Value,
        template: Option<&[u16]>,
    ) -> Result<(usize, usize, Vec<u16>), Error> {
        let length = self.get(result, &Value::string("length").units())?;
        let count = crate::value::integer_or_infinity(self.numeric(&length)?)
            .clamp(0.0, 9_007_199_254_740_991.0)
            - 1.0;
        let matched = self.get(result, &Value::string("0").units())?;
        let matched = self.string_units(&matched)?;
        let position = self.get(result, &Value::string("index").units())?;
        let position = bounded_position(
            crate::value::integer_or_infinity(self.numeric(&position)?),
            text.len(),
        );
        let mut captures = Vec::new();
        let mut index = 1u32;
        while f64::from(index) <= count {
            self.charge(1)?;
            if captures.len() >= self.limits.properties {
                return Err(Error::Limit {
                    resource: "replacement captures",
                });
            }
            let capture = self.get(result, &Value::string(&alloc::format!("{index}")).units())?;
            captures.push(if matches!(capture, Value::Undefined) {
                capture
            } else {
                Value::String(self.string_units(&capture)?)
            });
            index = index.checked_add(1).ok_or(Error::Limit {
                resource: "replacement captures",
            })?;
        }
        let named = self.get(result, &Value::string("groups").units())?;
        self.native_roots.push(named.clone());
        let substitution = if let Some(template) = template {
            let named = if matches!(named, Value::Undefined) {
                named
            } else {
                self.box_value(&named)?
            };
            self.native_roots.push(named.clone());
            self.get_substitution(
                template,
                Input {
                    matched: &matched,
                    text,
                    position,
                    captures: &captures,
                    named: &named,
                },
            )?
        } else {
            let size = captures
                .len()
                .saturating_add(3)
                .saturating_add(usize::from(!matches!(named, Value::Undefined)));
            if size.saturating_add(2) > self.limits.stack {
                return Err(Error::Limit {
                    resource: "replacement arguments",
                });
            }
            let mut args = Vec::with_capacity(size);
            args.push(Value::String(matched.clone()));
            args.extend(captures);
            args.push(super::number(position));
            args.push(Value::String(text.clone()));
            if !matches!(named, Value::Undefined) {
                args.push(named);
            }
            let value = self.call_sync(replacement.clone(), Value::Undefined, &args)?;
            self.string_units(&value)?.to_vec()
        };
        Ok((matched.len(), position, substitution))
    }
}
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "ToIntegerOrInfinity saturates to usize range, then clamps to the bounded input length"
)]
fn bounded_position(n: f64, length: usize) -> usize {
    (n as usize).min(length)
}
