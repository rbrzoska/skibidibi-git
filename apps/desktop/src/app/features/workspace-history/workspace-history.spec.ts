import { ComponentFixture, TestBed } from '@angular/core/testing';
import { ActivatedRoute, Router, provideRouter } from '@angular/router';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RepositoryCommitDetailResponse,
  type RepositoryCommitSummary,
  type RepositoryNavigationResponse,
  type RepositoryStatusResponse,
} from '../../core/ipc/desktop-ipc';
import { AiSupportStore, DEFAULT_AI_COMMIT_PROMPT } from '../../core/ai-support/ai-support.store';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../core/github';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { branchExpansionStorageKey } from './branch-expansion-state';
import { releaseBranchStorageKey } from './release-branch-state';
import { selectPreferredGitHubAccountId, WorkspaceHistory } from './workspace-history';

const rememberedRepository = {
  id: 'skibidibi-git',
  canonicalPath: '/work/skibidibi-git',
  displayName: 'skibidibi-git',
  provider: 'github' as const,
  transport: 'ssh' as const,
  hostedIdentity: { host: 'github.com', owner: 'rbrzoska', name: 'skibidibi-git' },
  availability: 'available' as const,
  gitHealth: { state: 'healthy' as const, issue: null, checkedAt: 10 },
  githubHealth: { state: 'healthy' as const, issue: null, checkedAt: 10 },
  pinned: false,
  openCount: 1,
  lastOpenedAt: 10,
  createdAt: 1,
  updatedAt: 10,
};

const commits: readonly RepositoryCommitSummary[] = [
  {
    oid: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    parents: ['parent-a'],
    author: { name: 'Ada', email: 'ada@example.test', authoredAt: '2023-11-14T22:13:20Z' },
    summary: 'Add repository history',
    refs: ['HEAD -> main'],
  },
  {
    oid: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
    parents: [],
    author: { name: 'Grace', email: 'grace@example.test', authoredAt: '2023-11-03T08:26:40Z' },
    summary: 'Create desktop shell',
    refs: [],
  },
];

function detailFor(commit: RepositoryCommitSummary): RepositoryCommitDetailResponse {
  return {
    ...commit,
    fullMessage: `${commit.summary}\n\nCommit body`,
    files: [
      {
        path: 'src/history.ts',
        oldPath: null,
        status: 'modified',
        additions: 12,
        deletions: 3,
        binary: false,
      },
    ],
  };
}

function repositoryStatus(): RepositoryStatusResponse {
  return {
    indexFingerprint: 'index-before',
    worktreeFingerprint: 'worktree-before',
    branch: {
      oid: 'abc',
      head: 'main',
      upstream: 'origin/main',
      ahead: 1,
      behind: 0,
      detached: false,
      unborn: false,
    },
    entries: [
      {
        kind: 'untracked' as const,
        path: 'new.ts',
        originalPath: null,
        indexStatus: 'unmodified' as const,
        worktreeStatus: 'untracked' as const,
        submodule: null,
      },
      {
        kind: 'ordinary' as const,
        path: 'app.ts',
        originalPath: null,
        indexStatus: 'unmodified' as const,
        worktreeStatus: 'modified' as const,
        submodule: null,
      },
    ],
  };
}

function requestedWorkingTreeStatus() {
  const base = repositoryStatus();
  return {
    ...base,
    entries: [
      { kind: 'untracked' as const, path: 'added-a.ts', originalPath: null, indexStatus: 'unmodified' as const, worktreeStatus: 'untracked' as const, submodule: null },
      { kind: 'ordinary' as const, path: 'added-b.ts', originalPath: null, indexStatus: 'added' as const, worktreeStatus: 'unmodified' as const, submodule: null },
      { kind: 'ordinary' as const, path: 'modified.ts', originalPath: null, indexStatus: 'unmodified' as const, worktreeStatus: 'modified' as const, submodule: null },
      { kind: 'renamedOrCopied' as const, path: 'renamed.ts', originalPath: 'before.ts', indexStatus: 'renamed' as const, worktreeStatus: 'unmodified' as const, submodule: null },
      ...['deleted-a.ts', 'deleted-b.ts', 'deleted-c.ts', 'deleted-d.ts'].map((path) => ({ kind: 'ordinary' as const, path, originalPath: null, indexStatus: 'deleted' as const, worktreeStatus: 'unmodified' as const, submodule: null })),
    ],
  };
}

