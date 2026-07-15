# Testing matrix

| Change | Minimum validation | Add or update tests |
| --- | --- | --- |
| Angular component/template | focused unit test; `pnpm exec ng test` | interaction, rendering branches, accessibility-relevant states |
| Angular service/state | focused unit test; `pnpm exec ng test` | success, failure, loading, and cancellation where applicable |
| Route or user journey | `pnpm exec ng build`; E2E if configured | navigation and critical outcome |
| Shared contract/integration | producer and consumer checks | serialization, validation, failure mapping |
| Rust behavior | `cargo fmt --check`; `cargo clippy -- -D warnings`; `cargo test` | happy path, edge cases, propagated errors |
| Dependency/config change | build plus impacted tests | only when behavior changes |

Use the workspace scripts when they provide equivalent project-specific commands. If a target or manifest is absent, do not fabricate one; report that the check is unavailable.
