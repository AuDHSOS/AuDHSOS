// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![forbid(unsafe_code)]
fuzz_support::fuzz_target!(|bytes:&[u8]|{
    let data:Vec<u16>=bytes.iter().take(512).map(|b|u16::from(*b)%7).collect();
    let split=bytes.first().map_or(0,|b|usize::from(*b)).min(data.len());let (needle,text)=data.split_at(split);
    let start=bytes.get(1).map_or(0,|b|usize::from(*b));let mut work=10000;
    let p=audhsos_utf16::Pattern::new(needle,512,&mut work).unwrap();
    let expected=(start.min(text.len())..=text.len()).find(|i|text.get(*i..i+needle.len())==Some(needle));
    assert_eq!(p.find(text,start,&mut work).unwrap(),expected);
    let expected=(0..=start.min(text.len())).rev().find(|i|text.get(*i..i+needle.len())==Some(needle));
    assert_eq!(p.rfind(text,start,&mut work).unwrap(),expected);
    let _=p.find(text,start,&mut 0);
    let units:Vec<u16>=bytes.chunks_exact(2).take(256).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
    let mut i=0;while let Some((c,width,lone))=audhsos_utf16::code_point_at(&units,i){assert!(width==1||width==2);assert!(c<=0x10ffff);assert_eq!(lone,(0xd800..=0xdfff).contains(&c));i+=width;}
    assert_eq!(i,units.len());
});
