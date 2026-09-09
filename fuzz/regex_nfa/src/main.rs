// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Patterns, UTF-16 inputs, assertions and limits. No external fuzzing engine.
#![forbid(unsafe_code)]

use audhsos_regex::{Error,Limits,Options,Regex};

fuzz_support::fuzz_target!(|bytes:&[u8]| {
    if bytes.len()>4096{return;}
    let split=bytes.iter().position(|b|*b==0).unwrap_or(bytes.len());
    let pattern:Vec<_>=bytes.get(..split).unwrap_or_default().iter().map(|b|u16::from(*b)).collect();
    let input_bytes=bytes.get(split.saturating_add(1)..).unwrap_or_default();
    let input:Vec<_>=input_bytes.iter().map(|b|u16::from(*b)).collect();
    check(&pattern,&input);
    // Also cover arbitrary code units, including lone surrogates, without
    // pretending malformed UTF-8 is valid UTF-8 source.
    let wide:Vec<_>=bytes.as_chunks::<2>().0.iter().map(|pair|u16::from_le_bytes(*pair)).collect();
    check(&wide,&wide);
});

fn check(pattern:&[u16],input:&[u16]){
    let limits=Limits{pattern_units:4096,depth:24,states:1024,captures:8,ranges:256,repetition:64,
        capture_cells:131_072,input_units:4096,work:100_000};
    for options in [Options::default(),Options{multiline:true,dot_all:true}]{
        let compiled=Regex::compile(pattern,options,limits);
        assert!(!matches!(compiled,Err(Error::InvalidProgram)));
        if let Ok(regex)=compiled{
            for sticky in [false,true]{
                let report=regex.find(input,0,sticky,limits);
                assert!(!matches!(report,Err(Error::InvalidProgram)));
                if let Ok(report)=report{
                    assert!(report.state_visits<=u64::try_from(regex.state_count().saturating_mul(input.len().saturating_add(1))).unwrap_or(u64::MAX));
                    if let Some(m)=report.matched{
                        assert!(m.range.start<=m.range.end&&m.range.end<=input.len());
                        for capture in m.captures.into_iter().flatten(){assert!(capture.start>=m.range.start&&capture.end<=m.range.end&&capture.start<=capture.end);}
                    }
                }
            }
        }
    }
}
