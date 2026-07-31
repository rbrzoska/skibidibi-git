# Desktop releases and automatic updates

Skibidibi Git publishes signed updater artifacts through GitHub Releases. Stable tags use
`app-vX.Y.Z`. The release workflow creates a draft containing macOS Apple Silicon, macOS Intel,
Windows NSIS and Windows MSI installers, and Linux x64 AppImage, Debian and RPM packages plus
`latest.json` and updater signatures.

Linux artifacts are built on Ubuntu 22.04 to retain compatibility with an older supported glibc
baseline. AppImage is the portable package for most x64 Linux distributions and is the artifact
used by Tauri's automatic updater. Debian and RPM packages provide native installation for their
respective package-manager families and should be upgraded through the package or a newer release.

## One-time configuration

1. Keep the updater private key outside Git. The current development key is stored at
   `~/.tauri/skibidibi-git.key`; its public key is committed in `tauri.conf.json`.
2. Add the private key contents to the repository secret `TAURI_SIGNING_PRIVATE_KEY`.
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is empty for the current key.
3. For warning-free public macOS builds, add a Developer ID Application certificate and:
   `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`,
   `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`.
4. Regular Windows installers remain unsigned unless a signing provider is configured. Microsoft
   Store builds have a separate mandatory Authenticode workflow; see
   [Microsoft Store submission](microsoft-store.md).

Never commit, print, attach, or copy the updater private key into diagnostics.

## Release

1. Update the same version in the root package, desktop package, Cargo workspace and
   `apps/desktop/src-tauri/tauri.conf.json`.
2. Run `node tools/verify-release-version.mjs` and the full Angular/Rust checks.
3. Push the release commit and tag it: `git tag app-vX.Y.Z && git push origin app-vX.Y.Z`.
4. Inspect every artifact in the draft GitHub Release. On Linux, smoke-test at least the AppImage
   and the package matching the test distribution, then publish the release.
5. Confirm that `latest.json` is available at the configured updater endpoint and test updating
   from the previous installed version.

Draft releases are intentionally invisible to the updater until a maintainer publishes them.

For Microsoft Store distribution, create the draft first and then run the dedicated
`Build signed Microsoft Store installer` workflow. It produces an immutable, signed MSI with the
offline WebView2 runtime without changing the smaller installers used by normal GitHub releases.
