# ADR 0001: Tauri shell with Angular frontend

- Status: Accepted
- Date: 2026-07-15

## Context

`skibidibi-git` needs a desktop interface while retaining access to the local Git installation and native operating-system capabilities. The UI should be productive for web developers without requiring a browser-hosted service or a bundled JavaScript runtime in production.

## Decision

Use Tauri as the desktop shell and Angular as the frontend. Angular owns presentation, client-side state, and user interactions. Tauri owns the application boundary, native capabilities, packaging, and Rust-side commands.

The application will be developed and checked on Linux, macOS, and Windows. Platform-specific behavior belongs behind Rust abstractions so the Angular layer remains portable.

## Consequences

- The frontend can use Angular tooling and its TypeScript ecosystem.
- The shipped application remains a native desktop binary with a webview UI.
- Native APIs and Git process execution must be exposed deliberately through Tauri commands.
- CI must exercise both the pnpm/Angular and Cargo/Rust toolchains on all supported operating systems.
