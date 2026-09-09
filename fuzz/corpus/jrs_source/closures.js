// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
function counter(n) { return () => ++n; }
let a = counter(0); let b = counter(5);
for (let i = 0; i < 20; i++) { let cycle = () => cycle; a(); b(); }
a() + b()
