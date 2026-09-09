// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
let n=0;
for (const [key,value] of Object.entries({a:40,b:2})) {
    switch(key) {case 'a': n+=value; break; default: n+=value;}
}
for (let key in {x:1,y:2}) { try { continue; } finally { n++; } }
n
