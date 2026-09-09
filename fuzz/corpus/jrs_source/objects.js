// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
let o = {n: 1, next() { return ++this.n; }};
let p = Object.create(o);
p.next(); p.self = p;
Object.defineProperty(p, 'x', {value: 42, configurable: true});
delete p.x;
Object.freeze(p);
