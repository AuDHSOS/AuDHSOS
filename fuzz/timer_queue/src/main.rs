// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![forbid(unsafe_code)]
use audhsos_timer_queue::{DeadlineQueue,Error};
fuzz_support::fuzz_target!(|bytes:&[u8]|{
    let limit=usize::from(bytes.first().copied().unwrap_or(0)%33);
    let mut queue=DeadlineQueue::new(limit);
    let mut model=Vec::new();
    for chunk in bytes.chunks_exact(4).take(1024) {
        let time=u128::from(chunk[1]);let token=u64::from(u16::from_le_bytes([chunk[2],chunk[3]]));
        match chunk[0]%3 {
            0=> {let result=queue.insert(time,token);if model.len()==limit {assert_eq!(result,Err(Error::Capacity));}
                else {model.push((time,result.unwrap(),token));model.sort();}},
            1=> {let expected=model.iter().position(|(_,id,_)|*id==token).map(|i|model.remove(i).2);assert_eq!(queue.remove(token),expected);},
            _=> {let expected=if model.first().is_some_and(|(t,_,_)|*t<=time) {let (_,id,v)=model.remove(0);Some((id,v))}else{None};assert_eq!(queue.pop_due(time),expected);}
        }
        assert_eq!(queue.len(),model.len());assert_eq!(queue.next_deadline(),model.first().map(|(t,_,_)|*t));
        assert_eq!(queue.values().copied().collect::<Vec<_>>(),model.iter().map(|(_,_,v)|*v).collect::<Vec<_>>());
    }
});
