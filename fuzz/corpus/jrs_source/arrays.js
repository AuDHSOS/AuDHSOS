// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
let a = [1,,3];
a.map((v, i) => v + i).forEach(v => { if (v > 4) a.push(v); });
Object.defineProperty(a, '1', {value:42, configurable:false});
try { Object.defineProperty(a, 'length', {value:0, writable:false}); }
catch(e) { e.name; }
a.length
