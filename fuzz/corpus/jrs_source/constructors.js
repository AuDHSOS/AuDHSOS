// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
function C(x) { this.x = x; }
C.prototype.get = function() { return this.x; };
new Array(new C(1), new C(2)).map(o => o.get()).join();
