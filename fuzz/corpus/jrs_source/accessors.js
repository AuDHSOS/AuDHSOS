// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
let o={n:1,get x(){return this.n;},set x(v){this.n=v;},valueOf(){return this.x;}};
let a=new Array;
Object.defineProperty(a,0,{get:function(){return o+1;}});
try{o.x=42;String(a);Math.max(NaN,o);}catch(e){e.name;}
