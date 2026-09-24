// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Host-only Unicode 18.0.0 table generator.
#![expect(
    clippy::arithmetic_side_effects,
    reason = "host generator checks Unicode ranges and field counts before indexing"
)]
#![cfg_attr(
    not(test),
    expect(
        clippy::indexing_slicing,
        reason = "validated generator ranges and fields"
    )
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    fs,
    path::Path,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const LIMIT: usize = 0x11_0000;
const VERSION: &str = "18.0.0";
type Record = (u32, u32, String);

pub(crate) struct Property {
    pub name: &'static str,
    pub function: &'static str,
    pub default: String,
    pub names: Vec<String>,
    pub values: Vec<u16>,
}
pub(crate) struct Model {
    pub properties: Vec<Property>,
    pub extensions: Vec<(u32, u32, Vec<String>)>,
    pub mirrors: Vec<(u32, u32)>,
    pub brackets: Vec<(u32, u32, String)>,
    pub decompositions: Vec<(u32, Vec<u32>)>,
    pub compositions: Vec<(u32, u32, u32)>,
    pub checksums: Vec<(String, u64)>,
}
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
fn source(root: &Path, name: &str, hashes: &mut Vec<(String, u64)>) -> Result<String> {
    let data = fs::read_to_string(root.join(name))?;
    if name != "UnicodeData.txt" && !data.lines().take(20).any(|line| line.contains(VERSION)) {
        return Err(format!("{name}: Unicode version mismatch").into());
    }
    let sum = checksum(data.as_bytes());
    if name == "UnicodeData.txt" && sum != 0xa0a3_2f31_0c04_54e3 {
        return Err("UnicodeData version fingerprint mismatch".into());
    }
    hashes.push((name.to_owned(), sum));
    Ok(data)
}
fn code(text: &str) -> Result<u32> {
    let n = u32::from_str_radix(text.trim(), 16)?;
    if n >= 0x11_0000 {
        return Err("code point exceeds Unicode range".into());
    }
    Ok(n)
}
fn range(text: &str) -> Result<(u32, u32)> {
    let (a, b) = text
        .trim()
        .split_once("..")
        .unwrap_or((text.trim(), text.trim()));
    let (a, b) = (code(a)?, code(b)?);
    if a > b {
        return Err("reversed Unicode range".into());
    }
    Ok((a, b))
}
fn fields(line: &str) -> Vec<&str> {
    line.split('#')
        .next()
        .unwrap_or("")
        .split(';')
        .map(str::trim)
        .collect()
}
fn records(data: &str, selected: Option<&str>, defaults: bool) -> Result<Vec<Record>> {
    let mut result = Vec::new();
    for raw in data.lines() {
        let line = if defaults {
            if let Some((_, line)) = raw.split_once("@missing:") {
                line
            } else {
                continue;
            }
        } else {
            raw
        };
        let f = fields(line);
        if f.len() < 2 || f[0].is_empty() {
            continue;
        }
        let value = if let Some(selected) = selected {
            if f[1] != selected {
                continue;
            }
            *f.get(2).unwrap_or(&"Yes")
        } else {
            f[1]
        };
        let (a, b) = range(f[0])?;
        result.push((a, b, value.to_owned()));
    }
    Ok(result)
}
fn property(
    name: &'static str,
    function: &'static str,
    default: &str,
    defaults: &[Record],
    records: &[Record],
) -> Result<Property> {
    let mut names = BTreeSet::from([default.to_owned()]);
    for (_, _, value) in defaults.iter().chain(records) {
        names.insert(value.clone());
    }
    let names: Vec<_> = names.into_iter().collect();
    let ids: BTreeMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(i, v)| Ok((v.as_str(), u16::try_from(i)?)))
        .collect::<Result<_>>()?;
    let mut values = vec![ids[default]; LIMIT];
    let mut assigned = vec![false; LIMIT];
    for (explicit, entries) in [(false, defaults), (true, records)] {
        for (a, b, value) in entries {
            let (a, b) = (usize::try_from(*a)?, usize::try_from(*b)?);
            if a > b || b >= LIMIT {
                return Err("invalid range".into());
            }
            for i in a..=b {
                if explicit && assigned[i] {
                    return Err(format!("{name}: overlapping property ranges").into());
                }
                values[i] = ids[value.as_str()];
                if explicit {
                    assigned[i] = true;
                }
            }
        }
    }
    Ok(Property {
        name,
        function,
        default: default.to_owned(),
        names,
        values,
    })
}
fn variant(name: &str) -> String {
    name.split('_')
        .map(|part| {
            if part.chars().all(|ch| !ch.is_lowercase()) {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |c| {
                    c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                })
            } else {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |c| {
                    c.to_uppercase().collect::<String>() + chars.as_str()
                })
            }
        })
        .collect()
}
fn literal(n: u32) -> String {
    format!("0x{:02x}_{:04x}", n >> 16, n & 0xffff)
}

