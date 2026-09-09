// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
function f(a={x:42}, read=()=>a.x, ...rest) { return `${read()}:${rest.join()}`; }
f(undefined,undefined,1,2);
`outer ${`inner ${/}/.test('}')}`}`;
