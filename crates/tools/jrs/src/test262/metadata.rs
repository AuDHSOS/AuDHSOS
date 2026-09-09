// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Deliberately bounded Test262 frontmatter subset, not a general YAML parser.
//! Execution-affecting unsupported shapes fail closed; descriptive scalars are ignored.
use std::collections::BTreeSet;

#[derive(Default, Debug)]
pub(super) struct Metadata {
    pub flags: Vec<String>,
    pub includes: Vec<String>,
    pub negative: Option<(String, String)>,
}
impl Metadata {
    pub(super) fn flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
    pub(super) fn variants(&self) -> Vec<Variant> {
        if self.flag("module") {
            vec![Variant::Module]
        } else if self.flag("raw") {
            vec![Variant::Raw]
        } else if self.flag("onlyStrict") {
            vec![Variant::Strict]
        } else if self.flag("noStrict") {
            vec![Variant::Sloppy]
        } else {
            vec![Variant::Sloppy, Variant::Strict]
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Variant {
    Sloppy,
    Strict,
    Raw,
    Module,
}
impl Variant {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Sloppy => "non-strict",
            Self::Strict => "strict",
            Self::Raw => "raw",
            Self::Module => "module",
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one frontmatter state machine validates the bounded execution metadata"
)]
pub(super) fn parse(source: &str) -> Result<Metadata, String> {
    let (_, tail) = source
        .split_once("/*---")
        .ok_or("missing Test262 frontmatter")?;
    let (text, _) = tail
        .split_once("---*/")
        .ok_or("unterminated Test262 frontmatter")?;
    if text.len() > 65536 {
        return Err("frontmatter exceeds 64 KiB".into());
    }
    let mut result = Metadata::default();
    let mut seen = BTreeSet::new();
    let mut section = "";
    let mut phase = None;
    let mut error_type = None;
    for line in text.split(['\n', '\r']) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            match section {
                "flags" | "includes" => {
                    let item = atom(
                        trimmed
                            .strip_prefix("- ")
                            .ok_or("invalid metadata list continuation")?,
                    )?;
                    if section == "flags" {
                        result.flags.push(item);
                    } else {
                        result.includes.push(item);
                    }
                }
                "negative" => {
                    let (key, value) =
                        trimmed.split_once(':').ok_or("invalid negative metadata")?;
                    let destination = match key {
                        "phase" => &mut phase,
                        "type" => &mut error_type,
                        _ => return Err("unknown negative metadata field".into()),
                    };
                    if destination.replace(atom(value)?).is_some() {
                        return Err("duplicate negative metadata field".into());
                    }
                }
                _ => {}
            }
        } else {
            let (key, value) = line.split_once(':').ok_or("invalid top-level metadata")?;
            section = key;
            if !seen.insert(key) {
                return Err(format!("duplicate metadata key {key}"));
            }
            match key {
                "flags" | "includes" => {
                    let values = if value.trim().is_empty() {
                        Vec::new()
                    } else {
                        flow_list(value)?
                    };
                    if key == "flags" {
                        result.flags = values;
                    } else {
                        result.includes = values;
                    }
                }
                "negative" => {
                    if !value.trim().is_empty() {
                        return Err("unsupported inline negative metadata".into());
                    }
                }
                "description" | "info" | "esid" | "es5id" | "es6id" | "features" | "locale"
                | "author" => {}
                _ => return Err(format!("unknown metadata field {key}")),
            }
        }
    }
    if seen.contains("negative") {
        let phase = phase.ok_or("negative phase is missing")?;
        let kind = error_type.ok_or("negative type is missing")?;
        if !["parse", "resolution", "runtime"].contains(&phase.as_str()) {
            return Err("unknown negative phase".into());
        }
        result.negative = Some((phase, kind));
    }
    let mut flags = BTreeSet::new();
    for flag in &result.flags {
        if !flags.insert(flag)
            || ![
                "onlyStrict",
                "noStrict",
                "module",
                "raw",
                "async",
                "generated",
                "CanBlockIsFalse",
                "CanBlockIsTrue",
                "non-deterministic",
            ]
            .contains(&flag.as_str())
        {
            return Err(format!("unknown or duplicate flag {flag}"));
        }
    }
    if ["onlyStrict", "noStrict", "module"]
        .iter()
        .filter(|f| result.flag(f))
        .count()
        > 1
    {
        return Err("conflicting execution flags".into());
    }
    if result.flag("raw") && (result.flag("onlyStrict") || result.flag("noStrict")) {
        return Err("conflicting raw mode flags".into());
    }
    if result.flag("CanBlockIsTrue") && result.flag("CanBlockIsFalse") {
        return Err("conflicting agent flags".into());
    }
    if result.includes.len() > 128 || result.flags.len() > 16 {
        return Err("metadata list limit".into());
    }
    Ok(result)
}
fn flow_list(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    let inner = value
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or("unsupported metadata list syntax")?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    inner.split(',').map(atom).collect()
}
fn atom(value: &str) -> Result<String, String> {
    let value = value.trim();
    let value = if value.starts_with('"') {
        value
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or("unterminated metadata string")?
    } else if value.starts_with('\'') {
        value
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .ok_or("unterminated metadata string")?
    } else {
        value
    };
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
    {
        return Err("unsupported metadata atom".into());
    }
    Ok(value.to_owned())
}
