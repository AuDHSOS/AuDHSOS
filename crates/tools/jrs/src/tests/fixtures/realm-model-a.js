// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
print(typeof later);
var count = 1;
let lexical = 2;
const fixed = 3;
function read() { return count + lexical + fixed; }
let increment = (function(x) { return () => ++x; })(4);
let signal = Promise.withResolvers();
async function suspended() { let value = await signal.promise; count += value; }
suspended();
Promise.resolve().then(() => count++);
print(read());
