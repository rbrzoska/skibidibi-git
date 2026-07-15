# Tooling

## Angular CLI MCP

The repository-local MCP configurations run the workspace-pinned Angular CLI:

```sh
pnpm exec ng mcp
```

It becomes usable after this repository declares `@angular/cli` in its pnpm workspace dependencies. Keep Angular and the CLI on matching compatible stable versions as required by [AGENTS.md](../../AGENTS.md).

## Optional Codex configuration

Do not edit user-global configuration as part of repository work. To enable the same server locally, add this snippet to your personal Codex configuration using that installation's MCP-server format:

```toml
[mcp_servers.angular_cli]
command = "pnpm"
args = ["exec", "ng", "mcp"]
```

Run it from this repository's workspace root so pnpm resolves the pinned CLI.
