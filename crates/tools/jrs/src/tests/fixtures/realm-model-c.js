// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
function read() { return 99; }
print(read(), saved(), count, increment());
class Example { method() { return lexical; } }
print(new Example().method());
print(Object.getOwnPropertyDescriptor(globalThis, 'count').configurable);
