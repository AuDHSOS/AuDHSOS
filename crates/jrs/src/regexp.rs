// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! ECMAScript adapter metadata; matching is only in the dedicated regex crate.

use crate::Error;
use alloc::{rc::Rc, string::String};

#[derive(Clone, Copy)]
pub(crate) enum Field {
    Source,
    Flags,
    Flag(char),
}

#[derive(Clone, Debug)]
pub(crate) struct RegExp {
    pub(crate) regex: audhsos_regex::Regex,
    pub(crate) source: Rc<[u16]>,
    pub(crate) flags: String,
    pub(crate) global: bool,
    pub(crate) sticky: bool,
    pub(crate) indices: bool,
}

impl RegExp {
    pub(crate) fn compile(source: Rc<[u16]>, flags: &str) -> Result<Self, Error> {
        let mut options = audhsos_regex::Options::default();
        let mut global = false;
        let mut sticky = false;
        let mut indices = false;
        let mut seen = alloc::collections::BTreeSet::new();
        for flag in flags.chars() {
            if !seen.insert(flag) {
                return Err(Error::Syntax {
                    offset: 0,
                    message: "duplicate RegExp flag",
                });
            }
            match flag {
                'g' => global = true,
                'y' => sticky = true,
                'd' => indices = true,
                'm' => options.multiline = true,
                's' => options.dot_all = true,
                'i' | 'u' | 'v' => {}
                _ => {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "invalid RegExp flag",
                    });
                }
            }
        }
        if seen.contains(&'u') && seen.contains(&'v') {
            return Err(Error::Syntax {
                offset: 0,
                message: "incompatible Unicode RegExp flags",
            });
        }
        if seen.iter().any(|c| matches!(c, 'i' | 'u' | 'v')) {
            return Err(Error::Unsupported {
                feature: "RegExp flag is not implemented by the automaton",
            });
        }
        let regex =
            audhsos_regex::Regex::compile(&source, options, audhsos_regex::Limits::default())
                .map_err(error)?;
        Ok(Self {
            regex,
            source,
            flags: String::from(flags),
            global,
            sticky,
            indices,
        })
    }
}

pub(crate) const fn error(error: audhsos_regex::Error) -> Error {
    match error {
        audhsos_regex::Error::Syntax { offset, message } => Error::Syntax { offset, message },
        audhsos_regex::Error::Unsupported { feature, .. } => Error::Unsupported { feature },
        audhsos_regex::Error::Limit { resource } => Error::Limit { resource },
        audhsos_regex::Error::InvalidProgram => Error::InvalidBytecode,
    }
}
