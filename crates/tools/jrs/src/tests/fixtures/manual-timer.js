// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
// Transport fixture: represent a harness that requires its own asynchronous done.
print('__jrs_wpt_start__',true);
let timerRan=false;
done=function(){
    result_callback({status:timerRan?0:1,name:'manual completion',message:timerRan});
    complete_callback([{status:0}],{status:0,message:null});
};
setTimeout(()=>{timerRan=true;done()},1);
