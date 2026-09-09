// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
// A successful result callback cannot hide an uncaught queued exception.
queueMicrotask(() => { throw 7; });
queueMicrotask(() => result_callback({status:0, name:'after exception', message:null}));
