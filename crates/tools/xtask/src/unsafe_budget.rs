// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Counting `unsafe` and inline-assembly sites per crate against the
//! budgets in the policy.

use std::path::Path;

use crate::error::Error;
use crate::fs;
use crate::policy::{CRATES, Kind};

/// Counts of the sites in a piece of source code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// `unsafe` keywords: blocks, functions, impls, traits.
    pub(crate) unsafe_keywords: u32,
    /// `asm!` and `naked_asm!` invocations.
    pub(crate) asm_macros: u32,
    /// `global_asm!` invocations.
    pub(crate) global_asm_macros: u32,
}

impl Counts {
    const fn add(&mut self, other: Counts) {
        self.unsafe_keywords = self.unsafe_keywords.saturating_add(other.unsafe_keywords);
        self.asm_macros = self.asm_macros.saturating_add(other.asm_macros);
        self.global_asm_macros = self
            .global_asm_macros
            .saturating_add(other.global_asm_macros);
    }
}

/// Counts the sites in `source`, ignoring comments and string literals.
pub(crate) fn count(source: &str) -> Counts {
    let code = strip_comments_and_strings(source);
    let mut counts = Counts::default();
    let mut rest = code.as_str();
    while let Some(word_start) = rest.find(|c: char| c.is_alphanumeric() || c == '_') {
        let after = rest.get(word_start..).unwrap_or("");
        let word_len = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let word = after.get(..word_len).unwrap_or("");
        let tail = after.get(word_len..).unwrap_or("");
        let is_macro = tail.trim_start().starts_with('!');
        match word {
            "unsafe" => counts.unsafe_keywords = counts.unsafe_keywords.saturating_add(1),
            "asm" | "naked_asm" if is_macro => {
                counts.asm_macros = counts.asm_macros.saturating_add(1);
            }
            "global_asm" if is_macro => {
                counts.global_asm_macros = counts.global_asm_macros.saturating_add(1);
            }
            _ => {}
        }
        rest = tail;
    }
    counts
}

/// Replaces comments and string and character literals with spaces so that
/// their contents are not scanned.
pub(crate) fn strip_comments_and_strings(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        let next = chars.get(i.saturating_add(1)).copied();
        if c == '/' && next == Some('/') {
            i = skip_line(&chars, i);
        } else if c == '/' && next == Some('*') {
            i = skip_block_comment(&chars, i);
        } else if c == '"' {
            i = skip_string(&chars, i);
        } else if c == 'r' && is_raw_string_start(&chars, i) {
            i = skip_raw_string(&chars, i);
        } else if c == '\'' && is_char_literal(&chars, i) {
            i = skip_char(&chars, i);
        } else if c == '\''
            && chars
                .get(i.saturating_add(1))
                .is_some_and(|n| n.is_alphabetic() || *n == '_')
        {
            i = skip_lifetime(&chars, i);
        } else {
            out.push(c);
            i = i.saturating_add(1);
        }
        if i > out.len() {
            // Keep the output the same length in characters so that `find`
            // offsets are stable for callers that need them.
            out.extend(std::iter::repeat_n(
                ' ',
                i.saturating_sub(out.chars().count()),
            ));
        }
    }
    out
}

fn skip_line(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars.get(i) != Some(&'\n') {
        i = i.saturating_add(1);
    }
    i
}

fn skip_block_comment(chars: &[char], mut i: usize) -> usize {
    let mut depth = 0u32;
    while i < chars.len() {
        let c = chars.get(i).copied();
        let next = chars.get(i.saturating_add(1)).copied();
        if c == Some('/') && next == Some('*') {
            depth = depth.saturating_add(1);
            i = i.saturating_add(2);
        } else if c == Some('*') && next == Some('/') {
            depth = depth.saturating_sub(1);
            i = i.saturating_add(2);
            if depth == 0 {
                break;
            }
        } else {
            i = i.saturating_add(1);
        }
    }
    i
}

fn skip_string(chars: &[char], mut i: usize) -> usize {
    i = i.saturating_add(1);
    while i < chars.len() {
        match chars.get(i) {
            Some('\\') => i = i.saturating_add(2),
            Some('"') => return i.saturating_add(1),
            _ => i = i.saturating_add(1),
        }
    }
    i
}

