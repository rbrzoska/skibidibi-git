# Skibidibi Git — release notes

## 0.1.9 — 2026-08-07

- Fixed release breakage by blocking push path when branch metadata is missing and adding safe guardrails for detached/unborn branch push analysis.

## 0.1.8 — 2026-08-07

- Fixed push blocking behavior so you can push with local uncommitted changes in the working tree.
- Kept a dirty-tree safety path by showing explicit branch readiness from local status when push analysis cannot run.
- Added a regression check for dirty-tree push scenarios.

## 0.1.7 — 2026-08-04

- Added a dedicated unsigned MSIX build for Microsoft Store distribution without requiring a commercial signing certificate.
- Fixed Windows release validation, Git fixtures, managed worktree paths, and package assets discovered during certification testing.
- Verified the disposable MSIX installation, launch, restart, uninstall, and manual Git workflow on Windows 11 with an overall WACK PASS result.
- Separated Microsoft Store updates from the standalone GitHub Releases updater while keeping release notes available in the application.
- Added a Partner Center resubmission checklist and certification notes for the external Git and editor integrations.

## 0.1.6 — 2026-07-31

- Added editable amend support for unpushed HEAD commits and compact commit reference labels.
- Added commit context actions for creating a branch, cherry-picking, reverting, and soft, mixed, or hard reset.
- Added persistent branch and worktree favorites with clearer current and release indicators.
- Added a safe cleanup assistant that groups worktrees with linked branches, suggests stale candidates, and supports multi-selection.
- Added an in-app “What’s new” view with bundled release history before and after updates.
- Added privacy and support information plus a dedicated signed, offline Microsoft Store MSI workflow.

## 0.1.5 — 2026-07-30

- Added an in-app update notifier with explicit download, install, and restart controls.
- Added signed updater manifests for Windows and macOS releases.
- Changed the Windows installer to a per-machine installation in `Program Files\Skibidibi Git`.
- Added UAC elevation for Windows installation and elevated MSI update support.
- Added Windows x64, macOS Apple Silicon, and macOS Intel release packages.

## 0.1.4 — 2026-07-29

- Added manual and automatic checks for application updates.
- Added a compact update notification with release information.
- Added deferred restart support after an update is installed.
- Added the first public cross-platform desktop release pipeline.
