// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Registry operations against a small independent sequential model.
#![forbid(unsafe_code)]
use audhsos_event_target::{Event,Listeners,Options};
use std::rc::Rc;
fuzz_support::fuzz_target!(|bytes:&[u8]|{
    let mut ls=Listeners::new();let mut model:Vec<(u64,u16,u8,Options)>=Vec::new();let mut id=0u64;let mut snapshot=Vec::new();
    let mut event=Event::new(Rc::from([120]),false,true,false);
    for chunk in bytes.chunks_exact(4).take(512){
        let ty=u16::from(chunk[1]%4);let cb=chunk[2]%8;let options=Options{capture:chunk[3]&1!=0,once:chunk[3]&2!=0,passive:chunk[3]&4!=0};
        match chunk[0]%7{
            0=>{let duplicate=model.iter().any(|(_,t,c,o)|*t==ty&&*c==cb&&o.capture==options.capture);let result=ls.add(Rc::from([ty]),cb,options,64,4);if duplicate{assert_eq!(result,Ok(false));}else if model.len()<64{assert_eq!(result,Ok(true));model.push((id,ty,cb,options));id+=1;}else{assert!(result.is_err());}}
            1=>{let old=model.len();model.retain(|(_,t,c,o)|!(*t==ty&&*c==cb&&o.capture==options.capture));assert_eq!(ls.remove(&[ty],&cb,options.capture),old!=model.len());}
            2=>{snapshot=ls.snapshot(&[ty],options.capture);assert_eq!(snapshot,model.iter().filter(|(_,t,_,o)|*t==ty&&o.capture==options.capture).map(|(id,_,_,_)|*id).collect::<Vec<_>>());}
            3=>{for id in &snapshot{let expected=model.iter().find(|(i,_,_,_)|i==id).copied();let actual=ls.take_for_invoke(*id);assert_eq!(actual.as_ref().map(|l|(l.id,l.callback,l.options)),expected.map(|(i,_,c,o)|(i,c,o)));if expected.is_some_and(|(_,_,_,o)|o.once){model.retain(|(i,_,_,_)|i!=id);}}}
            4=>{event.passive(options.passive);event.prevent_default();if !options.passive{assert!(event.canceled());}event.stop(options.once);assert!(event.stopped());}
            5=>{let id=u64::from(chunk[2]);let old=model.len();model.retain(|(i,_,_,_)|*i!=id);assert_eq!(ls.remove_id(id),old!=model.len());}
            _=>{event.finish();assert!(!event.stopped()&&!event.dispatching());assert!(event.begin());assert!(!event.begin());}
        }
        assert_eq!(ls.len(),model.len());
        assert_eq!(ls.next_id(),id);
    }
});