struct UnicodeData {
    categories: Vec<Record>,
    combining: Vec<Record>,
    decompositions: Vec<(u32, Vec<u32>)>,
}
fn unicode_data(unicode: &str) -> Result<UnicodeData> {
    let mut categories = Vec::new();
    let mut combining = Vec::new();
    let mut decompositions = Vec::new();
    let mut first = None;
    for line in unicode.lines() {
        let f = fields(line);
        if f.len() != 15 {
            return Err("UnicodeData needs fifteen fields".into());
        }
        let cp = code(f[0])?;
        if let Some(name) = f[1].strip_suffix(", First>") {
            if first.is_some() {
                return Err("nested First range".into());
            }
            first = Some((cp, name.to_owned(), f[2].to_owned(), f[3].to_owned()));
            continue;
        }
        let start = if let Some(name) = f[1].strip_suffix(", Last>") {
            let (start, n, cat, ccc) = first.take().ok_or("Last without First")?;
            if n != name || cat != f[2] || ccc != f[3] || start > cp {
                return Err("First/Last mismatch".into());
            }
            start
        } else {
            if first.is_some() {
                return Err("First without Last".into());
            }
            cp
        };
        categories.push((start, cp, f[2].to_owned()));
        combining.push((start, cp, f[3].to_owned()));
        if !f[5].is_empty() && !f[5].starts_with('<') {
            decompositions.push((
                cp,
                f[5].split_whitespace().map(code).collect::<Result<_>>()?,
            ));
        }
    }
    if first.is_some() {
        return Err("unterminated First range".into());
    }
    Ok(UnicodeData {
        categories,
        combining,
        decompositions,
    })
}

/// Canonical pairs sorted by (first, second), without `Full_Composition_Exclusion`
/// of UAX #15: the listed composites, singletons, and non-starter decompositions.
fn compositions(
    decompositions: &[(u32, Vec<u32>)],
    combining: &[Record],
    exclusions: &str,
) -> Result<Vec<(u32, u32, u32)>> {
    let mut excluded = BTreeSet::new();
    for line in exclusions.lines() {
        let f = fields(line);
        if !f[0].is_empty() {
            let (a, b) = range(f[0])?;
            excluded.extend(a..=b);
        }
    }
    let starter = |code: u32| {
        combining
            .partition_point(|(a, _, _)| *a <= code)
            .checked_sub(1)
            .and_then(|i| combining.get(i))
            .filter(|(_, b, _)| code <= *b)
            .is_none_or(|(_, _, ccc)| ccc == "0")
    };
    let mut pairs = Vec::new();
    for (code, parts) in decompositions {
        if let [first, second] = parts[..]
            && !excluded.contains(code)
            && starter(*code)
            && starter(first)
        {
            pairs.push((first, second, *code));
        }
    }
    pairs.sort_unstable();
    if pairs
        .windows(2)
        .any(|w| (w[0].0, w[0].1) == (w[1].0, w[1].1))
    {
        return Err("two primary composites share one canonical pair".into());
    }
    Ok(pairs)
}

const SOURCES: &[(&str, &str, &str, &str)] = &[
    (
        "auxiliary/GraphemeBreakProperty.txt",
        "GraphemeBreak",
        "grapheme_break",
        "Other",
    ),
    (
        "auxiliary/WordBreakProperty.txt",
        "WordBreak",
        "word_break",
        "Other",
    ),
    ("LineBreak.txt", "LineBreak", "line_break", "XX"),
    (
        "EastAsianWidth.txt",
        "EastAsianWidth",
        "east_asian_width",
        "N",
    ),
    ("Scripts.txt", "Script", "script", "Unknown"),
    (
        "extracted/DerivedBidiClass.txt",
        "BidiClass",
        "bidi_class",
        "L",
    ),
];

