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

## Definition of done

- The requested behavior is implemented with scoped tests updated or added where behavior changed.
- Relevant Angular/Rust checks pass, or any unavailable/failing command is reported with the exact reason.
- Formatting, security boundaries, migration impact, and user-facing error paths have been considered.
- `git diff` contains only intended changes; the handoff names files changed, validations run, and remaining risks.