fn is_raw_string_start(chars: &[char], i: usize) -> bool {
    let mut j = i.saturating_add(1);
    while chars.get(j) == Some(&'#') {
        j = j.saturating_add(1);
    }
    chars.get(j) == Some(&'"')
        && (i == 0
            || !chars
                .get(i.wrapping_sub(1))
                .is_some_and(|p| p.is_alphanumeric() || *p == '_'))
}

fn skip_raw_string(chars: &[char], i: usize) -> usize {
    let mut j = i.saturating_add(1);
    let mut hashes = 0usize;
    while chars.get(j) == Some(&'#') {
        hashes = hashes.saturating_add(1);
        j = j.saturating_add(1);
    }
    j = j.saturating_add(1);
    while j < chars.len() {
        if chars.get(j) == Some(&'"') {
            let mut k = j.saturating_add(1);
            let mut closing = 0usize;
            while closing < hashes && chars.get(k) == Some(&'#') {
                closing = closing.saturating_add(1);
                k = k.saturating_add(1);
            }
            if closing == hashes {
                return k;
            }
        }
        j = j.saturating_add(1);
    }
    j
}

/// A `'` starts a character literal (rather than a lifetime) if it is
/// followed by an escape or by one character and a closing `'`.
fn is_char_literal(chars: &[char], i: usize) -> bool {
    match chars.get(i.saturating_add(1)) {
        Some('\\') => true,
        Some(_) => chars.get(i.saturating_add(2)) == Some(&'\''),
        None => false,
    }
}

/// Skips a lifetime such as `'a` or `'static`, which starts like a
/// character literal but has no closing quote.
fn skip_lifetime(chars: &[char], mut i: usize) -> usize {
    i = i.saturating_add(1);
    while chars
        .get(i)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        i = i.saturating_add(1);
    }
    i
}

fn skip_char(chars: &[char], mut i: usize) -> usize {
    i = i.saturating_add(1);
    while i < chars.len() {
        match chars.get(i) {
            Some('\\') => i = i.saturating_add(2),
            Some('\'') => return i.saturating_add(1),
            _ => i = i.saturating_add(1),
        }
    }
    i
}

/// A per-crate line of the report.
#[derive(Clone, Debug)]
pub(crate) struct CrateReport {
    /// Crate name.
    pub(crate) name: &'static str,
    /// Counts over every `.rs` file of the crate.
    pub(crate) counts: Counts,
    /// Violations of the crate's budget.
    pub(crate) violations: Vec<String>,
}

/// Counts every crate and compares against the policy.
pub(crate) fn check(root: &Path) -> Result<Vec<CrateReport>, Error> {
    let mut reports = Vec::new();
    for krate in CRATES {
        let mut counts = Counts::default();
        for path in fs::walk_files(&root.join(krate.path))? {
            if fs::extension(&path) == "rs" {
                counts.add(count(&fs::read(&path)?));
            }
        }
        reports.push(CrateReport {
            name: krate.name,
            counts,
            violations: violations_for(krate.name, krate.kind, counts),
        });
    }
    Ok(reports)
}

/// The budget violations of one crate.
pub(crate) fn violations_for(name: &str, kind: Kind, counts: Counts) -> Vec<String> {
    let (unsafe_budget, asm_budget) = match kind {
        Kind::Adapter {
            unsafe_budget,
            asm_budget,
        } => (unsafe_budget, asm_budget),
        Kind::Logic | Kind::Host => (0, 0),
    };
    let mut violations = Vec::new();
    if counts.unsafe_keywords > unsafe_budget {
        violations.push(format!(
            "`{name}` has {} unsafe sites, budget is {unsafe_budget}",
            counts.unsafe_keywords
        ));
    }
    if counts.asm_macros > asm_budget {
        violations.push(format!(
            "`{name}` has {} asm! sites, budget is {asm_budget}",
            counts.asm_macros
        ));
    }
    if counts.global_asm_macros > 0 {
        violations.push(format!("`{name}` uses global_asm!, which is never allowed"));
    }
    violations
}
