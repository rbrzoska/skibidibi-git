# Microsoft Store submission

Skibidibi Git uses the Microsoft Store's unpackaged MSI/EXE submission path. The regular GitHub MSI
is not the Store package: a Store build embeds the offline WebView2 installer and is Authenticode
signed separately.

## Current external prerequisite

Acquire an Authenticode code-signing certificate that chains to a CA in the Microsoft Trusted Root
Program and can be exported as a password-protected PFX. Self-signed certificates and the Tauri
updater signing key are not accepted for an MSI/EXE Store submission.

Add these GitHub Actions repository secrets:

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE` | Base64-encoded PFX bytes, with no surrounding quotes |
| `WINDOWS_CERTIFICATE_PASSWORD` | PFX export password |
| `WINDOWS_TIMESTAMP_URL` | RFC 3161 or Authenticode timestamp URL supplied by the certificate issuer |

The workflow never commits the PFX or generated signing configuration. It imports the certificate
into the ephemeral runner's user certificate store, asks Tauri to sign the application executable
and MSI, verifies both signatures, and removes the temporary PFX.

## Produce a Store installer

1. Run the normal release workflow for a version and let it create the draft GitHub Release.
2. Open **Actions → Build signed Microsoft Store installer → Run workflow**.
3. Enter the exact existing tag, such as `app-v0.1.6`.
4. Download and smoke-test the workflow artifact on a clean Windows 10/11 VM.
5. Publish the GitHub Release. Do not replace the Store MSI after submitting its URL to Microsoft.

The resulting asset is named `Skibidibi.Git_<version>_x64_en-US_store.msi`. The workflow refuses to
overwrite an existing versioned asset.

## Partner Center values

### Properties

| Field | Value |
| --- | --- |
| Category | **Developer tools** |
| Product accesses personal information | **Yes** — repositories can contain author names/emails; GitHub integration processes account and PR data |
| Privacy policy URL | `https://github.com/rbrzoska/skibidibi-git/blob/main/PRIVACY.md` |
| Website | `https://github.com/rbrzoska/skibidibi-git` |
| Support contact info | `https://github.com/rbrzoska/skibidibi-git/issues` |

For system requirements, leave optional hardware unspecified. Mark **Keyboard** and **Mouse** as
recommended. Do not mark touch, camera, NFC, Bluetooth, telephony, microphone, dedicated GPU, or a
DirectX version as required.

Use this additional system requirement in the Store listing:

> Requires 64-bit Windows 10 or Windows 11 and Git installed and available on PATH. Network access
> is optional for remote Git operations, GitHub integration, update checks, and AI CLI features.

### Package details

| Field | Value |
| --- | --- |
| Package URL | `https://github.com/rbrzoska/skibidibi-git/releases/download/app-v<version>/Skibidibi.Git_<version>_x64_en-US_store.msi` |
| Architecture | **x64** |
| Installer parameters | `/qn /norestart` |
| Languages | **English (United States)** |
| App type | **MSI** |

Use the actual released version in both URL positions and verify the URL in a private browser
window after the GitHub Release is public. MSI uses the standard Windows Installer silent
parameters; UAC is permitted for the per-machine installation.

### Certification notes

> Skibidibi Git is a local-first desktop Git client. It requires a system Git executable on PATH.
> The submitted x64 MSI installs per-machine, supports silent installation with `/qn /norestart`,
> embeds the offline WebView2 runtime, and is Authenticode signed. The app makes network requests
> only for user-configured Git remotes, GitHub integration, update checks, and optional installed AI
> CLI integrations. No telemetry or advertising service is included.

### Availability, declarations, and age rating

- Select **Free** unless the distribution model changes.
- Choose the intended markets explicitly; **all markets** is appropriate only if support and legal
  availability are intended globally.
- The app does not install non-Microsoft drivers or NT services.
- Do not claim that accessibility guidelines have been fully tested until a dedicated audit passes.
- Do not declare pen/ink, camera, microphone, location, commerce, advertising, gambling, violence,
  sexual content, controlled substances, or social/user-to-user content features.
- The app opens explicit GitHub/support links in the system browser but is not a general web browser.
  Answer age-rating questions from the behavior of the submitted version, not from content that may
  exist in repositories a user independently opens.

### English Store listing

**Short description**

> A fast, local-first desktop Git client for branches, worktrees, pull requests, diffs, conflicts,
> and focused developer workflows.

**Description**

> Skibidibi Git is a compact cross-platform Git client designed for developers who work with many
> branches and worktrees. Inspect commit history and file diffs, stage and discard selected changes,
> commit or amend safely, manage stashes, compare refs, resolve conflicts, and run guarded pull,
> push, merge, reset, cherry-pick, and revert operations.
>
> GitHub integration shows pull requests, comments, reviews, and code changes. Repository cleanup
> tools highlight older branches and worktrees without deleting anything until you confirm it.
> Optional integrations can open a workspace in VS Code or Cursor and generate commit or review
> assistance through supported AI command-line tools already installed on your computer.
>
> Skibidibi Git is local-first: it has no advertising or telemetry service. It uses your system Git
> installation and contacts only the Git remotes and optional services you choose.

**Features**

- Branch and worktree management with favorites and cleanup assistance
- Compact commit history, commit inspector, file history, blame, and ref comparison
- Staging, discard by file or hunk, commit, amend, stash, cherry-pick, revert, and reset
- Guarded fetch, pull, push, merge, rebase, and upstream workflows
- Conflict resolver and submodule status
- GitHub pull request list, details, comments, reviews, and diffs
- Optional Codex, Claude, and Cursor CLI assistance
- Manual and automatic update checks with release notes

**Keywords**

`git`, `github`, `developer tools`, `worktree`, `pull request`, `diff`, `version control`

For **What's new**, copy the matching version section from `RELEASE_NOTES.md`; never describe
features that are not contained in the submitted binary.

**Applicable license terms**

Use the contents of the repository's `LICENSE` file (MIT License), or link to:
`https://github.com/rbrzoska/skibidibi-git/blob/main/LICENSE`.

## Pre-submission checklist

- `pnpm verify:microsoft-store`, `pnpm lint`, `pnpm test`, `pnpm build`, and Rust checks pass.
- The tag and every package/Cargo/Tauri version match.
- `Get-AuthenticodeSignature` reports `Valid` for the application EXE and MSI.
- Silent install succeeds on a clean x64 Windows 10/11 VM.
- The app launches without a console window and detects or clearly reports missing Git.
- Uninstall removes the application while leaving user repositories untouched.
- The versioned HTTPS URL downloads the exact tested MSI and will not be mutated.
- Privacy and support URLs are public.

Microsoft's current MSI/EXE rules require a versioned HTTPS URL, a standalone installer, silent
installation, and trusted signatures on the installer and all contained PE files. The Store build
uses Tauri's required `offlineInstaller` WebView2 mode; this adds roughly 127 MiB.
