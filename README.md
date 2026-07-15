# skibidibi-git

Cross-platform desktop Git client built with Tauri, Angular, and Rust.

## Developer setup

Prerequisites:

- Node.js 22 and pnpm 10
- Rust stable with Cargo
- A system Git installation available on `PATH`
- Tauri's platform prerequisites for your operating system

Install dependencies and run the development checks:

```sh
pnpm install
pnpm lint
pnpm test
pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run the Tauri development app with the project command once the application workspace is present:

```sh
pnpm tauri dev
```

The CI workflow runs the Angular and Rust checks on Linux, macOS, and Windows. Generated files, local documentation, and model caches are ignored; keep source and configuration changes reviewable.

## Architecture

```text
Angular UI (TypeScript)
        │ typed Tauri commands
        ▼
Rust/Tauri native boundary
        │ structured process execution
        ▼
System Git executable
```

Angular owns presentation and client-side state. Rust owns native capabilities, process execution, validation, and structured errors. Git is invoked without a shell and is not bundled. See [ADR 0001](docs/adr/0001-tauri-angular-desktop-architecture.md) and [ADR 0002](docs/adr/0002-system-git-and-typed-ipc.md) for the decisions behind this boundary.
