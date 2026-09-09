// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
// Project-owned transport fixture; does not imitate WPT assertions.
done=function(){};
let log='';
let id=setTimeout(()=>{throw 7},0);clearInterval(id);
setTimeout(()=>{log+='a';queueMicrotask(()=>log+='m')},0);
setTimeout(()=>{
    result_callback({status:log==='am'?0:1,name:'timer checkpoint',message:log});
    complete_callback([{status:0}],{status:0,message:null});
},1);
