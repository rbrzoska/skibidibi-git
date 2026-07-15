# ADR 0002: System Git behind typed Tauri IPC

- Status: Accepted
- Date: 2026-07-15

## Context

The application is a Git client, so behavior should match the Git version and configuration already installed by the user. The frontend also needs a stable, reviewable contract for invoking native operations without coupling UI code to process details.

## Decision

Invoke the user's system `git` executable from the Rust/Tauri layer; do not bundle Git in the application. Resolve and validate the executable through the platform's normal process lookup, pass arguments as structured values, and capture exit status, stdout, and stderr without invoking a shell.

Expose native operations through typed Tauri commands. Define request and response DTOs in Rust with serialization, keep command names and payloads versioned/documented, and map process failures to structured error values. Angular services call these commands through a small typed adapter rather than constructing Git commands directly.

## Consequences

- Users receive their configured Git behavior, credentials, hooks, and extensions.
- Installation stays smaller, but Git must be installed and discoverable on the host.
- Argument arrays and structured errors reduce shell-injection and parsing risks.
- IPC contracts become an explicit compatibility surface and should be tested at the Rust command boundary.
- Git output remains implementation detail of the native adapter; the UI consumes typed domain results.
