# Desktop releases and automatic updates

Skibidibi Git publishes signed updater artifacts through GitHub Releases. Stable tags use
`app-vX.Y.Z`. The release workflow creates a draft containing macOS Apple Silicon, macOS Intel,
Windows NSIS and Windows MSI installers plus `latest.json` and updater signatures.

## One-time configuration

1. Keep the updater private key outside Git. The current development key is stored at
   `~/.tauri/skibidibi-git.key`; its public key is committed in `tauri.conf.json`.
2. Add the private key contents to the repository secret `TAURI_SIGNING_PRIVATE_KEY`.
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is empty for the current key.
3. For warning-free public macOS builds, add a Developer ID Application certificate and:
   `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`,
   `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`.
4. Windows installers are currently unsigned. Add a Windows code-signing provider before a
   public production release.

Never commit, print, attach, or copy the updater private key into diagnostics.

## Release

1. Update the same version in the root package, desktop package, Cargo workspace and
   `apps/desktop/src-tauri/tauri.conf.json`.
2. Run `node tools/verify-release-version.mjs` and the full Angular/Rust checks.
3. Push the release commit and tag it: `git tag app-vX.Y.Z && git push origin app-vX.Y.Z`.
4. Inspect every artifact in the draft GitHub Release, then publish it.
5. Confirm that `latest.json` is available at the configured updater endpoint and test updating
   from the previous installed version.

Draft releases are intentionally invisible to the updater until a maintainer publishes them.