function navigationWithStash() {
  return {
    branches: [
      { kind: 'local' as const, fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 1, behind: 0, upstreamGone: false, symbolicTarget: null },
      { kind: 'remote' as const, fullName: 'refs/remotes/origin/main', name: 'origin/main', oid: 'remote-abc', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
    ],
    worktrees: [],
    stashes: [{ oid: 'stash-oid', selector: 'stash@{0}', message: 'Saved work', author: 'Ada', authoredAt: '2026-07-15T12:00:00Z' }],
  };
}

const textConflict = {
  path: 'src/conflicted.ts',
  base: { oid: 'base-oid', mode: '100644' },
  ours: { oid: 'ours-oid', mode: '100644' },
  theirs: { oid: 'theirs-oid', mode: '100644' },
};

function conflictedStatus() {
  return {
    ...repositoryStatus(),
    entries: [{
      kind: 'unmerged' as const,
      path: textConflict.path,
      originalPath: null,
      indexStatus: 'unmerged' as const,
      worktreeStatus: 'unmerged' as const,
      submodule: null,
    }],
  };
}

function conflictDetail(binary = false) {
  return {
    path: textConflict.path,
    base: { identity: textConflict.base, content: binary ? null : 'base\n', binary },
    ours: { identity: textConflict.ours, content: binary ? null : 'ours\n', binary },
    theirs: { identity: textConflict.theirs, content: binary ? null : 'theirs\n', binary },
    workingContent: binary ? null : 'ours\n<<<<<<<\ntheirs\n',
    workingBinary: binary,
  };
}

describe('WorkspaceHistory', () => {
  it('prefers a connected GitHub CLI account for the repository host', () => {
    expect(selectPreferredGitHubAccountId([
      { id: 'pat', login: 'ada', host: 'github.com', avatarUrl: null, state: 'connected', authKind: 'personalAccessToken' },
      { id: 'oauth', login: 'ada', host: 'github.com', avatarUrl: null, state: 'connected', authKind: 'oAuthDevice' },
      { id: 'cli-other', login: 'ada', host: 'enterprise.example', avatarUrl: null, state: 'connected', authKind: 'gitHubCli' },
      { id: 'cli', login: 'ada', host: 'GITHUB.COM', avatarUrl: null, state: 'connected', authKind: 'gitHubCli' },
      { id: 'cli-stale', login: 'ada', host: 'github.com', avatarUrl: null, state: 'authenticationRequired', authKind: 'gitHubCli' },
    ], 'github.com')).toBe('cli');
  });

  beforeEach(() => {
    globalThis.localStorage.removeItem(branchExpansionStorageKey('skibidibi-git', 'local'));
    globalThis.localStorage.removeItem(branchExpansionStorageKey('skibidibi-git', 'remote'));
    globalThis.localStorage.removeItem('skibidibi-git.workspace.current-only.skibidibi-git');
    globalThis.localStorage.removeItem('skibidibi-git.workspace.auto-fetch.skibidibi-git');
    globalThis.localStorage.removeItem('skibidibi-git.workspace.live-changes.skibidibi-git');
    globalThis.localStorage.removeItem(releaseBranchStorageKey('skibidibi-git'));
    globalThis.localStorage.removeItem('skibidibi-git.ai-support.v1');
  });

  async function createFixture(
    invokeImplementation: (command: string, request: unknown) => Promise<unknown>,
  ): Promise<{ fixture: ComponentFixture<WorkspaceHistory>; invoke: ReturnType<typeof vi.fn> }> {
    const invoke = vi.fn().mockImplementation(invokeImplementation);
    const ipc = { invoke } as unknown as DesktopIpcClient;
    const githubBridge: GitHubBridge = {
      githubListAccounts: async () => [],
      githubStartDeviceFlow: async () => { throw new Error('not used'); },
      githubPollDeviceFlow: async () => { throw new Error('not used'); },
      githubCancelDeviceFlow: async () => ({ cancelled: false }),
      githubOpenDeviceVerification: async () => undefined,
      githubConnectPat: async () => { throw new Error('not used'); },
      githubConnectCli: async () => { throw new Error('not used'); },
      githubDisconnectAccount: async () => ({ disconnected: false }),
      githubListRepositories: async () => ({ repositories: [], nextCursor: null }),
      githubListPullRequests: async () => ({ pullRequests: [], nextCursor: null }),
      githubPullRequestDetail: async () => { throw new Error('not used'); },
    };
    await TestBed.configureTestingModule({
      imports: [WorkspaceHistory],
      providers: [
        provideRouter([]),
        { provide: DESKTOP_IPC, useValue: ipc },
        { provide: GITHUB_BRIDGE, useValue: githubBridge },
        GitHubAccountStore,
        {
          provide: ActivatedRoute,
          useValue: { snapshot: { paramMap: { get: () => 'skibidibi-git' } } },
        },
      ],
    }).compileComponents();

    const fixture = TestBed.createComponent(WorkspaceHistory);
    fixture.detectChanges();
    await vi.waitFor(() => {
      fixture.detectChanges();
      const element = fixture.nativeElement as HTMLElement;
      const historySettled =
        element.querySelector('.commit-row') !== null ||
        element.textContent?.includes('History unavailable') ||
        element.textContent?.includes('No commits found');
      expect(historySettled).toBe(true);
    });
    return { fixture, invoke };
  }

  function statusStoreFor(fixture: ComponentFixture<WorkspaceHistory>): RepositoryStatusStore {
    return fixture.debugElement.injector.get(RepositoryStatusStore);
  }

  function aiSupportFor(fixture: ComponentFixture<WorkspaceHistory>): AiSupportStore {
    return fixture.debugElement.injector.get(AiSupportStore);
  }

  function enableCodexCommitGeneration(fixture: ComponentFixture<WorkspaceHistory>): void {
    const aiSupport = aiSupportFor(fixture);
    aiSupport.providerStatuses.set([
      { provider: 'codex', displayName: 'Codex', available: true, version: '1.0.0', detail: null },
      { provider: 'claude', displayName: 'Claude Code', available: false, version: null, detail: null },
      { provider: 'cursor', displayName: 'Cursor', available: false, version: null, detail: null },
    ]);
    aiSupport.setProviderEnabled('codex', true);
    fixture.detectChanges();
  }

  function showStagedCommitComposer(fixture: ComponentFixture<WorkspaceHistory>) {
    const stagedStatus = {
      ...repositoryStatus(),
      entries: [{
        kind: 'ordinary' as const,
        path: 'app.ts',
        originalPath: null,
        indexStatus: 'modified' as const,
        worktreeStatus: 'unmodified' as const,
        submodule: null,
      }],
    };
    statusStoreFor(fixture).state.set({ kind: 'ready', status: stagedStatus });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    return stagedStatus;
  }

  function defaultIpc(command: string, request: unknown): Promise<unknown> {
    if (command === 'ai_cli_status') {
      return Promise.resolve({ statuses: [] });
    }
    if (command === 'ai_generate_commit_message') {
      return Promise.resolve({
        message: 'Update staged changes',
        indexFingerprint: 'index-before',
        worktreeFingerprint: 'worktree-before',
      });
    }
    if (command === 'list_remembered_repositories') {
      return Promise.resolve([rememberedRepository]);
    }
    if (command === 'list_repository_relations') {
      return Promise.resolve([]);
    }
    if (command === 'repository_submodules') {
      return Promise.resolve({ submodules: [] });
    }
    if (command === 'repository_status') {
      return Promise.resolve(repositoryStatus());
    }
    if (command === 'repository_history' || command === 'repository_branch_history') {
      const cursor = (request as { cursor: string | null }).cursor;
      return Promise.resolve(
        cursor === null
          ? { commits, nextCursor: 'page-2' }
          : { commits: [{ ...commits[1], oid: 'cccccccccccccccccccccccccccccccccccccccc' }], nextCursor: null },
      );
    }
    if (command === 'repository_navigation') {
      return Promise.resolve({
        branches: [
          { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 1, behind: 0, upstreamGone: false, symbolicTarget: null },
          { kind: 'local', fullName: 'refs/heads/rb/feature', name: 'rb/feature', oid: 'def', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          { kind: 'remote', fullName: 'refs/remotes/origin/main', name: 'origin/main', oid: 'abc', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
        ],
        worktrees: [{ path: '/work/feature-tree', head: 'def', branch: 'rb/feature', detached: false, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null }],
        stashes: [],
      });
    }
    if (command === 'repository_worktree_dirty_states') {
      return Promise.resolve({
        states: [{
          branchFullName: 'refs/heads/rb/feature',
          worktreePath: '/work/feature-tree',
          dirty: true,
          changeCount: 3,
          errorMessage: null,
        }],
      });
    }
    if (command === 'repository_merge_branch') {
      return Promise.resolve({
        state: 'succeeded',
        headBefore: 'abc',
        headAfter: 'def',
        status: { ...repositoryStatus(), entries: [] },
        autoStash: {
          create: 'created',
          stash: { oid: 'stash-auto', selector: 'stash@{0}' },
          restore: 'applied',
          cleanup: 'dropped',
          createError: null,
          restoreError: null,
          cleanupError: null,
        },
        errorMessage: null,
        mutationMayHaveOccurred: false,
      });
    }
    if (command === 'repository_pull_inactive_branch') {
      return Promise.resolve({
        branchFullName: 'refs/heads/rb/feature',
        headBefore: 'def',
        headAfter: 'fed',
        upstream: 'origin/rb/feature',
        changed: true,
      });
    }
    if (command === 'switch_repository_branch') {
      return Promise.resolve({
        fullName: 'refs/heads/rb/feature',
        name: 'rb/feature',
        head: 'def',
        changed: true,
        stashCreated: false,
        operationSucceeded: true,
        operationError: null,
        autoStash: {
          create: 'notRequested',
          stash: null,
          restore: 'notRequired',
          cleanup: 'notRequired',
          createError: null,
          restoreError: null,
          cleanupError: null,
        },
      });
    }
    if (command === 'repository_fetch') {
      return Promise.resolve({ fetchedAt: '2026-07-15T12:00:00Z' });
    }
    if (command === 'repository_push_analysis') {
      return Promise.resolve({ branch: 'main', head: 'abc', upstream: 'origin/main', remote: 'origin', remoteRef: 'refs/heads/main', ahead: 1, behind: 0, readiness: 'ready' });
    }
    if (command === 'repository_pull') {
      const autoStash = (request as { operation: { autoStash: { message: string } | null } }).operation.autoStash;
      return Promise.resolve({
        state: 'succeeded',
        headBefore: 'abc',
        headAfter: 'def',
        status: { ...repositoryStatus(), branch: { ...repositoryStatus().branch, oid: 'def' } },
        autoStash: autoStash === null
          ? { create: 'notRequested', stash: null, restore: 'notRequired', cleanup: 'notRequired', createError: null, restoreError: null, cleanupError: null }
          : { create: 'created', stash: { oid: 'stash-auto', selector: 'stash@{0}' }, restore: 'applied', cleanup: 'dropped', createError: null, restoreError: null, cleanupError: null },
        errorMessage: null,
      });
    }
    if (command === 'repository_push') {
      return Promise.resolve({
        pushed: true,
        analysis: { branch: 'main', head: 'abc', upstream: 'origin/main', remote: 'origin', remoteRef: 'refs/heads/main', ahead: 0, behind: 0, readiness: 'upToDate' },
        status: repositoryStatus(),
      });
    }
    if (command === 'repository_set_upstream') {
      return Promise.resolve({ upstream: 'origin/main', status: repositoryStatus() });
    }
    if (command === 'repository_conflicts') {
      return Promise.resolve({ files: [], status: repositoryStatus() });
    }
    if (command === 'repository_resolve_conflict') {
      return Promise.resolve({ resolved: true, status: repositoryStatus(), errorMessage: null, mutationMayHaveOccurred: false });
    }
    if (command === 'create_repository_branch') {
      const operation = (request as { operation: { name: string | null; source: { kind: string; fullName?: string } } }).operation;
      const derivedName = operation.name ?? operation.source.fullName?.replace(/^refs\/remotes\/origin\//, '') ?? 'unknown';
      return Promise.resolve({
        fullName: `refs/heads/${derivedName}`,
        name: derivedName,
        head: 'abc',
        upstream: operation.source.kind === 'remoteTracking' ? 'origin/main' : null,
      });
    }
    if (command === 'repository_push_stash') {
      return Promise.resolve({
        state: 'created',
        stash: { oid: 'stash-new', selector: 'stash@{0}' },
        status: { ...repositoryStatus(), entries: [] },
        errorMessage: null,
        mutationOid: 'stash-new',
        mutationMayHaveOccurred: false,
      });
    }
    if (command === 'repository_apply_stash') {
      return Promise.resolve({
        stash: { oid: 'stash-oid', selector: 'stash@{0}' },
        restore: 'applied',
        cleanup: 'notRequired',
        status: repositoryStatus(),
        errorMessage: null,
        mutationMayHaveOccurred: false,
      });
    }
    if (command === 'repository_pop_stash') {
      return Promise.resolve({
        stash: { oid: 'stash-oid', selector: 'stash@{0}' },
        restore: 'applied',
        cleanup: 'dropped',
        status: repositoryStatus(),
        restoreError: null,
        cleanupError: null,
        mutationMayHaveOccurred: false,
      });
    }
    if (command === 'repository_drop_stash') {
      return Promise.resolve({
        stash: { oid: 'stash-oid', selector: 'stash@{0}' },
        cleanup: 'dropped',
        errorMessage: null,
        mutationMayHaveOccurred: false,
      });
    }
    if (command === 'delete_repository_branch') {
      return Promise.resolve({ changed: true });
    }
    if (command === 'remove_repository_worktree') {
      return Promise.resolve({
        path: '/work/feature-tree',
        branchFullName: 'refs/heads/rb/feature',
        worktreeRemoved: true,
        worktreeRemovalError: null,
        branchDeleted: true,
        branchDeletionError: null,
        mode: 'safe',
        stash: null,
      });
    }
    if (command === 'repository_commit_detail') {
      const oid = (request as { oid: string }).oid;
      const commit = commits.find((candidate) => candidate.oid === oid) ?? commits[0];
      return Promise.resolve(detailFor(commit));
    }
    if (command === 'repository_file_diff') {
      const { oid, path } = request as { oid: string; path: string };
      return Promise.resolve({
        oid,
        path,
        binary: false,
        patch: `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-old line\n+new line`,
      });
    }
    if (command === 'repository_stash_detail') {
      const { oid } = request as { oid: string };
      return Promise.resolve({
        oid,
        files: [
          {
            source: 'tracked',
            path: 'new name.txt',
            oldPath: 'old name.txt',
            status: 'renamed',
            additions: 2,
            deletions: 1,
            binary: false,
          },
          {
            source: 'untracked',
            path: 'snapshot.bin',
            oldPath: null,
            status: 'added',
            additions: null,
            deletions: null,
            binary: true,
          },
        ],
      });
    }
    if (command === 'repository_stash_file_diff') {
      const { oid, source, path } = request as { oid: string; source: string; path: string };
      return Promise.resolve({
        oid,
        source,
        path,
        binary: false,
        truncated: false,
        patch: `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-old stash line\n+new stash line`,
      });
    }
    if (command === 'repository_working_tree_file_diff') {
      const { path } = request as { path: string };
      const patch = `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-old working line\n+new working line`;
      return Promise.resolve({
        path,
        binary: false,
        truncated: false,
        patch,
        unstagedPatch: patch + '\n',
      });
    }
    if (command === 'repository_apply_index_change') {
      return Promise.resolve({
        changed: true,
        status: { ...repositoryStatus(), indexFingerprint: 'index-after' },
      });
    }
    if (command === 'repository_discard_worktree_changes' || command === 'repository_discard_worktree_hunk') {
      return Promise.resolve({
        discardedEntries: 1,
        deletedUntrackedFiles: 0,
        status: { ...repositoryStatus(), worktreeFingerprint: 'worktree-after-discard', entries: [] },
      });
    }
    if (command === 'repository_create_commit') {
      return Promise.resolve({
        oid: 'dddddddddddddddddddddddddddddddddddddddd',
        status: { ...repositoryStatus(), indexFingerprint: 'index-after-commit', entries: [] },
      });
    }
    if (command === 'repository_amend_commit') {
      return Promise.resolve({
        previousOid: 'abc',
        oid: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
        state: 'succeeded',
        errorMessage: null,
        status: {
          ...repositoryStatus(),
          branch: {
            ...repositoryStatus().branch,
            oid: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
          },
          indexFingerprint: 'index-after-amend',
          entries: [],
        },
      });
    }
    if (
      command === 'repository_cherry_pick_commit' ||
      command === 'repository_revert_commit' ||
      command === 'repository_reset_commit'
    ) {
      const targetOid = (request as { operation: { targetOid: string } }).operation.targetOid;
      return Promise.resolve({
        state: 'succeeded',
        targetOid,
        headBefore: 'abc',
        headAfter: 'f'.repeat(40),
        status: {
          ...repositoryStatus(),
          branch: { ...repositoryStatus().branch, oid: 'f'.repeat(40) },
          entries: [],
        },
        errorMessage: null,
        mutationMayHaveOccurred: true,
      });
    }
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  }

  async function defaultIpcWithoutCheckedOutFeature(
    command: string,
    request: unknown,
  ): Promise<unknown> {
    const response = await defaultIpc(command, request);
    return command === 'repository_navigation'
      ? { ...(response as RepositoryNavigationResponse), worktrees: [] }
      : response;
  }

  it('hides AI commit-message generators until a locally available CLI is enabled', async () => {
    const { fixture } = await createFixture(defaultIpc);
    showStagedCommitComposer(fixture);

    expect(fixture.nativeElement.querySelector('.ai-commit-action')).toBeNull();
  });

  it('shows an enabled locally available AI commit-message generator', async () => {
    const { fixture } = await createFixture(defaultIpc);
    showStagedCommitComposer(fixture);
    enableCodexCommitGeneration(fixture);

    const button = fixture.nativeElement.querySelector(
      '[data-ai-provider="codex"]',
    ) as HTMLButtonElement;
    expect(button.getAttribute('aria-label')).toBe('Generate a commit message with Codex');
    expect((button.querySelector('img') as HTMLImageElement).getAttribute('src')).toBe('ai-providers/codex.png');
    expect(button.closest('.commit-composer-heading')).not.toBeNull();
    expect(button.disabled).toBe(false);
  });

  it('sends the current repository state to the enabled AI CLI and fills only the commit message', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    showStagedCommitComposer(fixture);
    enableCodexCommitGeneration(fixture);

    (fixture.nativeElement.querySelector('[data-ai-provider="codex"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('ai_generate_commit_message', {
      repositoryId: 'skibidibi-git',
      provider: 'codex',
      promptTemplate: DEFAULT_AI_COMMIT_PROMPT,
      expectedHead: 'abc',
      indexFingerprint: 'index-before',
      worktreeFingerprint: 'worktree-before',
    });
    expect((fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement).value)
      .toBe('Update staged changes');
  });

  it('rejects an AI result when the staged changes changed while it was generating', async () => {
    let resolveGeneration: ((value: unknown) => void) | undefined;
    const { fixture } = await createFixture((command, request) =>
      command === 'ai_generate_commit_message'
        ? new Promise((resolve) => { resolveGeneration = resolve; })
        : defaultIpc(command, request),
    );
    showStagedCommitComposer(fixture);
    enableCodexCommitGeneration(fixture);

    (fixture.nativeElement.querySelector('[data-ai-provider="codex"]') as HTMLButtonElement).click();
    statusStoreFor(fixture).state.set({
      kind: 'ready',
      status: { ...repositoryStatus(), indexFingerprint: 'index-after' },
    });
    resolveGeneration?.({
      message: 'This response is stale',
      indexFingerprint: 'index-before',
      worktreeFingerprint: 'worktree-before',
    });
    await fixture.whenStable();
    fixture.detectChanges();

    expect((fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement).value).toBe('');
    expect(fixture.nativeElement.querySelector('.commit-message-generation-error')?.textContent)
      .toContain('staged changes changed');
  });

  it('shows the AI CLI generation error without attempting to create a commit', async () => {
    const { fixture, invoke } = await createFixture((command, request) =>
      command === 'ai_generate_commit_message'
        ? Promise.reject(new Error('Codex is not signed in'))
        : defaultIpc(command, request),
    );
    showStagedCommitComposer(fixture);
    enableCodexCommitGeneration(fixture);

    (fixture.nativeElement.querySelector('[data-ai-provider="codex"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.commit-message-generation-error')?.textContent)
      .toContain('Codex is not signed in');
    expect(invoke).not.toHaveBeenCalledWith('repository_create_commit', expect.anything());
  });

  it('renders real commit summaries in the three-column workspace', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const element = fixture.nativeElement as HTMLElement;

    expect(element.querySelector('.navigation')).toBeTruthy();
    expect(element.querySelector('.history')).toBeTruthy();
    expect(element.querySelector('.inspector')).toBeTruthy();
    expect(element.querySelectorAll('.commit-row')).toHaveLength(2);
    expect(element.querySelector('.commit-list')?.classList).toContain('compact');
    expect(element.querySelectorAll('.commit-row .graph')).toHaveLength(2);
    expect(element.textContent).toContain('Add repository history');
    expect(element.textContent).not.toContain('History backend is next');

    (element.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    const fileHistoryLink = element.querySelector('.file-history-link') as HTMLAnchorElement;
    expect(fileHistoryLink.getAttribute('href')).toContain('/workspace/skibidibi-git/file-history');
    expect(fileHistoryLink.getAttribute('href')).toContain(`oid=${'a'.repeat(40)}`);
    expect(fileHistoryLink.getAttribute('href')).toContain('path=');
  });

  it('confirms and cherry-picks the inspected non-merge commit with the current repository snapshot', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const cherryPick = fixture.nativeElement.querySelector(
      'button[title="Apply this commit to the current branch"]',
    ) as HTMLButtonElement;
    expect(cherryPick.disabled).toBe(false);
    cherryPick.click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-title')?.textContent)
      .toContain('Cherry-pick');

    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_cherry_pick_commit', {
      repositoryId: 'skibidibi-git',
      operation: {
        targetOid: 'a'.repeat(40),
        precondition: {
          expectedHead: 'abc',
          expectedHeadName: 'main',
          expectedDetached: false,
          expectedUnborn: false,
          expectedIndexFingerprint: 'index-before',
          expectedWorktreeFingerprint: 'worktree-before',
        },
      },
    });
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent)
      .toContain('Cherry-picked');
  });

  it('makes hard reset visually destructive and sends the explicit backend confirmation', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const mode = fixture.nativeElement.querySelector('#commit-reset-mode') as HTMLSelectElement;
    mode.value = 'hard';
    mode.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.reset-action') as HTMLButtonElement).click();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.destructive-action-description')?.textContent)
      .toContain('permanently discard tracked staged and unstaged changes');
    expect(fixture.nativeElement.querySelector('#confirm-destructive-action')?.classList)
      .toContain('danger');

    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_reset_commit', {
      repositoryId: 'skibidibi-git',
      operation: {
        targetOid: 'a'.repeat(40),
        mode: 'hard',
        confirmHardReset: true,
        precondition: expect.objectContaining({
          expectedHead: 'abc',
          expectedIndexFingerprint: 'index-before',
          expectedWorktreeFingerprint: 'worktree-before',
        }),
      },
    });
  });

  it('inspects a stash by immutable OID, opens its rename diff, and returns to the stash context', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_navigation'
        ? Promise.resolve(navigationWithStash())
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const inspect = fixture.nativeElement.querySelector('[aria-label="Inspect stash@{0}"]') as HTMLButtonElement;
    inspect.click();
    await fixture.whenStable();
    fixture.detectChanges();

    const inspector = fixture.nativeElement.querySelector('.inspector') as HTMLElement;
    expect(inspector.getAttribute('aria-label')).toBe('Stash inspector');
    expect(inspector.textContent).toContain('Saved work');
    expect(inspector.textContent).toContain('new name.txt');
    expect(inspector.textContent).toContain('untracked');
    expect(invoke).toHaveBeenCalledWith('repository_stash_detail', {
      repositoryId: 'skibidibi-git',
      oid: 'stash-oid',
    });

    const changedFile = inspector.querySelector('.changed-file') as HTMLButtonElement;
    changedFile.focus();
    changedFile.click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain('new stash line');
    expect(invoke).toHaveBeenCalledWith('repository_stash_file_diff', {
      repositoryId: 'skibidibi-git',
      oid: 'stash-oid',
      source: 'tracked',
      path: 'new name.txt',
      oldPath: 'old name.txt',
    });

    (fixture.nativeElement.querySelector('.close-diff') as HTMLButtonElement).click();
    await Promise.resolve();
    fixture.detectChanges();
    expect((fixture.nativeElement.querySelector('.inspector') as HTMLElement).getAttribute('aria-label')).toBe(
      'Stash inspector',
    );
    expect(fixture.nativeElement.querySelector('.stash-overview')?.textContent).toContain('Saved work');
    expect(globalThis.document.activeElement).toBe(changedFile);
  });

  it('ignores stale stash detail and diff responses after selecting another stash OID', async () => {
    const stashes = [
      { oid: 'stash-first', selector: 'stash@{0}', message: 'First stash', author: 'Ada', authoredAt: '2026-07-15T12:00:00Z' },
      { oid: 'stash-second', selector: 'stash@{1}', message: 'Second stash', author: 'Grace', authoredAt: '2026-07-14T12:00:00Z' },
    ];
    let resolveFirstDetail!: (value: unknown) => void;
    let resolveFirstDiff!: (value: unknown) => void;
    const firstDetail = new Promise((resolve) => { resolveFirstDetail = resolve; });
    const firstDiff = new Promise((resolve) => { resolveFirstDiff = resolve; });
    const stashFile = {
      source: 'tracked', path: 'first.txt', oldPath: null, status: 'modified', additions: 1, deletions: 1, binary: false,
    };
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve({ ...navigationWithStash(), stashes });
      }
      if (command === 'repository_stash_detail') {
        const oid = (request as { oid: string }).oid;
        return oid === stashes[0].oid
          ? firstDetail
          : Promise.resolve({ oid, files: [{ ...stashFile, path: 'second.txt' }] });
      }
      if (command === 'repository_stash_file_diff') {
        return firstDiff;
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    const inspect = fixture.nativeElement.querySelectorAll('.stash-inspect') as NodeListOf<HTMLButtonElement>;
    inspect[0].click();
    inspect[1].click();
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.stash-overview')?.textContent).toContain('Second stash');
    });
    resolveFirstDetail({ oid: stashes[0].oid, files: [stashFile] });
    await Promise.resolve();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.stash-overview')?.textContent).toContain('Second stash');
    expect(fixture.nativeElement.querySelector('.changed-files')?.textContent).toContain('second.txt');

    inspect[0].click();
    resolveFirstDetail({ oid: stashes[0].oid, files: [stashFile] });
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.changed-file')).toBeTruthy();
    });
    (fixture.nativeElement.querySelector('.changed-file') as HTMLButtonElement).click();
    inspect[1].click();
    resolveFirstDiff({
      oid: stashes[0].oid,
      source: 'tracked',
      path: 'first.txt',
      binary: false,
      truncated: false,
      patch: 'diff --git a/first.txt b/first.txt\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-stale\n+stale diff',
    });
    await Promise.resolve();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.diff-view')).toBeNull();
  });

  it('invalidates a selected stash when a mutation refresh no longer reports its OID', async () => {
    let navigationCalls = 0;
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        navigationCalls += 1;
        return Promise.resolve(navigationCalls === 1 ? navigationWithStash() : { ...navigationWithStash(), stashes: [] });
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.stash-inspect') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('[aria-label="Drop stash@{0}"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.stash-overview')).toBeNull();
    expect((fixture.nativeElement.querySelector('.inspector') as HTMLElement).getAttribute('aria-label')).toBe(
      'Commit inspector',
    );
    expect(fixture.nativeElement.querySelector('.selected.stash-inspect')).toBeNull();
  });

  it('summarizes working tree changes above commit history', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const statusStore = statusStoreFor(fixture);
    statusStore.setRepositoryPath('/work/skibidibi-git');
    statusStore.state.set({ kind: 'ready', status: repositoryStatus() });
    fixture.detectChanges();
    const summary = fixture.nativeElement.querySelector('.change-stats') as HTMLElement;

    expect(summary.textContent).toContain('2 changed');
    expect(summary.textContent).toContain('+1');
    expect(summary.textContent).toContain('~1');
  });

  it('pins the requested working-tree summary first and inspects files without fake commit metadata', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({ kind: 'ready', status: requestedWorkingTreeStatus() });
    fixture.detectChanges();

    const historyItems = fixture.nativeElement.querySelectorAll(
      '.commit-list > [role="listitem"]',
    ) as NodeListOf<HTMLButtonElement>;
    const workingRow = historyItems[0];
    expect(workingRow.classList).toContain('working-tree-history-row');
    expect(workingRow.textContent).toContain('Working Tree');
    expect(workingRow.textContent).toContain('+2');
    expect(workingRow.textContent).toContain('±1');
    expect(workingRow.textContent).toContain('→1');
    expect(workingRow.textContent).toContain('−4');
    expect(workingRow.textContent).toContain('8 uncommitted · 6 staged');
    expect(workingRow.querySelector('.graph.working-tree-graph > i')).not.toBeNull();
    expect(workingRow.querySelector('.working-tree-dot')).toBeNull();
    expect(workingRow.querySelector('.commit-author')?.textContent).toContain('you');
    expect(workingRow.querySelector('time')?.textContent).toContain('now');
    expect(workingRow.querySelector('code')?.textContent).toContain('—');
    expect(workingRow.getAttribute('aria-current')).toBe('true');

    workingRow.click();
    fixture.detectChanges();
    const inspector = fixture.nativeElement.querySelector('.inspector') as HTMLElement;
    expect(inspector.textContent).toContain('Working tree');
    expect(inspector.textContent).toContain('8 changed files');
    expect(inspector.querySelector('.commit-meta')).toBeNull();
    expect(inspector.querySelector('.commit-message')).toBeNull();
    expect(inspector.textContent).not.toContain('Ada');
    expect(inspector.textContent).not.toContain(commits[0].summary);
    expect(inspector.querySelector('[aria-label="Staged files"]')?.textContent).toContain('renamed.ts');
    expect(inspector.querySelector('[aria-label="Unstaged files"]')?.textContent).toContain('modified.ts');
    expect(inspector.querySelectorAll('.working-tree-file-group')).toHaveLength(2);
    const workingTreeLayout = inspector.querySelector('.working-tree-layout') as HTMLElement;
    expect(inspector.lastElementChild).toBe(workingTreeLayout);
    expect(workingTreeLayout.lastElementChild?.classList).toContain('commit-composer');

    (inspector.querySelector('.changed-file') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain(
      'new working line',
    );
    expect(invoke).toHaveBeenCalledWith('repository_working_tree_file_diff', {
      repositoryId: 'skibidibi-git',
      path: 'renamed.ts',
      oldPath: 'before.ts',
      entryKind: 'renamedOrCopied',
    });
  });

  it('hides the working-tree row when status becomes clean', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({ kind: 'ready', status: { ...repositoryStatus(), entries: [] } });
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.working-tree-history-row')).toBeNull();
    expect(fixture.nativeElement.querySelectorAll('.commit-row')).toHaveLength(2);
  });

  it('keeps staged deletion and untracked replacement diffs distinct at the same path', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({
      kind: 'ready',
      status: {
        ...repositoryStatus(),
        entries: [
          { kind: 'ordinary', path: 'same.txt', originalPath: null, indexStatus: 'deleted', worktreeStatus: 'unmodified', submodule: null },
          { kind: 'untracked', path: 'same.txt', originalPath: null, indexStatus: 'untracked', worktreeStatus: 'untracked', submodule: null },
        ],
      },
    });
    fixture.detectChanges();

    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    let files = fixture.nativeElement.querySelectorAll('.working-tree-files .changed-file') as NodeListOf<HTMLButtonElement>;
    expect(files).toHaveLength(2);

    files[0].click();
    await fixture.whenStable();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.close-diff') as HTMLButtonElement).click();
    fixture.detectChanges();
    files = fixture.nativeElement.querySelectorAll('.working-tree-files .changed-file') as NodeListOf<HTMLButtonElement>;
    files[1].click();
    await fixture.whenStable();

    const requests = invoke.mock.calls
      .filter(([command]) => command === 'repository_working_tree_file_diff')
      .map(([, request]) => request);
    expect(requests).toEqual([
      { repositoryId: 'skibidibi-git', path: 'same.txt', oldPath: null, entryKind: 'ordinary' },
      { repositoryId: 'skibidibi-git', path: 'same.txt', oldPath: null, entryKind: 'untracked' },
    ]);
  });

  it('keeps conflicted files visible but disables unsupported diff actions', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({
      kind: 'ready',
      status: {
        ...repositoryStatus(),
        entries: [
          { kind: 'unmerged', path: 'conflict.txt', originalPath: null, indexStatus: 'unmerged', worktreeStatus: 'unmerged', submodule: null },
          { kind: 'ordinary', path: 'staged.ts', originalPath: null, indexStatus: 'modified', worktreeStatus: 'unmodified', submodule: null },
          { kind: 'ordinary', path: 'unstaged.ts', originalPath: null, indexStatus: 'unmodified', worktreeStatus: 'modified', submodule: null },
        ],
      },
    });
    fixture.detectChanges();

    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const file = fixture.nativeElement.querySelector('.working-tree-files .changed-file') as HTMLButtonElement;
    expect(file.disabled).toBe(true);
    expect(file.title).toContain('conflict resolver');
    expect((fixture.nativeElement.querySelector('.working-tree-file-checkbox') as HTMLInputElement).disabled).toBe(true);
    expect((fixture.nativeElement.querySelector('.commit-composer .primary-action') as HTMLButtonElement).disabled).toBe(true);
    const indexActions = fixture.nativeElement.querySelectorAll(
      '.index-actions button',
    ) as NodeListOf<HTMLButtonElement>;
    expect([...indexActions].every((button) => button.disabled)).toBe(true);
    expect([...indexActions].every((button) => button.title.includes('Resolve all conflicts'))).toBe(true);
    expect(fixture.nativeElement.textContent).toContain(
      'Resolve all conflicts before staging or unstaging changes.',
    );
  });

  it('stages selected files once while a mutation is busy and accepts the returned status', async () => {
    let resolveMutation!: (value: unknown) => void;
    const mutation = new Promise((resolve) => { resolveMutation = resolve; });
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_apply_index_change' ? mutation : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({ kind: 'ready', status: repositoryStatus() });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    const checkbox = fixture.nativeElement.querySelector(
      '[aria-label="Select new.ts"]',
    ) as HTMLInputElement;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const stageSelected = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.index-actions button')]
      .find((button) => button.textContent?.includes('Stage selected')) as HTMLButtonElement;
    stageSelected.click();
    stageSelected.click();
    fixture.detectChanges();

    expect(invoke.mock.calls.filter(([command]) => command === 'repository_apply_index_change')).toHaveLength(1);
    expect(stageSelected.textContent).toContain('Staging…');
    const stageAll = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.index-actions button')]
      .find((button) => button.textContent?.includes('Stage all')) as HTMLButtonElement;
    expect(stageAll.textContent).toContain('Stage all');
    expect((fixture.nativeElement.querySelector('.workspace-bar button') as HTMLButtonElement).disabled).toBe(true);
    expect((fixture.nativeElement.querySelector('.worktree-row') as HTMLButtonElement).disabled).toBe(true);
    expect(invoke).toHaveBeenCalledWith('repository_apply_index_change', {
      repositoryId: 'skibidibi-git',
      operation: {
        action: 'stage',
        selection: {
          scope: 'selected',
          entries: [{ path: 'new.ts', oldPath: null, entryKind: 'untracked' }],
        },
        expectedHead: 'abc',
        expectedHeadName: 'main',
        expectedDetached: false,
        expectedUnborn: false,
        expectedIndexFingerprint: 'index-before',
        expectedWorktreeFingerprint: 'worktree-before',
      },
    });

    resolveMutation({
      changed: true,
      status: {
        ...repositoryStatus(),
        indexFingerprint: 'index-staged',
        entries: [{ kind: 'ordinary', path: 'new.ts', originalPath: null, indexStatus: 'added', worktreeStatus: 'unmodified', submodule: null }],
      },
    });
    await fixture.whenStable();
    fixture.detectChanges();

    expect(statusStore.state().kind).toBe('ready');
    expect((statusStore.state() as { status: { indexFingerprint: string } }).status.indexFingerprint).toBe('index-staged');
    expect((fixture.nativeElement.querySelector('.working-tree-file-checkbox') as HTMLInputElement).checked).toBe(false);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_history')).toHaveLength(1);
  });

  it('confirms and discards selected files with a permanent-deletion warning for untracked files', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({ kind: 'ready', status: repositoryStatus() });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    const checkbox = fixture.nativeElement.querySelector(
      '[aria-label="Select new.ts"]',
    ) as HTMLInputElement;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const discard = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.working-tree-file-group:last-child > header button')]
      .find((button) => button.textContent?.includes('Discard selected')) as HTMLButtonElement;
    discard.click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-description')?.textContent).toContain('permanently deleted');
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_discard_worktree_changes', {
      repositoryId: 'skibidibi-git',
      operation: {
        entries: [{ path: 'new.ts', oldPath: null, entryKind: 'untracked' }],
        expectedHead: 'abc',
        expectedHeadName: 'main',
        expectedDetached: false,
        expectedUnborn: false,
        expectedIndexFingerprint: 'index-before',
        expectedWorktreeFingerprint: 'worktree-before',
      },
    });
  });

  it('confirms and discards one fresh hunk from the working-tree diff', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const path = 'website/front-end-ng/src/app/features/customers/services/customers-dialog.service.ts';
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({
      kind: 'ready',
      status: {
        ...repositoryStatus(),
        entries: [{ kind: 'ordinary', path, originalPath: null, indexStatus: 'unmodified', worktreeStatus: 'modified', submodule: null }],
      },
    });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-files .changed-file') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const discard = fixture.nativeElement.querySelector('.discard-hunk') as HTMLButtonElement;
    expect(discard.textContent).toContain('Discard chunk 1');
    expect(discard.closest('.diff-row.hunk-header')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.diff-toolbar .discard-hunk')).toBeNull();
    const heading = fixture.nativeElement.querySelector('.diff-file-path') as HTMLElement;
    expect(heading.title).toBe(path);
    expect(heading.querySelector('.file-directory')?.textContent).toBe('website/front-end-ng/src/app/features/customers/services');
    expect(heading.querySelector('.file-name')?.textContent).toBe('customers-dialog.service.ts');
    expect(fixture.nativeElement.querySelector('.diff-row.file-header')).toBeNull();
    discard.click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-description')?.textContent).toContain('Discard this change block');
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_discard_worktree_hunk', expect.objectContaining({
      operation: expect.objectContaining({
        entry: { path, oldPath: null, entryKind: 'ordinary' },
        patch: expect.stringContaining('@@ -1 +1 @@'),
      }),
    }));
  });

  it('ignores a pending mutation result after the workspace is destroyed', async () => {
    let resolveMutation!: (value: unknown) => void;
    const mutation = new Promise((resolve) => { resolveMutation = resolve; });
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_apply_index_change' ? mutation : defaultIpc(command, request);
    const { fixture } = await createFixture(ipc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({ kind: 'ready', status: repositoryStatus() });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const checkbox = fixture.nativeElement.querySelector(
      '[aria-label="Select new.ts"]',
    ) as HTMLInputElement;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const stageSelected = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.index-actions button')]
      .find((button) => button.textContent?.includes('Stage selected')) as HTMLButtonElement;
    stageSelected.click();
    fixture.destroy();

    resolveMutation({
      changed: true,
      status: { ...repositoryStatus(), indexFingerprint: 'must-not-be-accepted' },
    });
    await mutation;
    await Promise.resolve();
    await Promise.resolve();

    expect(statusStore.state()).toEqual({ kind: 'ready', status: repositoryStatus() });
  });

  it('stages one unstaged file directly from its section action', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const status = repositoryStatus();
    statusStoreFor(fixture).state.set({ kind: 'ready', status });
    fixture.detectChanges();

    const stageFile = fixture.nativeElement.querySelector(
      '[aria-label="Stage new.ts"]',
    ) as HTMLButtonElement;
    expect(stageFile).toBeTruthy();
    stageFile.click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_apply_index_change', {
      repositoryId: 'skibidibi-git',
      operation: {
        action: 'stage',
        selection: {
          scope: 'selected',
          entries: [{ path: 'new.ts', oldPath: null, entryKind: 'untracked' }],
        },
        expectedHead: 'abc',
        expectedHeadName: 'main',
        expectedDetached: false,
        expectedUnborn: false,
        expectedIndexFingerprint: 'index-before',
        expectedWorktreeFingerprint: 'worktree-before',
      },
    });
  });

  it('keeps the commit message and reconciles file selection after commit creation fails', async () => {
    const stagedStatus = {
      ...repositoryStatus(),
      entries: [{ kind: 'ordinary' as const, path: 'app.ts', originalPath: null, indexStatus: 'modified' as const, worktreeStatus: 'unmodified' as const, submodule: null }],
    };
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_create_commit'
        ? Promise.reject({ message: 'pre-commit hook rejected the commit' })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    statusStoreFor(fixture).state.set({ kind: 'ready', status: stagedStatus });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    const checkbox = fixture.nativeElement.querySelector('.working-tree-file-checkbox') as HTMLInputElement;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change'));
    const textarea = fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement;
    textarea.value = 'Keep this message';
    textarea.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.commit-composer .primary-action') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_create_commit', {
      repositoryId: 'skibidibi-git',
      operation: {
        message: 'Keep this message',
        expectedHead: 'abc',
        expectedHeadName: 'main',
        expectedDetached: false,
        expectedUnborn: false,
        expectedIndexFingerprint: 'index-before',
        expectedWorktreeFingerprint: 'worktree-before',
      },
    });
    expect((fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement).value).toBe('Keep this message');
    expect((fixture.nativeElement.querySelector('.working-tree-file-checkbox') as HTMLInputElement).checked).toBe(false);
    expect(fixture.nativeElement.querySelector('[role="alert"]')?.textContent).toContain('pre-commit hook rejected');
  });

  it('unstages all staged files with the current optimistic state', async () => {
    const stagedStatus = {
      ...repositoryStatus(),
      entries: [{ kind: 'ordinary' as const, path: 'app.ts', originalPath: null, indexStatus: 'modified' as const, worktreeStatus: 'unmodified' as const, submodule: null }],
    };
    const { fixture, invoke } = await createFixture(defaultIpc);
    statusStoreFor(fixture).state.set({ kind: 'ready', status: stagedStatus });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const unstageAll = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.index-actions button')]
      .find((button) => button.textContent?.includes('Unstage all')) as HTMLButtonElement;
    unstageAll.click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_apply_index_change', {
      repositoryId: 'skibidibi-git',
      operation: {
        action: 'unstage',
        selection: { scope: 'all' },
        expectedHead: 'abc',
        expectedHeadName: 'main',
        expectedDetached: false,
        expectedUnborn: false,
        expectedIndexFingerprint: 'index-before',
        expectedWorktreeFingerprint: 'worktree-before',
      },
    });
  });

  it('clears composer state and refreshes history and navigation after a successful commit', async () => {
    const stagedStatus = {
      ...repositoryStatus(),
      entries: [{ kind: 'ordinary' as const, path: 'app.ts', originalPath: null, indexStatus: 'modified' as const, worktreeStatus: 'unmodified' as const, submodule: null }],
    };
    const { fixture, invoke } = await createFixture(defaultIpc);
    statusStoreFor(fixture).state.set({ kind: 'ready', status: stagedStatus });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const textarea = fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement;
    textarea.value = 'Create the commit';
    textarea.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    (fixture.nativeElement.querySelector('.commit-composer .primary-action') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke.mock.calls.filter(([command]) => command === 'repository_create_commit')).toHaveLength(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_history')).toHaveLength(2);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation')).toHaveLength(2);
    expect(fixture.nativeElement.querySelector('.working-tree-history-row')).toBeNull();
  });

  it('uses the refreshed navigation OID when reloading classified history after a commit', async () => {
    let headOid = 'before-commit';
    const taskStatus = (entries: ReturnType<typeof repositoryStatus>['entries']) => ({
      ...repositoryStatus(),
      branch: { ...repositoryStatus().branch, oid: headOid, head: 'demo/task', upstream: 'origin/demo/task' },
      entries,
    });
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        return Promise.resolve(taskStatus([]));
      }
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/demo/task', name: 'demo/task', oid: headOid, current: true, upstream: 'origin/demo/task', ahead: 1, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'main-oid', current: false, upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [],
          stashes: [],
        });
      }
      if (command === 'repository_create_commit') {
        headOid = 'after-commit';
        return Promise.resolve({
          oid: headOid,
          status: taskStatus([]),
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    const stagedEntry = {
      kind: 'ordinary' as const,
      path: 'app.ts',
      originalPath: null,
      indexStatus: 'modified' as const,
      worktreeStatus: 'unmodified' as const,
      submodule: null,
    };
    statusStoreFor(fixture).state.set({ kind: 'ready', status: taskStatus([stagedEntry]) });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const textarea = fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement;
    textarea.value = 'Commit task';
    textarea.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.commit-composer .primary-action') as HTMLButtonElement).click();
    await fixture.whenStable();

    const historyRequests = invoke.mock.calls.filter(([command]) => command === 'repository_branch_history');
    expect(historyRequests.at(-1)?.[1]).toMatchObject({ expectedBranchOid: 'after-commit' });
  });

  it('opens amend mode from the workspace toolbar when the working tree is clean', async () => {
    const cleanStatus = { ...repositoryStatus(), entries: [] };
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_status'
        ? Promise.resolve(cleanStatus)
        : defaultIpc(command, request);
    const { fixture } = await createFixture(ipc);
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.working-tree-history-row')).toBeNull();
    const trigger = fixture.nativeElement.querySelector('.amend-entry') as HTMLButtonElement;
    trigger.click();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.inspector')?.textContent).toContain('Amend HEAD');
    expect(fixture.nativeElement.querySelector('.commit-composer')).toBeTruthy();
    await vi.waitFor(() => expect(globalThis.document.activeElement?.id).toBe('commit-message'));
    const cancel = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.commit-composer-actions button')]
      .find((button) => button.textContent?.includes('Cancel')) as HTMLButtonElement;
    cancel.click();
    fixture.detectChanges();
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(trigger));
  });

  it('amends HEAD with a new message and every optimistic status fingerprint', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.amend-entry') as HTMLButtonElement).click();
    fixture.detectChanges();
    const textarea = fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement;
    textarea.value = 'Replacement message';
    textarea.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    const amend = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.commit-composer-actions button')]
      .find((button) => button.textContent?.includes('new message')) as HTMLButtonElement;
    amend.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_amend_commit', {
      repositoryId: 'skibidibi-git',
      operation: {
        message: 'Replacement message',
        confirmUpstreamRewrite: false,
        expectedHead: 'abc',
        expectedHeadName: 'main',
        expectedDetached: false,
        expectedUnborn: false,
        expectedIndexFingerprint: 'index-before',
        expectedWorktreeFingerprint: 'worktree-before',
      },
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_history')).toHaveLength(2);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation')).toHaveLength(2);
    expect(fixture.nativeElement.querySelector('.commit-composer')).toBeNull();
  });

  it('confirms an upstream rewrite, supports no-edit amend, and blocks duplicate submission', async () => {
    let resolveAmend!: (value: unknown) => void;
    const pendingAmend = new Promise((resolve) => { resolveAmend = resolve; });
    const publishedStatus = {
      ...repositoryStatus(),
      branch: { ...repositoryStatus().branch, ahead: 0 },
    };
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        return Promise.resolve(publishedStatus);
      }
      if (command === 'repository_amend_commit') {
        return pendingAmend;
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.amend-entry') as HTMLButtonElement).click();
    fixture.detectChanges();
    const keepMessage = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.commit-composer-actions button')]
      .find((button) => button.textContent?.includes('keep message')) as HTMLButtonElement;
    keepMessage.click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-title')?.textContent).toContain('published');
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    fixture.detectChanges();
    keepMessage.click();

    expect(invoke.mock.calls.filter(([command]) => command === 'repository_amend_commit')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('repository_amend_commit', expect.objectContaining({
      operation: expect.objectContaining({ message: null, confirmUpstreamRewrite: true }),
    }));

    resolveAmend({
      previousOid: 'abc',
      oid: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
      state: 'succeeded',
      errorMessage: null,
      status: { ...publishedStatus, branch: { ...publishedStatus.branch, oid: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' }, entries: [] },
    });
    await pendingAmend;
    await fixture.whenStable();
  });

  it('keeps amend mode and its message after a failed amend and refreshes status', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_amend_commit'
        ? Promise.reject({ message: 'pre-commit hook rejected amend' })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.amend-entry') as HTMLButtonElement).click();
    fixture.detectChanges();
    const textarea = fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement;
    textarea.value = 'Keep amended message';
    textarea.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    const amend = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.commit-composer-actions button')]
      .find((button) => button.textContent?.includes('new message')) as HTMLButtonElement;
    amend.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect((fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement).value).toBe('Keep amended message');
    expect(fixture.nativeElement.querySelector('.commit-composer')?.textContent).toContain('Amend HEAD');
    expect(fixture.nativeElement.querySelector('[role="alert"]')?.textContent).toContain('pre-commit hook rejected amend');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_status').length).toBeGreaterThan(1);
  });

  it('preserves amend input and requires inspection when the backend outcome is unknown', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_amend_commit'
        ? Promise.resolve({
            previousOid: 'abc',
            oid: null,
            status: null,
            state: 'outcomeUnknown',
            errorMessage: 'Git exited before the new HEAD could be verified.',
          })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.amend-entry') as HTMLButtonElement).click();
    fixture.detectChanges();
    const textarea = fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement;
    textarea.value = 'Message that must survive';
    textarea.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    const amend = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.commit-composer-actions button')]
      .find((button) => button.textContent?.includes('new message')) as HTMLButtonElement;
    amend.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect((fixture.nativeElement.querySelector('#commit-message') as HTMLTextAreaElement).value).toBe('Message that must survive');
    expect(fixture.nativeElement.querySelector('.working-tree-action-error')?.textContent).toContain('outcome is unknown');
    expect(fixture.nativeElement.querySelector('.working-tree-action-error')?.textContent).toContain('before the new HEAD could be verified');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_amend_commit')).toHaveLength(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_status').length).toBeGreaterThan(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_history').length).toBeGreaterThan(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
  });

  it('keeps duplicate paths independently selectable by entry identity', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    statusStoreFor(fixture).state.set({
      kind: 'ready',
      status: {
        ...repositoryStatus(),
        entries: [
          { kind: 'ordinary', path: 'same.txt', originalPath: null, indexStatus: 'deleted', worktreeStatus: 'unmodified', submodule: null },
          { kind: 'untracked', path: 'same.txt', originalPath: null, indexStatus: 'untracked', worktreeStatus: 'untracked', submodule: null },
        ],
      },
    });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const checkboxes = fixture.nativeElement.querySelectorAll('.working-tree-file-checkbox') as NodeListOf<HTMLInputElement>;
    checkboxes[1].checked = true;
    checkboxes[1].dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const stageSelected = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.index-actions button')]
      .find((button) => button.textContent?.includes('Stage selected')) as HTMLButtonElement;
    expect(stageSelected.disabled).toBe(true);
    expect(stageSelected.title).toContain('same path');
    checkboxes[0].checked = true;
    checkboxes[0].dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(stageSelected.disabled).toBe(false);
    stageSelected.click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_apply_index_change', expect.objectContaining({
      operation: expect.objectContaining({
        action: 'stage',
        selection: {
          scope: 'selected',
          entries: [{ path: 'same.txt', oldPath: null, entryKind: 'untracked' }],
        },
      }),
    }));
  });

  it('invalidates an in-flight working-tree diff after staging changes', async () => {
    let resolveDiff!: (value: unknown) => void;
    const pendingDiff = new Promise((resolve) => { resolveDiff = resolve; });
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_working_tree_file_diff') {
        return pendingDiff;
      }
      if (command === 'repository_apply_index_change') {
        return Promise.resolve({
          changed: true,
          status: { ...repositoryStatus(), indexFingerprint: 'index-after-stage', entries: [] },
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    const statusStore = statusStoreFor(fixture);
    statusStore.state.set({ kind: 'ready', status: repositoryStatus() });
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.working-tree-files .changed-file') as HTMLButtonElement).click();

    const checkbox = fixture.nativeElement.querySelector(
      '[aria-label="Select new.ts"]',
    ) as HTMLInputElement;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const stageSelected = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.index-actions button')]
      .find((button) => button.textContent?.includes('Stage selected')) as HTMLButtonElement;
    stageSelected.click();
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(
        (statusStore.state() as { status: { indexFingerprint: string } }).status.indexFingerprint,
      ).toBe('index-after-stage');
    });

    expect(fixture.nativeElement.querySelector('.diff-view')).toBeNull();
    resolveDiff({
      path: 'new.ts',
      binary: false,
      truncated: false,
      patch: 'diff --git a/new.ts b/new.ts\n@@ -0,0 +1 @@\n+stale content',
      unstagedPatch: '',
    });
    await Promise.resolve();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.diff-view')).toBeNull();
  });

  it('separates local and remote branches and collapses slash-delimited folders', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const element = fixture.nativeElement as HTMLElement;
    const local = element.querySelector('[aria-label="Local branches"]') as HTMLElement;
    const remote = element.querySelector('[aria-label="Remote branches"]') as HTMLElement;

    expect(local.textContent).toContain('main');
    expect(local.textContent).toContain('rb');
    expect(local.textContent).not.toContain('feature');
    expect(remote.textContent).toContain('origin');
    expect(remote.textContent).not.toContain('main');
    expect(remote.querySelector('button.branch-row')).toBeNull();

    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(local.textContent).toContain('feature');
    (remote.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(remote.textContent).toContain('main');
  });

  it('marks an exact local ref as release from the keyboard-accessible context menu', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const currentActions = fixture.nativeElement.querySelector(
      '[aria-label="Actions for main"]',
    ) as HTMLButtonElement;

    currentActions.click();
    fixture.detectChanges();
    const markRelease = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>(
      '.branch-context-menu button',
    )].find((button) => button.textContent?.includes('Mark as release')) as HTMLButtonElement;
    expect(markRelease).toBeTruthy();
    markRelease.click();
    fixture.detectChanges();

    expect(globalThis.localStorage.getItem(releaseBranchStorageKey('skibidibi-git'))).toBe(
      'refs/heads/main',
    );
    expect(fixture.nativeElement.querySelector('[title="Release branch"]')).toBeTruthy();
    expect(fixture.nativeElement.textContent).toContain('is now the release branch');
  });

  it('renders a lazy dirty marker only for the branch attached to a dirty worktree', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(local.querySelector('[title="3 uncommitted changes in its worktree"]')).toBeTruthy();
    });
    expect(local.querySelectorAll('.dirty-token')).toHaveLength(2);
    expect(
      (local.querySelector('.pinned-current') as HTMLElement).querySelector('.dirty-token'),
    ).toBeTruthy();
  });

  it('uses an in-app source-to-target confirmation and forwards auto-stash to merge', async () => {
    const confirm = vi.spyOn(globalThis, 'confirm');
    const { fixture, invoke } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    const mergeIntoActive = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>(
      '.branch-context-menu button',
    )].find((button) => button.textContent?.includes('Merge into active branch')) as HTMLButtonElement;
    mergeIntoActive.click();

    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.merge-confirmation')).toBeTruthy();
    });
    const route = fixture.nativeElement.querySelector('.merge-route') as HTMLElement;
    expect(route.textContent).toContain('rb/feature');
    expect(route.textContent).toContain('main');
    expect((fixture.nativeElement.querySelector('.merge-autostash input') as HTMLInputElement).checked).toBe(true);

    (fixture.nativeElement.querySelector('#confirm-branch-merge') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('repository_merge_branch', {
        repositoryId: 'skibidibi-git',
        operation: expect.objectContaining({
          sourceFullName: 'refs/heads/rb/feature',
          targetFullName: 'refs/heads/main',
          autoStash: expect.objectContaining({ message: expect.stringContaining('WIP') }),
        }),
      });
    });
    expect(confirm).not.toHaveBeenCalled();
  });

  it('fast-forwards an inactive branch in the background without switching worktrees', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'local', fullName: 'refs/heads/rb/feature', name: 'rb/feature', oid: 'def', current: false, upstream: 'origin/rb/feature', ahead: 0, behind: 2, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [],
          stashes: [],
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();

    const pull = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>(
      '.branch-context-menu button',
    )].find((button) => button.textContent?.includes('Pull branch (FF-only)')) as HTMLButtonElement;
    pull.click();

    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('repository_pull_inactive_branch', {
        repositoryId: 'skibidibi-git',
        operation: {
          branchFullName: 'refs/heads/rb/feature',
          expectedOid: 'def',
          expectedUpstream: 'origin/rb/feature',
        },
      });
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch')).toHaveLength(0);
  });

  it('confirms main into a noncurrent target once, then switches and merges fresh refs', async () => {
    let switched = false;
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'switch_repository_branch') {
        switched = true;
        return defaultIpc(command, request);
      }
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: switched
            ? [
                { kind: 'local', fullName: 'refs/heads/rb/feature', name: 'rb/feature', oid: 'fresh-feature', current: true, upstream: 'origin/rb/feature', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
                { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'fresh-main', current: false, upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
              ]
            : [
                { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
                { kind: 'local', fullName: 'refs/heads/rb/feature', name: 'rb/feature', oid: 'def', current: false, upstream: 'origin/rb/feature', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
              ],
          worktrees: [],
          stashes: [],
        });
      }
      return defaultIpc(command, request);
    };
    const confirm = vi.spyOn(globalThis, 'confirm');
    const { fixture, invoke } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    const mergeMain = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>(
      '.branch-context-menu button',
    )].find((button) => button.textContent?.includes('Merge main into this branch')) as HTMLButtonElement;
    mergeMain.click();
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.merge-confirmation')).toBeTruthy();
    });
    (fixture.nativeElement.querySelector('#confirm-branch-merge') as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('repository_merge_branch', {
        repositoryId: 'skibidibi-git',
        operation: expect.objectContaining({
          sourceFullName: 'refs/heads/main',
          expectedSourceOid: 'fresh-main',
          targetFullName: 'refs/heads/rb/feature',
          expectedTargetOid: 'fresh-feature',
        }),
      });
    });
    const commands = invoke.mock.calls.map(([command]) => command);
    expect(commands.indexOf('switch_repository_branch')).toBeLessThan(commands.indexOf('repository_merge_branch'));
    expect(confirm).not.toHaveBeenCalled();
  });

  it('keeps local and remote folders with the same path independently collapsible', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command !== 'repository_navigation') {
        return defaultIpc(command, request);
      }
      return Promise.resolve({
        branches: [
          { kind: 'local', fullName: 'refs/heads/rb/local-feature', name: 'rb/local-feature', oid: 'abc', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          { kind: 'remote', fullName: 'refs/remotes/rb/remote-feature', name: 'rb/remote-feature', oid: 'def', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
        ],
        worktrees: [],
        stashes: [],
      });
    };
    const { fixture } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    const remote = fixture.nativeElement.querySelector('[aria-label="Remote branches"]') as HTMLElement;

    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    expect(local.textContent).toContain('local-feature');
    expect(remote.textContent).not.toContain('remote-feature');
    expect(remote.querySelector('.folder-row')?.getAttribute('aria-expanded')).toBe('false');
  });

  it('filters branch trees while retaining matching folders', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const input = fixture.nativeElement.querySelector('.filter input') as HTMLInputElement;
    input.value = 'feature';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    const remote = fixture.nativeElement.querySelector('[aria-label="Remote branches"]') as HTMLElement;
    expect(local.textContent).toContain('rb');
    expect(local.textContent).toContain('feature');
    expect(local.textContent).toContain('main');
    expect(remote.querySelectorAll('.tree-row')).toHaveLength(0);
  });

  it('persists an expanded folder across navigation refreshes', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    expect(local.textContent).toContain('feature');
    expect(
      globalThis.localStorage.getItem(branchExpansionStorageKey('skibidibi-git', 'local')),
    ).toBe('["rb"]');

    const refresh = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.workspace-bar button')]
      .find((button) => button.textContent?.trim() === 'Refresh');
    expect(refresh).toBeDefined();
    refresh?.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(
      (fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement)
        .textContent,
    ).toContain('feature');
  });

  it('opens Compare with the active branch and a distinct main ref as its default target', async () => {
    const { fixture } = await createFixture(defaultIpc);
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect((fixture.nativeElement.querySelector('.compare-entry') as HTMLButtonElement).disabled).toBe(false);
    });
    const router = fixture.debugElement.injector.get(Router);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    (fixture.nativeElement.querySelector('.compare-entry') as HTMLButtonElement).click();

    expect(navigate).toHaveBeenCalledWith(
      ['/workspace', 'skibidibi-git', 'compare'],
      {
        queryParams: {
          source: 'refs/heads/main',
          target: 'refs/remotes/origin/main',
        },
      },
    );
  });

  it('prefers the remembered release branch as the Compare target', async () => {
    globalThis.localStorage.setItem(
      releaseBranchStorageKey('skibidibi-git'),
      'refs/heads/rb/feature',
    );
    const { fixture } = await createFixture(defaultIpc);
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect((fixture.nativeElement.querySelector('.compare-entry') as HTMLButtonElement).disabled).toBe(false);
    });
    const router = fixture.debugElement.injector.get(Router);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    (fixture.nativeElement.querySelector('.compare-entry') as HTMLButtonElement).click();

    expect(navigate).toHaveBeenCalledWith(
      ['/workspace', 'skibidibi-git', 'compare'],
      {
        queryParams: {
          source: 'refs/heads/main',
          target: 'refs/heads/rb/feature',
        },
      },
    );
  });

  it('previews a local branch without switching its active worktree and compares it with main by default', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    ([...local.querySelectorAll<HTMLButtonElement>('button.branch-row')]
      .find((button) => button.title.includes('Preview rb/feature')) as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_branch_history', {
      repositoryId: 'skibidibi-git',
      branchFullName: 'refs/heads/rb/feature',
      expectedBranchOid: 'def',
      targetFullName: 'refs/heads/main',
      expectedTargetOid: 'abc',
      cursor: null,
      limit: 50,
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch')).toHaveLength(0);
    expect(fixture.nativeElement.querySelector('.branch-preview-banner')?.textContent).toContain('rb/feature');
    expect(fixture.nativeElement.querySelector('.working-tree-history-row')).toBeNull();

    (fixture.nativeElement.querySelector('.branch-preview-banner button') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.branch-preview-banner')).toBeNull();
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_history')).toHaveLength(2);
  });

  it('clears a read-only preview after a successful real checkout', async () => {
    const { fixture } = await createFixture(defaultIpcWithoutCheckedOutFeature);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...local.querySelectorAll<HTMLButtonElement>('button.branch-row')]
      .find((button) => button.title.includes('Preview rb/feature')) as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.branch-preview-banner')).not.toBeNull();

    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.branch-context-menu button')]
      .find((button) => button.textContent?.includes('Checkout in active worktree')) as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#confirm-branch-switch') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.branch-preview-banner')).toBeNull();
  });

  it('renders task, merge, and shared-base classifications without text labels', async () => {
    const previewCommits = [
      { ...commits[0], relation: 'task' as const },
      { ...commits[1], relation: 'merge' as const },
      { ...commits[1], oid: 'cccccccccccccccccccccccccccccccccccccccc', relation: 'base' as const },
    ];
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_branch_history'
        ? Promise.resolve({ commits: previewCommits, nextCursor: null })
        : defaultIpc(command, request);
    const { fixture } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...local.querySelectorAll<HTMLButtonElement>('button.branch-row')]
      .find((button) => button.title.includes('Preview rb/feature')) as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.commit-task .graph')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.commit-merge .graph')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.commit-base .graph')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.commit-relation')).toBeNull();
  });

  it('classifies the active branch against the marked release branch', async () => {
    globalThis.localStorage.setItem(
      releaseBranchStorageKey('skibidibi-git'),
      'refs/heads/rb/feature',
    );
    const classifiedCommits = [
      { ...commits[0], relation: 'task' as const },
      { ...commits[1], relation: 'merge' as const },
    ];
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_branch_history'
        ? Promise.resolve({ commits: classifiedCommits, nextCursor: null })
        : defaultIpc(command, request);

    const { fixture, invoke } = await createFixture(ipc);

    expect(invoke).toHaveBeenCalledWith('repository_branch_history', {
      repositoryId: 'skibidibi-git',
      branchFullName: 'refs/heads/main',
      expectedBranchOid: 'abc',
      targetFullName: 'refs/heads/rb/feature',
      expectedTargetOid: 'def',
      cursor: null,
      limit: 50,
    });
    expect(fixture.nativeElement.querySelector('.commit-task')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.commit-merge')).not.toBeNull();
  });

  it('classifies an active task branch against main when no release is marked', async () => {
    const classifiedCommits = [
      { ...commits[0], relation: 'task' as const },
      { ...commits[1], relation: 'base' as const },
    ];
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/demo/task', name: 'demo/task', oid: 'def', current: true, upstream: null, ahead: 2, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: false, upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [],
          stashes: [],
        });
      }
      return command === 'repository_branch_history'
        ? Promise.resolve({ commits: classifiedCommits, nextCursor: null })
        : defaultIpc(command, request);
    };

    const { fixture, invoke } = await createFixture(ipc);

    expect(invoke).toHaveBeenCalledWith('repository_branch_history', {
      repositoryId: 'skibidibi-git',
      branchFullName: 'refs/heads/demo/task',
      expectedBranchOid: 'def',
      targetFullName: 'refs/heads/main',
      expectedTargetOid: 'abc',
      cursor: null,
      limit: 50,
    });
    expect(fixture.nativeElement.querySelector('.commit-task')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.commit-base')).not.toBeNull();
  });

  it('opens a local branch in VS Code through the explicit context action', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'open_branch_workspace'
        ? Promise.resolve({ path: '/work/feature-tree', worktreeCreated: true, target: 'vsCode' })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.branch-context-menu button')]
      .find((button) => button.textContent?.includes('Open in VS Code')) as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('open_branch_workspace', {
      request: {
        repositoryId: 'skibidibi-git',
        branchFullName: 'refs/heads/rb/feature',
        expectedOid: 'def',
        target: 'vsCode',
      },
    });
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('Created a managed worktree');
  });

  it('confirms and checks out a non-current local branch from its explicit context action, then refreshes workspace data', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_status'
        ? Promise.resolve({ ...repositoryStatus(), entries: [] })
        : defaultIpcWithoutCheckedOutFeature(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.branch-context-menu button')]
      .find((button) => button.textContent?.includes('Checkout in active worktree')) as HTMLButtonElement).click();
    fixture.detectChanges();

    const dialog = fixture.nativeElement.querySelector('.switch-confirmation') as HTMLDialogElement;
    expect(dialog.textContent).toContain('main');
    expect(dialog.textContent).toContain('rb/feature');
    expect(invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch')).toHaveLength(0);
    (dialog.querySelector('#confirm-branch-switch') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('switch_repository_branch', {
      repositoryId: 'skibidibi-git',
      operation: {
        fullName: 'refs/heads/rb/feature',
        expectedOid: 'def',
        stashOnDirty: false,
        stashMessage: null,
      },
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
  });

  it('opens the existing worktree instead of switching a branch that is checked out there', async () => {
    const worktreeRepository = {
      ...rememberedRepository,
      id: 'feature-tree',
      repositoryGroupId: 'shared-git-directory',
      worktreeRole: 'linked' as const,
      canonicalPath: '/work/feature-tree',
      displayName: 'feature-tree',
    };
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'remember_repository' ? Promise.resolve(worktreeRepository) : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const router = TestBed.inject(Router);
    const navigateByUrl = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.branch-context-menu button')]
      .find((button) => button.textContent?.includes('Checkout in active worktree')) as HTMLButtonElement).click();
    fixture.detectChanges();

    const dialog = fixture.nativeElement.querySelector('.switch-confirmation') as HTMLDialogElement;
    expect(dialog.textContent).toContain('already checked out in worktree “feature-tree”');
    expect(dialog.textContent).toContain('take you to that worktree');
    expect(dialog.textContent).toContain('uncommitted changes will stay unchanged');
    expect(dialog.textContent).toContain('/work/feature-tree');
    expect(dialog.querySelector('input[type="checkbox"]')).toBeNull();
    expect((dialog.querySelector('#confirm-branch-switch') as HTMLButtonElement).textContent).toContain('Open worktree');

    (dialog.querySelector('#confirm-branch-switch') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch')).toHaveLength(0);
    expect(invoke).toHaveBeenCalledWith('remember_repository', { repositoryPath: '/work/feature-tree' });
    expect(navigateByUrl).toHaveBeenCalledWith('/repositories', { skipLocationChange: true });
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'feature-tree', 'history']);
  });

  it('creates a local branch from current HEAD without checking it out', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const newBranch = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.workspace-bar button')]
      .find((button) => button.textContent?.includes('New branch')) as HTMLButtonElement;
    newBranch.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('#new-branch-name') as HTMLInputElement;
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(input));
    input.value = 'feature/current-head';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.navigation-inline-form button[type="submit"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('create_repository_branch', {
      repositoryId: 'skibidibi-git',
      operation: {
        name: 'feature/current-head',
        source: { kind: 'current', expectedOid: 'abc' },
      },
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch')).toHaveLength(0);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('Created local branch');
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(newBranch));
  });

  it('creates a branch from the exact selected commit and returns focus to its inspector action', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const trigger = fixture.nativeElement.querySelector('.create-branch-from-commit') as HTMLButtonElement;
    trigger.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('#new-branch-name') as HTMLInputElement;
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(input));
    expect(fixture.nativeElement.querySelector('.navigation-inline-form')?.textContent).toContain(
      commits[0].oid.slice(0, 7),
    );
    input.value = 'feature/from-selected';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.navigation-inline-form button[type="submit"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('create_repository_branch', {
      repositoryId: 'skibidibi-git',
      operation: {
        name: 'feature/from-selected',
        source: { kind: 'commit', oid: commits[0].oid },
      },
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch')).toHaveLength(0);
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(trigger));
  });

  it('returns focus after cancelling branch creation', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const newBranch = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.workspace-bar button')]
      .find((button) => button.textContent?.includes('New branch')) as HTMLButtonElement;
    newBranch.click();
    fixture.detectChanges();
    await vi.waitFor(() => expect(globalThis.document.activeElement?.id).toBe('new-branch-name'));
    (fixture.nativeElement.querySelector('.navigation-inline-form button[type="button"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(newBranch));
  });

  it('creates the same-named local tracking branch directly from the exact remote ref', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const remote = fixture.nativeElement.querySelector('[aria-label="Remote branches"]') as HTMLElement;
    (remote.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('[aria-label="Create local branch from origin/main"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('create_repository_branch', {
      repositoryId: 'skibidibi-git',
      operation: {
        name: null,
        source: {
          kind: 'remoteTracking',
          fullName: 'refs/remotes/origin/main',
          expectedOid: 'abc',
        },
      },
    });
    expect(fixture.nativeElement.querySelector('.navigation-inline-form')).toBeNull();
    expect(fixture.nativeElement.textContent).toContain('Created local branch “main” tracking origin/main.');
    await vi.waitFor(() => expect(globalThis.document.activeElement?.id).toBe('navigation-action-notice'));
  });

  it('preserves nested branch path segments when deriving the local name from a remote ref', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'remote', fullName: 'refs/remotes/origin/rb/feature', name: 'origin/rb/feature', oid: 'def', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [],
          stashes: [],
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    const remote = fixture.nativeElement.querySelector('[aria-label="Remote branches"]') as HTMLElement;
    (remote.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (remote.querySelectorAll('.folder-row')[1] as HTMLButtonElement).click();
    fixture.detectChanges();
    (remote.querySelector('[aria-label="Create local branch from origin/rb/feature"]') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('create_repository_branch', {
      repositoryId: 'skibidibi-git',
      operation: {
        name: null,
        source: {
          kind: 'remoteTracking',
          fullName: 'refs/remotes/origin/rb/feature',
          expectedOid: 'def',
        },
      },
    });
  });

  it('refreshes after branch creation failure and explains manual cleanup recovery', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'create_repository_branch'
        ? Promise.reject({ message: 'manualCleanupRequired: ref write succeeded before verification failed' })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const newBranch = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.workspace-bar button')]
      .find((button) => button.textContent?.includes('New branch')) as HTMLButtonElement;
    newBranch.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('#new-branch-name') as HTMLInputElement;
    input.value = 'possibly-created';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.navigation-inline-form button[type="submit"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const alert = fixture.nativeElement.querySelector('.navigation-action-error') as HTMLElement;
    expect(alert.textContent).toContain('branch may have been created');
    expect(alert.textContent).toContain('delete it manually');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_status').length).toBeGreaterThan(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
  });

  it('offers to stash a dirty working tree and retries the branch switch with a WIP message', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command !== 'switch_repository_branch') {
        return defaultIpcWithoutCheckedOutFeature(command, request);
      }
      const operation = (request as { operation: { stashOnDirty: boolean } }).operation;
      return operation.stashOnDirty
        ? Promise.resolve({
            fullName: 'refs/heads/rb/feature',
            name: 'rb/feature',
            head: 'def',
            changed: true,
            stashCreated: true,
            operationSucceeded: true,
            operationError: null,
            autoStash: {
              create: 'created',
              stash: { oid: 'stash-oid', selector: 'stash@{0}' },
              restore: 'applied',
              cleanup: 'dropped',
              createError: null,
              restoreError: null,
              cleanupError: null,
            },
          })
        : Promise.reject({ message: 'dirtyWorkingTree: uncommitted changes prevent checkout' });
    };
    const { fixture, invoke } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.branch-context-menu button')]
      .find((button) => button.textContent?.includes('Checkout in active worktree')) as HTMLButtonElement).click();
    fixture.detectChanges();
    const dialog = fixture.nativeElement.querySelector('.switch-confirmation') as HTMLDialogElement;
    expect((dialog.querySelector('input[type="checkbox"]') as HTMLInputElement).checked).toBe(true);
    (dialog.querySelector('#confirm-branch-switch') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const switchCalls = invoke.mock.calls.filter(([command]) => command === 'switch_repository_branch');
    expect(switchCalls).toHaveLength(1);
    expect(switchCalls[0][1]).toMatchObject({
      operation: {
        fullName: 'refs/heads/rb/feature',
        expectedOid: 'def',
        stashOnDirty: true,
      },
    });
    expect((switchCalls[0][1] as { operation: { stashMessage: string } }).operation.stashMessage)
      .toMatch(/^WIP \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2} main$/);
    expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeNull();
  });

  it('surfaces a resolved partial auto-stash switch result and refreshes all repository slices', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'switch_repository_branch'
        ? Promise.resolve({
            fullName: 'refs/heads/rb/feature',
            name: 'rb/feature',
            head: 'abc',
            changed: false,
            stashCreated: true,
            operationSucceeded: false,
            operationError: 'checkout failed',
            autoStash: {
              create: 'created',
              stash: { oid: 'stash-oid', selector: 'stash@{0}' },
              restore: 'conflicted',
              cleanup: 'retained',
              createError: null,
              restoreError: 'resolve conflicts',
              cleanupError: null,
            },
          })
        : defaultIpcWithoutCheckedOutFeature(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Actions for rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    ([...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.branch-context-menu button')]
      .find((button) => button.textContent?.includes('Checkout in active worktree')) as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#confirm-branch-switch') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const alert = fixture.nativeElement.querySelector('.navigation-action-error') as HTMLElement;
    expect(alert.closest('.workspace-toasts')).not.toBeNull();
    expect(alert.closest('.navigation')).toBeNull();
    expect(alert.textContent).toContain('checkout failed');
    expect(alert.textContent).toContain('Operation: switch failed');
    expect(alert.textContent).toContain('Restore: changes were applied with conflicts');
    expect(alert.textContent).toContain('Cleanup: auto-stash stash@{0} retained');
    expect(alert.textContent).toContain('conflicts');
    expect(alert.textContent).toContain('resolve conflicts');
    expect(alert.textContent).toContain('retained');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_status').length).toBeGreaterThan(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_history').length).toBeGreaterThan(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
    (alert.querySelector('[aria-label="Dismiss workspace error"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.navigation-action-error')).toBeNull();
  });

  it('creates a manual stash including untracked files with a full state precondition', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_navigation'
        ? Promise.resolve(navigationWithStash())
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    [...fixture.nativeElement.querySelectorAll('.workspace-bar button')]
      .find((button: HTMLButtonElement) => button.textContent?.trim() === 'Stash')?.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('[aria-label="Stash message"]') as HTMLInputElement;
    expect(input.value).toMatch(/^WIP \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2} main$/);
    input.value = 'Pause current work';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.stash-creation-dialog button[type="submit"]') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('repository_push_stash', {
      repositoryId: 'skibidibi-git',
      operation: {
        message: 'Pause current work',
        includeUntracked: true,
        precondition: {
          expectedHead: 'abc',
          expectedHeadName: 'main',
          expectedDetached: false,
          expectedUnborn: false,
          expectedIndexFingerprint: 'index-before',
          expectedWorktreeFingerprint: 'worktree-before',
        },
      },
    });
  });

  it('preserves the stash message and requires inspection for an unknown mutation outcome', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve(navigationWithStash());
      }
      if (command === 'repository_push_stash') {
        return Promise.resolve({
          state: 'failed',
          stash: null,
          status: null,
          errorMessage: 'Git exited before stash creation could be verified.',
          mutationOid: 'mutation-123',
          mutationMayHaveOccurred: true,
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    [...fixture.nativeElement.querySelectorAll('.workspace-bar button')]
      .find((button: HTMLButtonElement) => button.textContent?.trim() === 'Stash')?.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('[aria-label="Stash message"]') as HTMLInputElement;
    input.value = 'Do not retry blindly';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.stash-creation-dialog button[type="submit"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(input.value).toBe('Do not retry blindly');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('before stash creation could be verified');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('mutating command was attempted');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('before retrying');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_status').length).toBeGreaterThan(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
  });

  it('accepts a verified stash creation even when a mutating command was attempted', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve(navigationWithStash());
      }
      if (command === 'repository_push_stash') {
        return Promise.resolve({
          state: 'created',
          stash: { oid: 'stash-new', selector: 'stash@{0}' },
          status: { ...repositoryStatus(), entries: [] },
          errorMessage: null,
          mutationOid: 'stash-new',
          mutationMayHaveOccurred: true,
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    [...fixture.nativeElement.querySelectorAll('.workspace-bar button')]
      .find((button: HTMLButtonElement) => button.textContent?.trim() === 'Stash')?.click();
    fixture.detectChanges();
    const input = fixture.nativeElement.querySelector('[aria-label="Stash message"]') as HTMLInputElement;
    input.value = 'Verified stash';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.stash-creation-dialog button[type="submit"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.navigation-action-error')).toBeNull();
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('Created stash@{0}');
  });

  it('shows progress on the exact stash action while it is pending', async () => {
    let resolveApply!: (value: unknown) => void;
    const pendingApply = new Promise((resolve) => { resolveApply = resolve; });
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve(navigationWithStash());
      }
      if (command === 'repository_apply_stash') {
        return pendingApply;
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('[aria-label="Apply stash@{0}"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('[aria-label="Apply stash@{0}"]')?.textContent).toContain('Applying…');

    resolveApply({
      stash: { oid: 'stash-oid', selector: 'stash@{0}' },
      restore: 'applied',
      cleanup: 'notRequired',
      status: repositoryStatus(),
      errorMessage: null,
      mutationMayHaveOccurred: false,
    });
    await pendingApply;
    await fixture.whenStable();
  });

  it('accepts verified apply, pop, and drop results when mutation was attempted', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve(navigationWithStash());
      }
      if (command === 'repository_apply_stash') {
        return Promise.resolve({
          stash: { oid: 'stash-oid', selector: 'stash@{0}' },
          restore: 'applied',
          cleanup: 'retained',
          status: repositoryStatus(),
          errorMessage: null,
          mutationMayHaveOccurred: true,
        });
      }
      if (command === 'repository_pop_stash') {
        return Promise.resolve({
          stash: { oid: 'stash-oid', selector: 'stash@{0}' },
          restore: 'applied',
          cleanup: 'dropped',
          status: repositoryStatus(),
          restoreError: null,
          cleanupError: null,
          mutationMayHaveOccurred: true,
        });
      }
      if (command === 'repository_drop_stash') {
        return Promise.resolve({
          stash: { oid: 'stash-oid', selector: 'stash@{0}' },
          cleanup: 'dropped',
          errorMessage: null,
          mutationMayHaveOccurred: true,
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    expect(fixture.nativeElement.querySelector('.stash-action-menu [role="menu"]')).not.toBeNull();

    (fixture.nativeElement.querySelector('[aria-label="Apply stash@{0}"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.navigation-action-error')).toBeNull();
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('changes applied');
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('stash retained');

    (fixture.nativeElement.querySelector('[aria-label="Pop stash@{0}"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.navigation-action-error')).toBeNull();
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('stash dropped');

    (fixture.nativeElement.querySelector('[aria-label="Drop stash@{0}"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.navigation-action-error')).toBeNull();
    expect(fixture.nativeElement.querySelector('.navigation-action-notice')?.textContent).toContain('Dropped stash@{0}');
  });

  it('applies an exact stash identity with index restore and reports conflicts without hiding retention', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve(navigationWithStash());
      }
      if (command === 'repository_apply_stash') {
        return Promise.resolve({
          stash: { oid: 'stash-oid', selector: 'stash@{0}' },
          restore: 'conflicted',
          cleanup: 'retained',
          status: repositoryStatus(),
          errorMessage: 'resolve app.ts',
          mutationMayHaveOccurred: false,
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('[aria-label="Apply stash@{0}"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_apply_stash', {
      repositoryId: 'skibidibi-git',
      operation: {
        stash: { oid: 'stash-oid', selector: 'stash@{0}' },
        restoreIndex: true,
        precondition: expect.objectContaining({
          expectedHead: 'abc',
          expectedIndexFingerprint: 'index-before',
          expectedWorktreeFingerprint: 'worktree-before',
        }),
      },
    });
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('conflicts');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('retained');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('resolve app.ts');
  });

  it('reports pop cleanup failure and confirms dropping the exact stash identity', async () => {
    let pop = true;
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve(navigationWithStash());
      }
      if (command === 'repository_pop_stash') {
        pop = false;
        return Promise.resolve({
          stash: { oid: 'stash-oid', selector: 'stash@{0}' },
          restore: 'applied',
          cleanup: 'failed',
          status: repositoryStatus(),
          restoreError: null,
          cleanupError: 'stash ref changed concurrently',
          mutationMayHaveOccurred: false,
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('[aria-label="Pop stash@{0}"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(pop).toBe(false);
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('Restore: changes applied');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('Cleanup: failed');
    expect(fixture.nativeElement.querySelector('.navigation-action-error')?.textContent).toContain('stash ref changed concurrently');

    (fixture.nativeElement.querySelector('[aria-label="Drop stash@{0}"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-description')?.textContent).toContain('Drop stash@{0}');
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await fixture.whenStable();
    expect(invoke).toHaveBeenCalledWith('repository_drop_stash', {
      repositoryId: 'skibidibi-git',
      operation: { stash: { oid: 'stash-oid', selector: 'stash@{0}' } },
    });
  });

  it('persists current-only mode and hides unrelated references while keeping current items expanded', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command !== 'repository_navigation') {
        return defaultIpc(command, request);
      }
      return Promise.resolve({
        branches: [
          { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 1, behind: 0, upstreamGone: false, symbolicTarget: null },
          { kind: 'local', fullName: 'refs/heads/feature', name: 'feature', oid: 'def', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          { kind: 'remote', fullName: 'refs/remotes/origin/main', name: 'origin/main', oid: 'abc', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
        ],
        worktrees: [
          { path: '/work/skibidibi-git', head: 'abc', branch: 'refs/heads/main', detached: false, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null },
          { path: '/work/feature-tree', head: 'def', branch: 'refs/heads/feature', detached: false, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null },
        ],
        stashes: [{ oid: 'stash', selector: 'stash@{0}', message: 'WIP', author: 'Ada', authoredAt: '2026-07-15T12:00:00Z' }],
      });
    };
    const { fixture } = await createFixture(ipc);
    const element = fixture.nativeElement as HTMLElement;
    const currentOnly = [...element.querySelectorAll<HTMLLabelElement>('.toggle-control')]
      .find((label) => label.textContent?.includes('Current only'))
      ?.querySelector('input') as HTMLInputElement;

    currentOnly.click();
    fixture.detectChanges();

    const navigation = fixture.nativeElement.querySelector('.navigation') as HTMLElement;
    expect(navigation.textContent).toContain('main');
    expect(navigation.textContent).not.toContain('feature');
    expect(navigation.textContent).not.toContain('Remote branches');
    expect(navigation.textContent).not.toContain('Stashes');
    expect(navigation.textContent).toContain('Pull Requests');
    expect(navigation.querySelectorAll('.worktree-row')).toHaveLength(1);
    expect(globalThis.localStorage.getItem('skibidibi-git.workspace.current-only.skibidibi-git')).toBe('true');
  });

  it('opens an in-app dialog and deletes a non-current local branch only after explicit confirmation', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    (local.querySelector('[aria-label="Delete local branch rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();

    const dialog = fixture.nativeElement.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog.textContent).toContain('Delete local branch?');
    expect(dialog.textContent).toContain('rb/feature');
    expect(invoke.mock.calls.some(([command]) => command === 'delete_repository_branch')).toBe(false);
    await vi.waitFor(() => expect(globalThis.document.activeElement?.id).toBe('cancel-reference-deletion'));

    const confirm = dialog.querySelector('#confirm-reference-deletion') as HTMLButtonElement;
    confirm.click();
    confirm.click();
    await fixture.whenStable();

    expect(invoke.mock.calls.filter(([command]) => command === 'delete_repository_branch')).toEqual([[
      'delete_repository_branch',
      {
      repositoryId: 'skibidibi-git',
      fullName: 'refs/heads/rb/feature',
      expectedOid: 'def',
      },
    ]]);
  });

  it('lists the associated branch before removing a worktree after explicit confirmation', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);

    (fixture.nativeElement.querySelector('[aria-label="Remove worktree /work/feature-tree"]') as HTMLButtonElement).click();
    fixture.detectChanges();

    const dialog = fixture.nativeElement.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog.textContent).toContain('/work/feature-tree');
    expect(dialog.textContent).toContain('Local branch to delete: rb/feature');
    expect(dialog.textContent).toContain('Unmerged commits may be lost');
    expect(invoke.mock.calls.some(([command]) => command === 'remove_repository_worktree')).toBe(false);

    (dialog.querySelector('#confirm-reference-deletion') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('remove_repository_worktree', {
      repositoryId: 'skibidibi-git',
      path: '/work/feature-tree',
      expectedHead: 'def',
      branchFullName: 'refs/heads/rb/feature',
      mode: 'safe',
      stashMessage: null,
    });
  });

  it('requires acknowledgement before force removing a worktree without a stash', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('[aria-label="Remove worktree /work/feature-tree"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    const dialog = fixture.nativeElement.querySelector('[role="alertdialog"]') as HTMLElement;

    (dialog.querySelector('input[value="force"]') as HTMLInputElement).click();
    fixture.detectChanges();
    const confirm = dialog.querySelector('#confirm-reference-deletion') as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
    expect(dialog.textContent).toContain('modified, untracked and ignored files can be permanently deleted');
    (dialog.querySelector('.force-removal-acknowledgement input') as HTMLInputElement).click();
    fixture.detectChanges();
    expect(confirm.disabled).toBe(false);

    confirm.click();
    await fixture.whenStable();
    expect(invoke).toHaveBeenCalledWith('remove_repository_worktree', {
      repositoryId: 'skibidibi-git',
      path: '/work/feature-tree',
      expectedHead: 'def',
      branchFullName: 'refs/heads/rb/feature',
      mode: 'force',
      stashMessage: null,
    });
  });

  it('sends the existing WIP name when stashing changes before force removal', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('[aria-label="Remove worktree /work/feature-tree"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    const dialog = fixture.nativeElement.querySelector('[role="alertdialog"]') as HTMLElement;

    (dialog.querySelector('input[value="stashAndForce"]') as HTMLInputElement).click();
    fixture.detectChanges();
    const message = (dialog.querySelector('.worktree-removal-warning code') as HTMLElement).textContent?.trim() ?? '';
    expect(message).toMatch(/^WIP \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2} rb\/feature$/);
    expect(dialog.textContent).toContain('Ignored files');

    (dialog.querySelector('#confirm-reference-deletion') as HTMLButtonElement).click();
    await fixture.whenStable();
    expect(invoke).toHaveBeenCalledWith('remove_repository_worktree', {
      repositoryId: 'skibidibi-git',
      path: '/work/feature-tree',
      expectedHead: 'def',
      branchFullName: 'refs/heads/rb/feature',
      mode: 'stashAndForce',
      stashMessage: message,
    });
  });

  it('keeps the dialog open after safe removal is rejected and never retries with force', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'remove_repository_worktree'
        ? Promise.resolve({
            path: '/work/feature-tree',
            branchFullName: 'refs/heads/rb/feature',
            worktreeRemoved: false,
            worktreeRemovalError: 'contains modified or untracked files',
            branchDeleted: false,
            branchDeletionError: null,
            mode: 'safe',
            stash: null,
          })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('[aria-label="Remove worktree /work/feature-tree"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#confirm-reference-deletion') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('#worktree-removal-dialog-error').textContent).toContain(
      'contains modified or untracked files',
    );
    const removalCalls = invoke.mock.calls.filter(([command]) => command === 'remove_repository_worktree');
    expect(removalCalls).toHaveLength(1);
    expect(removalCalls[0][1]).toMatchObject({ mode: 'safe', stashMessage: null });
  });

  it('states that no branch will be deleted for a detached worktree', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [{ path: '/work/detached', head: 'def', branch: null, detached: true, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null }],
          stashes: [],
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);

    (fixture.nativeElement.querySelector('[aria-label="Remove worktree /work/detached"]') as HTMLButtonElement).click();
    fixture.detectChanges();

    const dialog = fixture.nativeElement.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog.textContent).toContain('No local branch will be deleted because this worktree is detached.');
    expect(invoke.mock.calls.some(([command]) => command === 'remove_repository_worktree')).toBe(false);
  });

  it('does not invoke destructive branch or worktree commands when the in-app dialog is cancelled', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();

    const branchTrigger = local.querySelector('[aria-label="Delete local branch rb/feature"]') as HTMLButtonElement;
    branchTrigger.click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#cancel-reference-deletion') as HTMLButtonElement).click();
    fixture.detectChanges();
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(branchTrigger));

    const worktreeTrigger = fixture.nativeElement.querySelector('[aria-label="Remove worktree /work/feature-tree"]') as HTMLButtonElement;
    worktreeTrigger.click();
    fixture.detectChanges();
    const dialog = fixture.nativeElement.querySelector('[role="alertdialog"]') as HTMLElement;
    dialog.dispatchEvent(new Event('cancel', { bubbles: false, cancelable: true }));
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('[role="alertdialog"]')).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === 'delete_repository_branch')).toBe(false);
    expect(invoke.mock.calls.some(([command]) => command === 'remove_repository_worktree')).toBe(false);
  });

  it('closes the dialog and focuses the error when branch deletion is rejected', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'delete_repository_branch'
        ? Promise.reject(new Error('branch is checked out in another worktree'))
        : defaultIpc(command, request);
    const { fixture } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (local.querySelector('[aria-label="Delete local branch rb/feature"]') as HTMLButtonElement).click();
    fixture.detectChanges();

    (fixture.nativeElement.querySelector('#confirm-reference-deletion') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('[role="alertdialog"]')).toBeNull();
    expect(fixture.nativeElement.querySelector('#navigation-action-error').textContent).toContain(
      'branch is checked out in another worktree',
    );
    await vi.waitFor(() => expect(globalThis.document.activeElement?.id).toBe('navigation-action-error'));
  });

  it('runs opt-in auto fetch and live status refreshes and clears their intervals on destroy', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    vi.useFakeTimers();
    const element = fixture.nativeElement as HTMLElement;
    const toggles = [...element.querySelectorAll<HTMLLabelElement>('.toggle-control')];
    const autoFetch = toggles.find((label) => label.textContent?.includes('Auto fetch'))?.querySelector('input') as HTMLInputElement;
    const liveChanges = toggles.find((label) => label.textContent?.includes('Live changes'))?.querySelector('input') as HTMLInputElement;

    autoFetch.click();
    liveChanges.click();
    fixture.detectChanges();
    await vi.advanceTimersByTimeAsync(60_000);

    expect(invoke.mock.calls.filter(([command]) => command === 'repository_fetch').length).toBeGreaterThanOrEqual(2);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_status').length).toBeGreaterThan(2);
    expect(globalThis.localStorage.getItem('skibidibi-git.workspace.auto-fetch.skibidibi-git')).toBe('true');
    expect(globalThis.localStorage.getItem('skibidibi-git.workspace.live-changes.skibidibi-git')).toBe('true');

    fixture.destroy();
    const callsAfterDestroy = invoke.mock.calls.length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(invoke.mock.calls).toHaveLength(callsAfterDestroy);
    vi.useRealTimers();
  });

  it('opens a worktree as a grouped repository from its dropdown menu', async () => {
    const worktreeRepository = {
      ...rememberedRepository,
      id: 'feature-tree',
      repositoryGroupId: 'shared-git-directory',
      worktreeRole: 'linked' as const,
      canonicalPath: '/work/feature-tree',
      displayName: 'feature-tree',
    };
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'remember_repository' ? Promise.resolve(worktreeRepository) : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const router = TestBed.inject(Router);
    const catalog = TestBed.inject(RepositoryCatalog);
    const navigateByUrl = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    const trigger = fixture.nativeElement.querySelector(
      '[aria-label="Worktree actions /work/feature-tree"]',
    ) as HTMLButtonElement;
    trigger.click();
    fixture.detectChanges();
    const menu = fixture.nativeElement.querySelector('#worktree-context-menu') as HTMLElement;
    expect(menu).not.toBeNull();
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(menu.textContent).toContain('Open as new repository');

    (menu.querySelector('[role="menuitem"]') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('remember_repository', { repositoryPath: '/work/feature-tree' });
    expect(catalog.find('feature-tree')).toMatchObject({
      repositoryGroupId: 'shared-git-directory',
      worktreeRole: 'linked',
      path: '/work/feature-tree',
    });
    expect(navigateByUrl).toHaveBeenCalledWith('/repositories', { skipLocationChange: true });
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'feature-tree', 'history']);
  });

  it('shows submodule status and opens its repository through the dedicated IPC command', async () => {
    const submoduleRepository = {
      ...rememberedRepository,
      id: 'vendor-module',
      repositoryGroupId: 'vendor-module-group',
      canonicalPath: '/work/skibidibi-git/vendor/module',
      displayName: 'module',
    };
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_submodules') {
        return Promise.resolve({
          submodules: [{
            name: 'module',
            path: 'vendor/module',
            url: 'git@github.com:example/module.git',
            expectedOid: 'expected',
            currentOid: 'current',
            present: true,
            initialized: true,
            commitState: 'different',
            worktreeState: 'modified',
            changeCount: 2,
          }],
        });
      }
      if (command === 'open_submodule_repository') {
        return Promise.resolve(submoduleRepository);
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    const router = TestBed.inject(Router);
    const navigateByUrl = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.textContent).toContain('commit differs');
    });
    const open = fixture.nativeElement.querySelector(
      '[aria-label="Open submodule module"]',
    ) as HTMLButtonElement;
    expect(open.textContent).toContain('modified · 2');
    open.click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('open_submodule_repository', {
      parentRepositoryId: 'skibidibi-git',
      path: 'vendor/module',
    });
    expect(navigateByUrl).toHaveBeenCalledWith('/repositories', { skipLocationChange: true });
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'vendor-module', 'history']);
  });

  it('closes the worktree dropdown with Escape and restores focus', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const trigger = fixture.nativeElement.querySelector(
      '[aria-label="Worktree actions /work/feature-tree"]',
    ) as HTMLButtonElement;

    trigger.click();
    fixture.detectChanges();
    globalThis.document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('#worktree-context-menu')).toBeNull();
    await vi.waitFor(() => expect(globalThis.document.activeElement).toBe(trigger));
  });

  it('labels and disables the current, bare, and prunable worktrees', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command !== 'repository_navigation') {
        return defaultIpc(command, request);
      }
      return Promise.resolve({
        branches: [],
        worktrees: [
          { path: '/work/skibidibi-git', head: 'abc', branch: 'refs/heads/main', detached: false, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null },
          { path: '/work/bare.git', head: 'def', branch: 'refs/heads/archive', detached: false, bare: true, locked: false, lockReason: null, prunable: false, prunableReason: null },
          { path: '/work/gone', head: 'ghi', branch: 'refs/heads/old', detached: false, bare: false, locked: false, lockReason: null, prunable: true, prunableReason: 'directory missing' },
        ],
        stashes: [],
      });
    };
    const { fixture, invoke } = await createFixture(ipc);
    const rows = fixture.nativeElement.querySelectorAll('.worktree-row') as NodeListOf<HTMLButtonElement>;

    expect(rows).toHaveLength(3);
    expect(rows[0].disabled).toBe(true);
    expect(rows[0].querySelector('.worktree-identity > strong')?.textContent).toBe('skibidibi-git');
    expect(rows[0].textContent).toContain('main');
    expect(rows[0].textContent).not.toContain('refs/heads');
    expect(rows[0].textContent).toContain('current');
    expect(rows[1].disabled).toBe(true);
    expect(rows[1].querySelector('.worktree-identity > strong')?.textContent).toBe('bare.git');
    expect(rows[1].textContent).toContain('bare');
    expect(rows[2].disabled).toBe(true);
    expect(rows[2].querySelector('.worktree-identity > strong')?.textContent).toBe('gone');
    expect(rows[2].textContent).toContain('prunable');

    rows.forEach((row) => row.click());
    expect(invoke.mock.calls.filter(([command]) => command === 'remember_repository')).toHaveLength(0);
  });

  it('resizes the sidebar from the keyboard and persists the width defensively', async () => {
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width.v2');
    const { fixture } = await createFixture(defaultIpc);
    const separator = fixture.nativeElement.querySelector('.sidebar-resizer') as HTMLElement;

    separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    fixture.detectChanges();

    expect(separator.getAttribute('aria-valuenow')).toBe('266');
    expect(globalThis.localStorage.getItem('skibidibi-git.workspace.sidebar-width.v2')).toBe('266');
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width.v2');
  });

  it('reserves the history and inspector columns near the 58rem breakpoint', async () => {
    const originalWidth = globalThis.innerWidth;
    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: 1200 });
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width.v2');
    const { fixture } = await createFixture(defaultIpc);
    const separator = fixture.nativeElement.querySelector('.sidebar-resizer') as HTMLElement;
    separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }));
    fixture.detectChanges();
    expect(separator.getAttribute('aria-valuenow')).toBe('600');

    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: 1100 });
    globalThis.dispatchEvent(new Event('resize'));
    fixture.detectChanges();
    const clampedWidth = Number(separator.getAttribute('aria-valuenow'));

    expect(clampedWidth).toBe(527);
    expect(clampedWidth + 18 * 16 + 17 * 16 + 2 * 0.4 * 16).toBeLessThanOrEqual(1100);
    fixture.destroy();
    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: originalWidth });
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width.v2');
  });

  it('resizes, persists, maximizes, and restores the inspector independently', async () => {
    const originalWidth = globalThis.innerWidth;
    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: 1440 });
    globalThis.localStorage.removeItem('skibidibi-git.workspace.inspector-width.v2');
    const { fixture } = await createFixture(defaultIpc);
    const separator = fixture.nativeElement.querySelector('.inspector-resizer') as HTMLElement;

    expect(separator.getAttribute('aria-valuenow')).toBe('296');
    separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }));
    fixture.detectChanges();
    expect(separator.getAttribute('aria-valuenow')).toBe('312');
    expect(globalThis.localStorage.getItem('skibidibi-git.workspace.inspector-width.v2')).toBe('312');

    const maximize = fixture.nativeElement.querySelector('.inspector-maximize') as HTMLButtonElement;
    maximize.click();
    fixture.detectChanges();
    expect(maximize.getAttribute('aria-pressed')).toBe('true');
    expect(maximize.getAttribute('aria-label')).toBe('Restore inspector size');
    expect(fixture.nativeElement.querySelector('.workspace-grid').classList).toContain('inspector-maximized');
    maximize.click();
    fixture.detectChanges();
    expect(maximize.getAttribute('aria-pressed')).toBe('false');

    fixture.destroy();
    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: originalWidth });
    globalThis.localStorage.removeItem('skibidibi-git.workspace.inspector-width.v2');
  });

  it('loads and displays selected commit details with changed-file totals', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const inspector = fixture.nativeElement.querySelector('.inspector') as HTMLElement;
    expect(inspector.textContent).toContain('Add repository history');
    expect(inspector.textContent).toContain('1 changed file');
    expect(inspector.textContent).toContain('src/history.ts');
    expect(inspector.querySelector('.file-directory')?.textContent).toBe('src');
    expect(inspector.querySelector('.file-name')?.textContent).toBe('history.ts');
    expect(inspector.textContent).toContain('+12');
    expect(inspector.querySelector('.commit-meta')?.textContent).toContain('Author');
    expect(inspector.querySelector('.commit-meta')?.textContent).toContain('Email');
    expect(inspector.querySelector('.commit-meta')?.textContent).toContain('Authored');
    expect(inspector.querySelector('.file-summary-stats')?.textContent).toContain('−3');
    expect(invoke).toHaveBeenCalledWith('repository_commit_detail', {
      repositoryId: 'skibidibi-git',
      oid: commits[0].oid,
    });
  });

  it('shows the backend reason when selected commit details fail', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_commit_detail') {
        return Promise.reject({ message: 'Commit file summaries were inconsistent.' });
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);

    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain(
      'Commit file summaries were inconsistent.',
    );
  });

  it('loads the next history page without replacing existing commits', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.load-more') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelectorAll('.commit-row')).toHaveLength(3);
    expect(invoke).toHaveBeenCalledWith('repository_history', {
      repositoryId: 'skibidibi-git',
      cursor: 'page-2',
      limit: 50,
    });
    expect(fixture.nativeElement.querySelector('.load-more')).toBeNull();
  });

  it('ignores commit details that arrive after a newer selection', async () => {
    let resolveFirst!: (detail: RepositoryCommitDetailResponse) => void;
    let resolveSecond!: (detail: RepositoryCommitDetailResponse) => void;
    const firstDetail = new Promise<RepositoryCommitDetailResponse>((resolve) => {
      resolveFirst = resolve;
    });
    const secondDetail = new Promise<RepositoryCommitDetailResponse>((resolve) => {
      resolveSecond = resolve;
    });
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_commit_detail') {
        return (request as { oid: string }).oid === commits[0].oid ? firstDetail : secondDetail;
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    const rows = fixture.nativeElement.querySelectorAll('.commit-row') as NodeListOf<HTMLButtonElement>;

    rows[0].click();
    rows[1].click();
    resolveSecond(detailFor(commits[1]));
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain(
        'Create desktop shell',
      );
    });
    resolveFirst(detailFor(commits[0]));
    await Promise.resolve();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain(
      'Create desktop shell',
    );
    expect(fixture.nativeElement.querySelector('.inspector').textContent).not.toContain(
      'Add repository history',
    );
  });

  it('pins the current branch and worktree first and renders distinct tracking badges', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command !== 'repository_navigation') {
        return defaultIpc(command, request);
      }
      return Promise.resolve({
        branches: [
          { kind: 'local', fullName: 'refs/heads/rb/feature', name: 'rb/feature', oid: 'def', current: false, upstream: 'origin/rb/feature', ahead: 0, behind: 2, upstreamGone: false, symbolicTarget: null },
          { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: 'origin/main', ahead: 3, behind: 1, upstreamGone: true, symbolicTarget: null },
        ],
        worktrees: [
          { path: '/work/feature-tree', head: 'def', branch: 'refs/heads/rb/feature', detached: false, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null },
          { path: '/work/skibidibi-git', head: 'abc', branch: 'refs/heads/main', detached: false, bare: false, locked: false, lockReason: null, prunable: false, prunableReason: null },
        ],
        stashes: [],
      });
    };
    const { fixture } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    const localRows = local.querySelectorAll('.branch-row') as NodeListOf<HTMLElement>;
    const worktreeRows = fixture.nativeElement.querySelectorAll('.worktree-row') as NodeListOf<HTMLButtonElement>;

    expect(localRows[0].textContent).toContain('main');
    expect(localRows[0].querySelector('.ahead-token')?.textContent).toContain('↑3');
    expect(localRows[0].querySelector('.behind-token')?.textContent).toContain('↓1');
    expect(localRows[0].querySelector('.upstream-gone')?.textContent).toContain('gone');
    expect(localRows[1].textContent).toContain('feature');
    expect(worktreeRows[0].textContent).toContain('main');
    expect(worktreeRows[0].textContent).toContain('current');
  });

  it('opens a file diff and closes it without refetching commit details', async () => {
    const { fixture, invoke } = await createFixture(defaultIpc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const changedFile = fixture.nativeElement.querySelector('.changed-file') as HTMLButtonElement;
    changedFile.focus();
    changedFile.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain('new line');
    expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain('src/history.ts');
    expect(fixture.nativeElement.querySelector('.commit-list')).toBeNull();

    (fixture.nativeElement.querySelector('.close-diff') as HTMLButtonElement).click();
    await Promise.resolve();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelectorAll('.commit-row')).toHaveLength(2);
    expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain('Add repository history');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_commit_detail')).toHaveLength(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_file_diff')).toHaveLength(1);
    expect(globalThis.document.activeElement).toBe(changedFile);
    expect(invoke).toHaveBeenCalledWith('repository_file_diff', {
      repositoryId: 'skibidibi-git',
      oid: commits[0].oid,
      path: 'src/history.ts',
      oldPath: null,
    });
  });

  it('shows contextual change sections by default and toggles the full diff without IPC', async () => {
    const patch = [
      'diff --git a/src/history.ts b/src/history.ts',
      '--- a/src/history.ts',
      '+++ b/src/history.ts',
      '@@ -1,13 +1,13 @@',
      '-first old',
      '+first new',
      ' context-1',
      ' context-2',
      ' context-3',
      ' hidden-middle',
      ' context-5',
      ' context-6',
      ' context-7',
      '-second old',
      '+second new',
    ].join('\n');
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_file_diff'
        ? Promise.resolve({
            oid: commits[0].oid,
            path: 'src/history.ts',
            patch,
            binary: false,
            truncated: false,
          })
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.changed-file') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const contextual = fixture.nativeElement.querySelector('.diff-table') as HTMLElement;
    expect(contextual.textContent).toContain('1 unchanged line hidden');
    expect(contextual.textContent).not.toContain('hidden-middle');

    (fixture.nativeElement.querySelector('.diff-mode-toggle') as HTMLButtonElement).click();
    fixture.detectChanges();

    expect((fixture.nativeElement.querySelector('.diff-table') as HTMLElement).textContent).toContain(
      'hidden-middle',
    );
    expect(fixture.nativeElement.querySelector('.diff-mode-toggle')?.textContent).toContain(
      'Show changes only',
    );
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_file_diff')).toHaveLength(1);
  });

  it('ignores a stale file diff after another file is selected', async () => {
    let resolveFirst!: (response: unknown) => void;
    let resolveSecond!: (response: unknown) => void;
    const first = new Promise((resolve) => { resolveFirst = resolve; });
    const second = new Promise((resolve) => { resolveSecond = resolve; });
    const files = [
      { path: 'src/first.ts', oldPath: null, status: 'modified' as const, additions: 1, deletions: 1, binary: false },
      { path: 'src/second.ts', oldPath: null, status: 'added' as const, additions: 1, deletions: 0, binary: false },
    ];
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_commit_detail') {
        return Promise.resolve({ ...detailFor(commits[0]), files });
      }
      if (command === 'repository_file_diff') {
        return (request as { path: string }).path === files[0].path ? first : second;
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    const buttons = fixture.nativeElement.querySelectorAll('.changed-file') as NodeListOf<HTMLButtonElement>;
    buttons[0].click();
    buttons[1].click();
    resolveSecond({ oid: commits[0].oid, path: files[1].path, binary: false, patch: `diff --git a/${files[1].path} b/${files[1].path}\n--- a/${files[1].path}\n+++ b/${files[1].path}\n@@ -0,0 +1 @@\n+second content` });
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain('second content');
    });
    resolveFirst({ oid: commits[0].oid, path: files[0].path, binary: false, patch: `diff --git a/${files[0].path} b/${files[0].path}\n--- a/${files[0].path}\n+++ b/${files[0].path}\n@@ -0,0 +1 @@\n+stale content` });
    await Promise.resolve();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain('second content');
    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).not.toContain('stale content');
    expect(fixture.nativeElement.querySelector('.changed-file.selected')?.textContent).toContain('second.ts');
  });

  it('shows honest binary and error states for file diffs', async () => {
    let mode: 'binary' | 'error' = 'binary';
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_file_diff') {
        return mode === 'binary'
          ? Promise.resolve({ oid: commits[0].oid, path: 'src/history.ts', patch: '', binary: true })
          : Promise.reject(new Error('diff output exceeded the safe limit'));
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.commit-row') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.changed-file') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('.diff-state')?.textContent).toContain('Binary file');

    (fixture.nativeElement.querySelector('.close-diff') as HTMLButtonElement).click();
    fixture.detectChanges();
    mode = 'error';
    (fixture.nativeElement.querySelector('.changed-file') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.diff-state.error-state')?.textContent).toContain('diff output exceeded the safe limit');
    expect(fixture.nativeElement.querySelector('.diff-state.error-state')?.textContent).toContain('Try again');
  });

  it('shows a retryable empty-data error without fabricated commits', async () => {
    const failingIpc = (command: string): Promise<unknown> => {
      if (command === 'list_remembered_repositories') {
        return Promise.resolve([rememberedRepository]);
      }
      if (command === 'repository_status') {
        return Promise.resolve(repositoryStatus());
      }
      return Promise.reject(new Error('history failed'));
    };
    const { fixture } = await createFixture(failingIpc);

    expect(fixture.nativeElement.querySelectorAll('.commit-row')).toHaveLength(0);
    expect(fixture.nativeElement.textContent).toContain('History unavailable');
    expect(fixture.nativeElement.textContent).toContain('Try again');
  });

  it('retries a dirty pull with the selected strategy and a dated auto-stash message', async () => {
    let pulls = 0;
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_pull' && ++pulls === 1) {
        return Promise.reject(new Error('Pull requires a clean working tree'));
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    const strategy = fixture.nativeElement.querySelector('[aria-label="Pull strategy"]') as HTMLSelectElement;
    strategy.value = 'rebase';
    strategy.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    const pull = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.network-actions button')]
      .find((button) => button.textContent?.trim() === 'Pull')!;
    pull.click();
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.querySelector('#destructive-action-description')?.textContent)
        .toContain('Pull requires a clean working tree');
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_pull')).toHaveLength(1);
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.textContent).toContain('Pull succeeded.');
    });

    const calls = invoke.mock.calls.filter(([command]) => command === 'repository_pull');
    expect(calls).toHaveLength(2);
    expect(calls[0][1].operation).toMatchObject({ strategy: 'rebase', autoStash: null });
    expect(calls[1][1].operation.strategy).toBe('rebase');
    expect(calls[1][1].operation.autoStash.message).toMatch(/^WIP \d{4}-\d{2}-\d{2}T.* main$/);
    expect(fixture.nativeElement.textContent).toContain('create=created, restore=applied, cleanup=dropped');
  });

  it('does not offer auto-stash for an unrelated pull failure', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'repository_pull'
        ? Promise.reject(new Error('authentication failed'))
        : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const pull = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.network-actions button')]
      .find((button) => button.textContent?.trim() === 'Pull')!;
    pull.click();

    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(fixture.nativeElement.textContent).toContain('authentication failed');
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_pull')).toHaveLength(1);
    expect(fixture.nativeElement.querySelector('#destructive-action-description')).toBeNull();
  });

  it('blocks unsafe push analysis and confirms origin setup when no upstream exists', async () => {
    let readiness: 'behind' | 'noUpstream' = 'behind';
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        const status = repositoryStatus();
        return Promise.resolve({ ...status, branch: { ...status.branch, upstream: readiness === 'noUpstream' ? null : 'origin/main' } });
      }
      if (command === 'repository_push_analysis') {
        return Promise.resolve({ branch: 'main', head: 'abc', upstream: readiness === 'noUpstream' ? null : 'origin/main', remote: readiness === 'noUpstream' ? null : 'origin', remoteRef: readiness === 'noUpstream' ? null : 'refs/heads/main', ahead: 1, behind: readiness === 'behind' ? 1 : 0, readiness });
      }
      return defaultIpc(command, request);
    };
    let created = await createFixture(ipc);
    let push = [...(created.fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.network-actions button')]
      .find((button) => button.textContent?.trim() === 'Push')!;
    expect(push.disabled).toBe(true);
    expect(push.title).toContain('behind');
    expect(created.invoke.mock.calls.some(([command]) => command === 'repository_push')).toBe(false);
    created.fixture.destroy();
    TestBed.resetTestingModule();

    readiness = 'noUpstream';
    created = await createFixture(ipc);
    push = [...(created.fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.network-actions button')]
      .find((button) => button.textContent?.trim() === 'Push')!;
    push.click();
    await vi.waitFor(() => {
      created.fixture.detectChanges();
      expect(created.fixture.nativeElement.querySelector('#destructive-action-description')?.textContent)
        .toContain('Push “main” to origin/main and set it as upstream?');
    });
    expect(created.invoke.mock.calls.some(([command]) => command === 'repository_push')).toBe(false);
    (created.fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(created.invoke.mock.calls.some(([command]) => command === 'repository_push')).toBe(true));
    const pushCall = created.invoke.mock.calls.find(([command]) => command === 'repository_push')!;
    expect(pushCall[1].operation.target).toEqual({ kind: 'setUpstream', remote: 'origin', remoteBranch: 'main' });
  });

  it('requires explicit confirmation before setting a natural upstream and supports cancel', async () => {
    const upstreamStatus = {
      ...repositoryStatus(),
      branch: { ...repositoryStatus().branch, upstream: null },
    };
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        return Promise.resolve(upstreamStatus);
      }
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'abc', current: true, upstream: null, ahead: 1, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'remote', fullName: 'refs/remotes/origin/main', name: 'origin/main', oid: 'origin-abc', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [],
          stashes: [],
        });
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    const remoteBranch = {
      kind: 'remote' as const,
      fullName: 'refs/remotes/origin/main',
      name: 'origin/main',
      oid: 'origin-abc',
      current: false,
      upstream: null,
      ahead: 0,
      behind: 0,
      upstreamGone: false,
      symbolicTarget: null,
    };
    (fixture.componentInstance as unknown as { setNaturalUpstream(branch: typeof remoteBranch): Promise<void> })
      .setNaturalUpstream(remoteBranch);
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-description')?.textContent)
      .toContain('Set origin/main as the upstream for main?');
    expect(invoke.mock.calls.some(([command]) => command === 'repository_set_upstream')).toBe(false);

    (fixture.nativeElement.querySelector('#cancel-destructive-action') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(invoke.mock.calls.some(([command]) => command === 'repository_set_upstream')).toBe(false);

    (fixture.componentInstance as unknown as { setNaturalUpstream(branch: typeof remoteBranch): Promise<void> })
      .setNaturalUpstream(remoteBranch);
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('#confirm-destructive-action') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'repository_set_upstream')).toBe(true));
    expect(invoke).toHaveBeenCalledWith('repository_set_upstream', {
      repositoryId: 'skibidibi-git',
      operation: expect.objectContaining({ remoteFullName: 'refs/remotes/origin/main', expectedOid: 'origin-abc' }),
    });
  });

  it('loads exact conflict stages and resolves edited content with repository preconditions', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        return Promise.resolve(conflictedStatus());
      }
      if (command === 'repository_conflicts') {
        return Promise.resolve({ files: [textConflict], status: conflictedStatus() });
      }
      if (command === 'repository_conflict_detail') {
        return Promise.resolve(conflictDetail());
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.conflict-files button') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_conflict_detail', {
      repositoryId: 'skibidibi-git',
      operation: {
        path: textConflict.path,
        expectedBase: textConflict.base,
        expectedOurs: textConflict.ours,
        expectedTheirs: textConflict.theirs,
      },
    });
    const result = fixture.nativeElement.querySelector('[aria-label="Resolved content"]') as HTMLTextAreaElement;
    expect(result.value).toContain('<<<<<<<');
    result.value = 'final content\n';
    result.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    const resolve = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.conflict-actions button')]
      .find((button) => button.textContent?.includes('edited content'))!;
    resolve.click();
    await vi.waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'repository_resolve_conflict')).toBe(true));
    const resolveCall = invoke.mock.calls.find(([command]) => command === 'repository_resolve_conflict')!;
    expect(resolveCall[1].operation).toMatchObject({
      path: textConflict.path,
      expectedBase: textConflict.base,
      expectedOurs: textConflict.ours,
      expectedTheirs: textConflict.theirs,
      resolution: { kind: 'content', content: 'final content\n' },
      precondition: { expectedHead: 'abc', expectedIndexFingerprint: 'index-before', expectedWorktreeFingerprint: 'worktree-before' },
    });
  });

  it('requires confirmation before staging unchanged conflict markers', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        return Promise.resolve(conflictedStatus());
      }
      if (command === 'repository_conflicts') {
        return Promise.resolve({ files: [textConflict], status: conflictedStatus() });
      }
      if (command === 'repository_conflict_detail') {
        return Promise.resolve(conflictDetail());
      }
      return defaultIpc(command, request);
    };
    const { fixture, invoke } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.conflict-files button') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    const resolve = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.conflict-actions button')]
      .find((button) => button.textContent?.includes('edited content'))!;
    resolve.click();

    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('#destructive-action-description')?.textContent)
      .toContain('The resolved content is unchanged or still contains conflict markers.');
    (fixture.nativeElement.querySelector('#cancel-destructive-action') as HTMLButtonElement).click();
    expect(invoke.mock.calls.some(([command]) => command === 'repository_resolve_conflict')).toBe(false);
  });

  it('offers only side selection or deletion for a binary conflict', async () => {
    const ipc = (command: string, request: unknown): Promise<unknown> => {
      if (command === 'repository_status') {
        return Promise.resolve(conflictedStatus());
      }
      if (command === 'repository_conflicts') {
        return Promise.resolve({ files: [textConflict], status: conflictedStatus() });
      }
      if (command === 'repository_conflict_detail') {
        return Promise.resolve(conflictDetail(true));
      }
      return defaultIpc(command, request);
    };
    const { fixture } = await createFixture(ipc);
    (fixture.nativeElement.querySelector('.working-tree-history-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('.conflict-files button') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    const actions = fixture.nativeElement.querySelector('.conflict-actions') as HTMLElement;
    expect(fixture.nativeElement.querySelector('[aria-label="Resolved content"]')).toBeNull();
    expect(actions.textContent).toContain('Use ours');
    expect(actions.textContent).toContain('Use theirs');
    expect(actions.textContent).toContain('Delete');
    expect(actions.textContent).not.toContain('edited content');
  });
});
