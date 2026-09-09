// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
function f(x) { try { if (x) throw {x}; return 42; } finally { x++; } }
for (let i=0;i<10;i++) { try { f(i); } catch(e) { e.x; } finally { if(i==8) break; } }