fn properties(
    root: &Path,
    hashes: &mut Vec<(String, u64)>,
    categories: &[Record],
    combining: &[Record],
    bidi_aliases: &BTreeMap<String, String>,
) -> Result<Vec<Property>> {
    let mut properties = vec![
        property("GeneralCategory", "general_category", "Cn", &[], categories)?,
        property("CombiningClass", "combining_class", "0", &[], combining)?,
    ];
    for &(file, name, function, default) in SOURCES {
        let data = source(root, file, hashes)?;
        let mut defaults = records(&data, None, true)?;
        if name == "BidiClass" {
            for (_, _, v) in &mut defaults {
                *v = bidi_aliases.get(v).ok_or("unknown bidi alias")?.clone();
            }
        }
        properties.push(property(
            name,
            function,
            default,
            &defaults,
            &records(&data, None, false)?,
        )?);
    }
    let data = source(root, "DerivedCoreProperties.txt", hashes)?;
    properties.push(property(
        "IndicConjunctBreak",
        "indic_conjunct_break",
        "None",
        &[],
        &records(&data, Some("InCB"), false)?,
    )?);
    properties.push(property(
        "DefaultIgnorable",
        "default_ignorable",
        "No",
        &[],
        &records(&data, Some("Default_Ignorable_Code_Point"), false)?,
    )?);
    let data = source(root, "emoji/emoji-data.txt", hashes)?;
    properties.push(property(
        "ExtendedPictographic",
        "extended_pictographic",
        "No",
        &[],
        &records(&data, Some("Extended_Pictographic"), false)?,
    )?);
    let mut joining_defaults = Vec::new();
    for (a, b, category) in categories {
        if matches!(category.as_str(), "Mn" | "Me" | "Cf") {
            joining_defaults.push((*a, *b, "T".to_owned()));
        }
    }
    let data = source(root, "ArabicShaping.txt", hashes)?;
    let mut joining = Vec::new();
    for line in data.lines() {
        let f = fields(line);
        if f.len() >= 4 {
            let (a, b) = range(f[0])?;
            joining.push((a, b, f[2].to_owned()));
        }
    }
    properties.push(property(
        "JoiningType",
        "joining_type",
        "U",
        &joining_defaults,
        &joining,
    )?);
    Ok(properties)
}

pub(crate) fn load(root: &Path) -> Result<Model> {
    let mut hashes = Vec::new();
    let aliases = source(root, "PropertyValueAliases.txt", &mut hashes)?;
    let mut bidi_aliases = BTreeMap::new();
    let mut scripts = BTreeMap::new();
    for line in aliases.lines() {
        let f = fields(line);
        if f.len() >= 3 {
            if f[0] == "bc" {
                bidi_aliases.insert(f[2].to_owned(), f[1].to_owned());
            } else if f[0] == "sc" {
                scripts.insert(f[1].to_owned(), f[2].to_owned());
            }
        }
    }
    let UnicodeData {
        categories,
        combining,
        decompositions,
    } = unicode_data(&source(root, "UnicodeData.txt", &mut hashes)?)?;
    let compositions = compositions(
        &decompositions,
        &combining,
        &source(root, "CompositionExclusions.txt", &mut hashes)?,
    )?;
    let properties = properties(root, &mut hashes, &categories, &combining, &bidi_aliases)?;
    let data = source(root, "ScriptExtensions.txt", &mut hashes)?;
    let mut extensions = Vec::new();
    for (a, b, value) in records(&data, None, false)? {
        let mut names = value
            .split_whitespace()
            .map(|s| {
                scripts
                    .get(s)
                    .cloned()
                    .ok_or_else(|| "unknown script alias".into())
            })
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        extensions.push((a, b, names));
    }
    extensions.sort_by_key(|v| v.0);
    let mut mirrors = Vec::new();
    let data = source(root, "BidiMirroring.txt", &mut hashes)?;
    for line in data.lines() {
        let f = fields(line);
        if f.len() >= 2 {
            mirrors.push((code(f[0])?, code(f[1])?));
        }
    }
    mirrors.sort_unstable();
    let mut brackets = Vec::new();
    let data = source(root, "BidiBrackets.txt", &mut hashes)?;
    for line in data.lines() {
        let f = fields(line);
        if f.len() >= 3 {
            if !matches!(f[2], "o" | "c") {
                return Err("unknown bracket kind".into());
            }
            brackets.push((code(f[0])?, code(f[1])?, f[2].to_owned()));
        }
    }
    brackets.sort();
    hashes.sort();
    Ok(Model {
        properties,
        extensions,
        mirrors,
        brackets,
        decompositions,
        compositions,
        checksums: hashes,
    })
}

