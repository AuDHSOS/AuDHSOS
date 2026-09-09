// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
let r = /a(b+)?/dg;
let a = r.exec('xxabbb');
if (a) a.indices[0].join();
'a aba'.match(/a./g);
/(a+)+$/.test('aaaaaaaaaaaaaaaa!');
