// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Strict parser and UTF-16 quoting roundtrip, independent of the JS adapter.
#![forbid(unsafe_code)]
use audhsos_json::{Kind, Limits, parse, quote};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let bytes=&bytes[..bytes.len().min(8192)];
    let raw:Vec<u16>=bytes.chunks(2).map(|c|u16::from_le_bytes([c[0],*c.get(1).unwrap_or(&0)])).collect();
    let ascii:Vec<u16>=bytes.iter().map(|b|u16::from(*b)).collect();
    let limits=Limits{input:65536,nodes:8192,depth:48,strings:32768};
    for text in [&raw,&ascii]{
        let mut work=100_000;
        if let Ok(doc)=parse(text,limits,&mut work){
            assert_eq!(doc.root,doc.nodes.len()-1);
            for (index,node)in doc.nodes.iter().enumerate(){
                assert!(node.source.start<node.source.end&&node.source.end<=text.len());
                match &node.kind{
                    Kind::Array(items)=>assert!(items.iter().all(|i|*i<index)),
                    Kind::Object(items)=>assert!(items.iter().all(|(_,i)|*i<index)),
                    _=>{},
                }
                let parsed=parse(&text[node.source.clone()],limits,&mut 100_000).unwrap();
                assert_eq!(core::mem::discriminant(&parsed.nodes[parsed.root].kind),core::mem::discriminant(&node.kind));
            }
        }
    }
    let mut output=Vec::new();
    quote(&mut output,&raw,65536,&mut 100_000).unwrap();
    let doc=parse(&output,limits,&mut 100_000).unwrap();
    assert!(matches!(&doc.nodes[doc.root].kind,Kind::String(s) if s.as_ref()==raw));
    let tiny=Limits{input:32,nodes:4,depth:2,strings:8};
    let _=parse(&ascii,tiny,&mut 10);
    let _=quote(&mut Vec::new(),&raw,16,&mut 10);
});
