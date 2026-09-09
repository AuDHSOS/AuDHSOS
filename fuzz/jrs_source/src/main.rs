// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Arbitrary source must compile and terminate within limits or return an error.
#![forbid(unsafe_code)]

use jrs::{Error, Host, Limits, Realm, Runtime, SilentHost, Value, compile,compile_script};

struct Echo { time:u128 }
impl Host for Echo {
    fn timer_now(&mut self)->Result<u128,Error>{self.time=self.time.saturating_add(1_000_000);Ok(self.time)}
    fn print(&mut self,_:&[Value])->Result<(),Error>{Ok(())}
    fn call(&mut self,id:u32,_:&Value,args:&[Value])->Result<Value,Error>{
        let value=args.first().cloned().unwrap_or(Value::Undefined);
        if id==1 {Ok(value)}else{Err(Error::Thrown{value})}
    }
}

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let Ok(source) = core::str::from_utf8(bytes) else { return; };
    let limits = Limits {
        source_bytes: 4096,
        tokens: 2048,
        nesting: 32,
        instructions: 4096,
        fuel: 4096,
        stack: 128,
        string_units: 4096,
        heap_entries: 256,
        call_frames: 32,
        binding_slots: 512,
        properties: 128,
        jobs: 128,
        weak_entries: 128,
    };
    if let Ok(program) = compile(source, limits) {
        let result = Runtime::new(limits).run(&program, &mut SilentHost);
        assert!(!matches!(result, Err(Error::InvalidBytecode)), "accepted source violated a VM invariant: {source}");
    }
    let mut host=Echo {time:0};
    if let Ok(mut realm)=Realm::new(limits,&mut host){
        let _=realm.install_queue_microtask();
        let _=realm.install_events();
        let _=realm.install_timers();
        if let Ok(gc)=realm.gc_function(){let _=realm.set_global("gc",&gc);}
        if let Ok(eval)=realm.eval_script_function(){let _=realm.set_global("evalScript",&eval);}
        for(id,name)in [(1,"hostEcho"),(2,"hostThrow")]{
            if let Ok(f)=realm.host_function(id,name,1){let _=realm.set_global(name,&f);}
        }
        // A delimiter can split arbitrary bytes into separate script boundaries.
        // The cumulative fuel/heap quotas bound the whole sequence.
        for script in source.split('\0').take(4){
            let result=match compile_script(script,limits){Ok(program)=>realm.evaluate_compiled(&program),Err(e)=>Err(e)};
            assert!(!matches!(result,Err(Error::InvalidBytecode)),"realm invariant: {source}");
            if let Ok(value)=result {
                if matches!(value,Value::Function(_)){
                    let result=realm.to_string(&value);
                    assert!(!matches!(result,Err(Error::InvalidBytecode)),"source-text invariant: {source}");
                    let _=realm.queue_microtask(&value);
                    let result=realm.call(&value,&Value::Undefined,&[]);
                    assert!(!matches!(result,Err(Error::InvalidBytecode)),"host callback invariant: {source}");
                }
                let result=realm.get(&value,&Value::string("x"));
                assert!(!matches!(result,Err(Error::InvalidBytecode)),"host property invariant: {source}");
                let _=realm.release(&value);
            }
        }
        for _ in 0..8 {
            let result=realm.run_timer();
            assert!(!matches!(result,Err(Error::InvalidBytecode)),"timer invariant: {source}");
            if result!=Ok(true) {break;}
        }
    }
    // Independent fragments: compile arbitrary body or parameter text through
    // the real Function constructor without executing attacker-generated code.
    if bytes.len()<=1024 {
        let mut host=Echo {time:0};
        if let Ok(mut realm)=Realm::new(limits,&mut host){
            let normal=realm.get_global("Function");
            let asynchronous=realm.evaluate("(async()=>{}).constructor");
            for ctor in [normal,asynchronous].into_iter().filter_map(Result::ok){
              for args in [vec![Value::string(source)],vec![Value::string(source),Value::string("return 1")]]{
                let result=realm.call(&ctor,&Value::Undefined,&args);
                assert!(!matches!(result,Err(Error::InvalidBytecode)),"dynamic Function invariant: {source}");
                if let Ok(value)=result{let text=realm.to_string(&value);assert!(!matches!(text,Err(Error::InvalidBytecode)),"dynamic source-text invariant: {source}");}
              }
            }
        }
    }
});
