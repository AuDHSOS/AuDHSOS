// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
print('__jrs_wpt_start__',true);
done=function(){
    result_callback({status:0,name:'must not be invoked by runner',message:null});
    complete_callback([{status:0}],{status:0,message:null});
};
