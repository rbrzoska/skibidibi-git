# Skibidibi Git Privacy Policy

> Effective date: July 31, 2026

Skibidibi Git is a local-first desktop Git client maintained by the Skibidibi Git project. This
policy explains what information the application processes, where it is stored, and when it may
leave your computer.

## Information processed locally

To provide Git functionality, Skibidibi Git reads information from repositories and worktrees that
you explicitly open. This may include file paths and contents, diffs, commit messages, commit author
names and email addresses, branches, remotes, stashes, submodules, and repository statistics.

The application stores preferences, a catalog of remembered repositories, GitHub account metadata,
and bounded diagnostic events on your device. By default, native settings and diagnostics are kept
in `~/.skibidibi-git`; the diagnostics directory and size limit can be changed in Settings.
Authentication secrets are stored in the operating system credential store and are not written to
diagnostic logs.

## Network services and third parties

Skibidibi Git does not operate an analytics, advertising, telemetry, or user-profile service. Data
leaves your computer only when you initiate or enable a feature that needs another service:

- Git fetch, pull, push, clone, and similar operations communicate with the remotes you configured.
- GitHub integration communicates with GitHub to authenticate and retrieve repository, pull
  request, review, and comment information. GitHub CLI authentication remains managed by GitHub CLI.
- Update checks communicate with the public Skibidibi Git repository on GitHub Releases.
- Optional Codex, Claude, or Cursor CLI features send the selected prompt and relevant Git context
  to the provider configured in that CLI. Those providers process the data under their own terms
  and privacy policies. Review the diff before using an AI feature with confidential source code.

GitHub's processing is described in the
[GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
Each optional AI provider publishes its own privacy policy and retention controls.

## Diagnostics

Diagnostics are disabled from collecting source code, diffs, prompts, tokens, CLI output, and raw
CLI errors. The local diagnostic file is size-bounded and can be viewed or cleared from Settings.
Skibidibi Git does not upload diagnostics automatically. If you choose to attach diagnostics to a
support request, you control what is shared.

## Retention and deletion

Local data remains on your device until you remove it. You can clear diagnostic logs in Settings,
disconnect GitHub accounts from the application, remove remembered repositories, or delete the
configured Skibidibi Git data directory after closing the application. Removing application data
does not delete your Git repositories. Credentials managed by GitHub CLI or an operating system
credential manager must be removed using those tools.

## Children

Skibidibi Git is a developer tool and is not directed to children. The project does not knowingly
collect children's personal information.

## Changes

Material changes to this policy will be committed to this repository and noted in release notes
when they affect application behavior. The effective date above will be updated.

## Contact

For privacy or support questions, open an issue at
[github.com/rbrzoska/skibidibi-git/issues](https://github.com/rbrzoska/skibidibi-git/issues).
Do not include access tokens, private source code, repository URLs, personal data, or other secrets
in a public issue.
