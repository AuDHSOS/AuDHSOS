// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![forbid(unsafe_code)]
use audhsos_regex_bt::{Error,Limits,Options,Regex};
fuzz_support::fuzz_target!(|bytes:&[u8]|{
    if bytes.len()>1024{return;}
    let split=bytes.iter().position(|b|*b==0).unwrap_or(bytes.len()/2);
    let pattern:Vec<u16>=bytes[..split].iter().map(|b|u16::from(*b)).collect();
    let input:Vec<u16>=bytes.get(split+1..).unwrap_or_default().iter().map(|b|u16::from(*b)).collect();
    check(&pattern,&input);
    let units:Vec<u16>=bytes.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
    let (p,t)=units.split_at(units.len()/2);check(p,t);
    // Grammar-valid probes ensure mutation explores backreference/lookaround
    // state even when the arbitrary pattern portion is mostly invalid.
    if let Some(selector)=bytes.first(){
        let patterns=[r"^(a+)+$",r"(a?)\1",r"(?=(a+))a*b\1",r"(?<=\1([ab]+))c",r"((a?)*)+b",r"(?!(a+))b\1",r"(?<=((a)|(b))+)[ab]",r"(a|aa)*?b"];
        let pattern:Vec<u16>=patterns[usize::from(*selector)%patterns.len()].encode_utf16().collect();
        let text:Vec<u16>=bytes.iter().take(32).map(|b|u16::from(b%3)+97).collect();check(&pattern,&text);
    }
});
fn check(pattern:&[u16],input:&[u16]){
    let limits=Limits{core:audhsos_regex::Limits{pattern_units:1024,depth:16,states:512,captures:8,ranges:256,repetition:16,capture_cells:32768,input_units:1024,work:20000},backtrack_frames:128,assertion_frames:16};
    for options in [Options::default(),Options{multiline:true,dot_all:true}]{
        let result=Regex::compile(pattern,options,limits);assert!(!matches!(result,Err(Error::InvalidProgram)));
        if let Ok(regex)=result{for sticky in [false,true]{
            let result=regex.find(input,0,sticky,limits);assert!(!matches!(result,Err(Error::InvalidProgram)));
            if let Ok(report)=result{
                assert!(report.work<=limits.core.work&&report.peak_backtrack_frames<=limits.backtrack_frames&&report.peak_assertion_frames<=limits.assertion_frames);
                if let Some(m)=&report.matched{assert!(m.range.start<=m.range.end&&m.range.end<=input.len());assert_eq!(m.captures.len(),regex.capture_count());for r in m.captures.iter().flatten(){assert!(r.start<=r.end&&r.end<=input.len());}}
                let nfa_limits=audhsos_regex::Limits{capture_cells:1_048_576,work:1_000_000,..limits.core};
                if let Ok(nfa)=audhsos_regex::Regex::compile(pattern,options,nfa_limits)&&let Ok(regular)=nfa.find(input,0,sticky,nfa_limits){assert_eq!(report.matched,regular.matched,"{pattern:?} {input:?}");}
            }
        }}
    }
}
