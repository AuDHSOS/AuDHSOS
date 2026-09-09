// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
print('__jrs_wpt_start__',true);
let failed=false;
done=function(){
    result_callback({status:failed?1:0,name:'late failure',message:null});
    complete_callback([{status:0}],{status:0,message:null});
};
setTimeout(()=>{failed=true;done()},1);
