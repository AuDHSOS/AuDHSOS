// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
// Project-owned runner transport test, not a WPT conformance result.
let order = [];
Promise.resolve().then(() => { order.push('a'); queueMicrotask(() => order.push('d')); });
queueMicrotask(() => { order.push('b'); Promise.resolve().then(() => order.push('e')); });
Promise.reject().catch(() => order.push('c'));
queueMicrotask(() => queueMicrotask(() => {
  result_callback({status: order.join() === 'a,b,c,d,e' ? 0 : 1,
                   name:'mixed FIFO', message:order.join()});
}));
