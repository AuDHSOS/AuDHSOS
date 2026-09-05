# Contributing

Read [docs/README.md](docs/README.md) first. The rules below are the short
form; the documents are binding.

## Rules

- Every change passes `cargo xtask check` before it is merged.
- Every crate except the adapter crates listed in the safety policy carries
  `#![forbid(unsafe_code)]`. Every `unsafe` block in an adapter crate has a
  `// SAFETY:` comment and stays within the budget in
  `crates/tools/xtask/src/policy.rs`.
- No dependency outside this repository, in any dependency section.
- Every new behavior comes with tests for the edge cases listed in the
  testing strategy's catalog.
- Every public item has documentation.
- Every file starts with the SPDX header:

```
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
```

## Worktrees

A worktree session of the coding agent branches from the local `HEAD`, as
`.claude/settings.json` says, and puts its checkout under
`.claude/worktrees/`. That directory is ignored, and the checks never
descend into it: a second checkout inside the checkout would be walked
twice and its files judged by the path they have in the worktree. A
worktree holds what is committed and nothing else, so whatever a session
needs — the wrapper scripts under `tools/`, the instructions an agent
reads — belongs in a commit. Each worktree builds into its own `target/`;
`cargo xtask check` runs in it unchanged.

## Commits

Conventional Commits: `feat(scope): ...`, `fix(scope): ...`, `test(scope):
...`, `docs: ...`, `chore: ...`, `refactor(scope): ...`. The scope is the
crate's short name. A footer `Decision: D-nn` references the decision
register when one applies. `CHANGELOG.md` is updated in the same commit.
