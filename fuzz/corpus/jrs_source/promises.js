// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
async function work(x) { try { return (await x) + 1; } finally { await 0; } }
let r=Promise.withResolvers();
work(r.promise).then(x=>x).catch(e=>e);
r.resolve({then(resolve,reject){resolve(41);reject(1);}});
Promise.all([1,Promise.resolve(2)]).finally(()=>0);