pub(crate) fn generate(model: &Model) -> Result<String> {
    let mut out = "// SPDX-License-Identifier: AGPL-3.0-only AND Unicode-3.0\n// Copyright (C) 2026 Manuel Baesler and contributors\n// Copyright © 2026 Unicode, Inc.\n// Generated from Unicode data; see LICENSE.txt in this directory.\n\nuse super::{Range, lookup};\n\n".to_owned();
    let major = VERSION.split('.').next().ok_or("version major")?;
    writeln!(
        out,
        "/// Unicode version of every generated property.\npub const VERSION: &str = \"{VERSION}\";\n/// Major component of `VERSION`, reported in the layout stream header.\npub const VERSION_MAJOR: u16 = {major};\n"
    )?;
    for (path, sum) in &model.checksums {
        writeln!(out, "// FNV-1a-64 {sum:016x} {path}")?;
    }
    for prop in &model.properties {
        emit_property(&mut out, prop)?;
    }
    writeln!(
        out,
        "pub(super) static EXTENSIONS: &[Range<&[Script]>] = &["
    )?;
    for (a, b, names) in &model.extensions {
        writeln!(
            out,
            "    Range {{ lo: {}, hi: {}, value: &[{}] }},",
            literal(*a),
            literal(*b),
            names
                .iter()
                .map(|n| format!("Script::{}", variant(n)))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out, "pub(super) static MIRRORS: &[(u32,u32)] = &[")?;
    for (a, b) in &model.mirrors {
        writeln!(out, "    ({}, {}),", literal(*a), literal(*b))?;
    }
    writeln!(out, "];")?;
    writeln!(out, "pub(super) static BRACKETS: &[(u32,u32,bool)] = &[")?;
    for (a, b, k) in &model.brackets {
        writeln!(out, "    ({}, {}, {}),", literal(*a), literal(*b), k == "o")?;
    }
    writeln!(out, "];")?;
    writeln!(
        out,
        "pub(super) static DECOMPOSITIONS: &[(u32,&[u32])] = &["
    )?;
    for (a, seq) in &model.decompositions {
        writeln!(
            out,
            "    ({}, &[{}]),",
            literal(*a),
            seq.iter()
                .map(|n| literal(*n))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out, "pub(super) static COMPOSITIONS: &[(u32,u32,u32)] = &[")?;
    for (first, second, code) in &model.compositions {
        writeln!(
            out,
            "    ({}, {}, {}),",
            literal(*first),
            literal(*second),
            literal(*code)
        )?;
    }
    writeln!(out, "];")?;
    Ok(out)
}

fn emit_property(out: &mut String, prop: &Property) -> Result<()> {
    let constant = prop.function.to_uppercase();
    let numeric = prop.name == "CombiningClass";
    if !numeric {
        writeln!(
            out,
            "\n/// Unicode `{}` property.\n#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]\n#[repr(u16)]\npub enum {} {{",
            prop.name, prop.name
        )?;
        for name in &prop.names {
            writeln!(out, "    /// UCD value `{name}`.\n    {},", variant(name))?;
        }
        writeln!(out, "}}")?;
    }
    let ty = if numeric { "u8" } else { prop.name };
    let value = |name: &str| {
        if numeric {
            name.to_owned()
        } else {
            format!("{}::{}", prop.name, variant(name))
        }
    };
    writeln!(
        out,
        "\n/// Look up `{}`; out-of-range input has the default value.\n#[must_use]\npub fn {}(code: u32) -> {ty} {{\n    lookup({constant}, code, {})\n}}",
        prop.name,
        prop.function,
        value(&prop.default)
    )?;
    writeln!(out, "\nstatic {constant}: &[Range<{ty}>] = &[")?;
    let mut start = 0;
    while start < prop.values.len() {
        let id = prop.values[start];
        let mut end = start + 1;
        while end < prop.values.len() && prop.values[end] == id {
            end += 1;
        }
        let name = &prop.names[usize::from(id)];
        if *name != prop.default {
            writeln!(
                out,
                "    Range {{ lo: {}, hi: {}, value: {} }},",
                literal(u32::try_from(start)?),
                literal(u32::try_from(end - 1)?),
                value(name)
            )?;
        }
        start = end;
    }
    writeln!(out, "];\n")?;
    Ok(())
}

#[cfg(test)]
#[path = "../src/tests/unicode_generator.rs"]
mod tests;
