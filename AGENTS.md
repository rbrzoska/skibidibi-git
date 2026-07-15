# AI Development Guide

This file is the canonical instruction set for AI contributors. Keep `CLAUDE.md` and Cursor rules as pointers to it; do not duplicate policy elsewhere.

## Repository boundaries

- This is an Angular client with Rust code where native, service, or tooling concerns require it. Keep UI, application state, platform integration, and Rust boundaries explicit; do not leak platform APIs into reusable UI code.
- Preserve public contracts and make the smallest coherent change. Do not change generated artifacts by hand.
- Use the Angular version declared in `package.json` and the matching workspace-pinned `@angular/cli`. When upgrading, move Angular packages and the CLI together to the latest stable compatible release; do not mix major versions.

## Required workflow

- Install and run JavaScript tooling with pnpm. Generate Angular code CLI-first: `pnpm exec ng generate <schematic> ...`. Do not hand-create the standard Angular scaffolding the CLI can generate.
- Run the narrowest relevant checks first, then project-wide checks before handoff:
  - `pnpm exec ng test`
  - `pnpm exec ng build`
  - `pnpm exec ng e2e` when an end-to-end target exists
- For Rust changes, use `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` from the relevant Cargo workspace/package.

## Engineering constraints

- Prefer `OnPush`, lazy loading, `track`/`trackBy`, bounded subscriptions, and measured rendering/data work. Avoid unnecessary change detection, large eager bundles, and unbounded lists.
- Validate untrusted input at every boundary; never expose secrets in client code, logs, fixtures, or commits. Use parameterized APIs and least-privilege access.
- Rust: format with rustfmt; satisfy clippy; propagate actionable errors instead of `unwrap`/`expect` in production paths; avoid `unsafe` unless documented with a justified invariant and a safe wrapper.

## Worktrees and agents

- Work only in the assigned worktree. Inspect `git status` first and preserve unrelated changes.
- Claim files before editing them. One agent owns a file at a time; coordinate handoffs before overlapping edits.
- Keep commits focused and do not include caches, local settings, generated output, or another agent's work.
- Optimize model cost by default: reserve the strongest/Sol profile for security-sensitive Git execution, cross-platform process behavior, contract arbitration, and final blocker review. Delegate bounded Angular presentation, pure reducers/parsers, fixtures, documentation, and focused regression tests to cheaper profiles whenever file ownership can remain disjoint.
- Split parallel work only after DTOs and file ownership are frozen. Prefer several small independently verifiable tasks over one broad agent task, and always run an integrated review/test pass before handoff.

## Git test isolation

- Never run mutating Git tests, fixtures, smoke scenarios, branch switches, stash operations, rebases, pulls, resets, or worktree operations against the user's production repositories, including `voucherify-mono`.
- Prefer disposable repositories created under a temporary directory for automated and real-Git integration tests. Each test owns its repository, branches, remotes, and worktrees and removes them through the test fixture lifecycle.
- When a persistent manual test repository is genuinely needed, use this `skibidibi-git` repository only with clearly named dedicated test branches/worktrees (for example `codex/test-*`) created specifically for the scenario. Never reuse the user's normal branches or existing worktrees.
- Before any manual mutating Git smoke test, verify the repository root, current branch, worktree path, and clean/expected state. Stop if the target is not an isolated fixture or an explicitly dedicated `skibidibi-git` test worktree.
- Do not use another user repository as a test fixture even for read-only validation. Read-only inspection is allowed only when the user explicitly requests analysis of that repository, never as a substitute for an isolated test repository; do not modify its refs, worktrees, configuration, index, or files.

## Definition of done

- The requested behavior is implemented with scoped tests updated or added where behavior changed.
- Relevant Angular/Rust checks pass, or any unavailable/failing command is reported with the exact reason.
- Formatting, security boundaries, migration impact, and user-facing error paths have been considered.
- `git diff` contains only intended changes; the handoff names files changed, validations run, and remaining risks.
