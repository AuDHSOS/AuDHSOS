// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
function later() { return 7; }
print(read(), increment(), increment(), later());
print(globalThis.count, globalThis.lexical, globalThis.fixed);
globalThis.count = 8;
var count;
print(read());
implicit = 9;
print(delete implicit, typeof implicit, delete count);
signal.resolve(5);
let saved = read;
