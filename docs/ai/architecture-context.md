# Architecture context

## Current baseline

The repository foundation does not yet include an application manifest or source tree. Do not invent module names, APIs, deployment targets, or persistence assumptions. Discover them from `package.json`, `angular.json`, source, and Cargo manifests when they are added.

## Intended boundaries

| Area | Responsibility | Must not depend directly on |
| --- | --- | --- |
| Angular UI | presentation, accessibility, interaction | platform/Rust internals |
| Application layer | feature orchestration and state | browser/platform implementation details |
| Integration layer | HTTP, storage, native bridges | UI components |
| Rust | native/service/tooling capabilities | Angular component concerns |

Expose cross-boundary behavior through typed, narrow contracts. Validate at the receiving boundary and map failures to user-safe errors.

## Discovery checklist

Before a feature or refactor, identify the workspace CLI version, Angular target(s), test runner, lint/format scripts, Cargo workspace(s), public API contracts, and existing error/telemetry conventions.
