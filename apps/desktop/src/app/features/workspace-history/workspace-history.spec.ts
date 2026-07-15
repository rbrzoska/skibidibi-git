import { ComponentFixture, TestBed } from '@angular/core/testing';
import { ActivatedRoute, Router, provideRouter } from '@angular/router';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RepositoryCommitDetailResponse,
  type RepositoryCommitSummary,
} from '../../core/ipc/desktop-ipc';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { WorkspaceHistory } from './workspace-history';

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

function repositoryStatus() {
  return {
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

describe('WorkspaceHistory', () => {
  async function createFixture(
    invokeImplementation: (command: string, request: unknown) => Promise<unknown>,
  ): Promise<{ fixture: ComponentFixture<WorkspaceHistory>; invoke: ReturnType<typeof vi.fn> }> {
    const invoke = vi.fn().mockImplementation(invokeImplementation);
    const ipc = { invoke } as unknown as DesktopIpcClient;
    await TestBed.configureTestingModule({
      imports: [WorkspaceHistory],
      providers: [
        provideRouter([]),
        { provide: DESKTOP_IPC, useValue: ipc },
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

  function defaultIpc(command: string, request: unknown): Promise<unknown> {
    if (command === 'list_remembered_repositories') {
      return Promise.resolve([rememberedRepository]);
    }
    if (command === 'repository_status') {
      return Promise.resolve(repositoryStatus());
    }
    if (command === 'repository_history') {
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
    if (command === 'switch_repository_branch') {
      return Promise.resolve({ fullName: 'refs/heads/rb/feature', name: 'rb/feature', head: 'def', changed: true });
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
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  }

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
  });

  it('summarizes working tree changes above commit history', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const statusStore = TestBed.inject(RepositoryStatusStore);
    statusStore.setRepositoryPath('/work/skibidibi-git');
    statusStore.state.set({ kind: 'ready', status: repositoryStatus() });
    fixture.detectChanges();
    const summary = fixture.nativeElement.querySelector('.change-stats') as HTMLElement;

    expect(summary.textContent).toContain('2 changed');
    expect(summary.textContent).toContain('+1');
    expect(summary.textContent).toContain('~1');
  });

  it('separates local and remote branches and collapses slash-delimited folders', async () => {
    const { fixture } = await createFixture(defaultIpc);
    const element = fixture.nativeElement as HTMLElement;
    const local = element.querySelector('[aria-label="Local branches"]') as HTMLElement;
    const remote = element.querySelector('[aria-label="Remote branches"]') as HTMLElement;

    expect(local.textContent).toContain('main');
    expect(local.textContent).toContain('rb');
    expect(local.textContent).toContain('feature');
    expect(remote.textContent).toContain('origin');
    expect(remote.textContent).toContain('main');
    expect(remote.querySelector('button.branch-row')).toBeNull();

    (local.querySelector('.folder-row') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(local.textContent).not.toContain('feature');
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

    expect(local.textContent).not.toContain('local-feature');
    expect(remote.textContent).toContain('remote-feature');
    expect(remote.querySelector('.folder-row')?.getAttribute('aria-expanded')).toBe('true');
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

  it('confirms and switches a non-current local branch, then refreshes workspace data', async () => {
    const confirm = vi.spyOn(globalThis, 'confirm').mockReturnValue(true);
    const { fixture, invoke } = await createFixture(defaultIpc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    const branch = [...local.querySelectorAll<HTMLButtonElement>('button.branch-row')].find(
      (button) => button.title.includes('rb/feature'),
    );

    branch?.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(confirm).toHaveBeenCalledWith('Switch the active worktree to “rb/feature”?');
    expect(invoke).toHaveBeenCalledWith('switch_repository_branch', {
      repositoryId: 'skibidibi-git',
      fullName: 'refs/heads/rb/feature',
    });
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_navigation').length).toBeGreaterThan(1);
    confirm.mockRestore();
  });

  it('surfaces a dirty-worktree message returned as a structured Tauri rejection', async () => {
    const confirm = vi.spyOn(globalThis, 'confirm').mockReturnValue(true);
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'switch_repository_branch'
        ? Promise.reject({ message: 'Commit or stash your working tree changes before switching.' })
        : defaultIpc(command, request);
    const { fixture } = await createFixture(ipc);
    const local = fixture.nativeElement.querySelector('[aria-label="Local branches"]') as HTMLElement;
    const branch = [...local.querySelectorAll<HTMLButtonElement>('button.branch-row')].find(
      (button) => button.title.includes('rb/feature'),
    );

    branch?.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain(
      'Commit or stash your working tree changes before switching.',
    );
    confirm.mockRestore();
  });

  it('remembers a selected worktree and navigates to its workspace', async () => {
    const worktreeRepository = { ...rememberedRepository, id: 'feature-tree', canonicalPath: '/work/feature-tree', displayName: 'feature-tree' };
    const ipc = (command: string, request: unknown): Promise<unknown> =>
      command === 'remember_repository' ? Promise.resolve(worktreeRepository) : defaultIpc(command, request);
    const { fixture, invoke } = await createFixture(ipc);
    const router = TestBed.inject(Router);
    const navigateByUrl = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    (fixture.nativeElement.querySelector('.worktree-row') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('remember_repository', { repositoryPath: '/work/feature-tree' });
    expect(navigateByUrl).toHaveBeenCalledWith('/repositories', { skipLocationChange: true });
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'feature-tree', 'history']);
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
    expect(rows[0].textContent).toContain('main');
    expect(rows[0].textContent).not.toContain('refs/heads');
    expect(rows[0].textContent).toContain('current');
    expect(rows[1].disabled).toBe(true);
    expect(rows[1].textContent).toContain('bare');
    expect(rows[2].disabled).toBe(true);
    expect(rows[2].textContent).toContain('prunable');

    rows.forEach((row) => row.click());
    expect(invoke.mock.calls.filter(([command]) => command === 'remember_repository')).toHaveLength(0);
  });

  it('resizes the sidebar from the keyboard and persists the width defensively', async () => {
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width');
    const { fixture } = await createFixture(defaultIpc);
    const separator = fixture.nativeElement.querySelector('.sidebar-resizer') as HTMLElement;

    separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    fixture.detectChanges();

    expect(separator.getAttribute('aria-valuenow')).toBe('304');
    expect(globalThis.localStorage.getItem('skibidibi-git.workspace.sidebar-width')).toBe('304');
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width');
  });

  it('reserves the history and inspector columns near the 68rem breakpoint', async () => {
    const originalWidth = globalThis.innerWidth;
    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: 1200 });
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width');
    const { fixture } = await createFixture(defaultIpc);
    const separator = fixture.nativeElement.querySelector('.sidebar-resizer') as HTMLElement;
    separator.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }));
    fixture.detectChanges();
    expect(separator.getAttribute('aria-valuenow')).toBe('537');

    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: 1100 });
    globalThis.dispatchEvent(new Event('resize'));
    fixture.detectChanges();
    const clampedWidth = Number(separator.getAttribute('aria-valuenow'));

    expect(clampedWidth).toBe(437);
    expect(clampedWidth + 24 * 16 + 17 * 16 + 0.4 * 16).toBeLessThanOrEqual(1100);
    fixture.destroy();
    Object.defineProperty(globalThis, 'innerWidth', { configurable: true, value: originalWidth });
    globalThis.localStorage.removeItem('skibidibi-git.workspace.sidebar-width');
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
    expect(inspector.textContent).toContain('+12');
    expect(invoke).toHaveBeenCalledWith('repository_commit_detail', {
      repositoryId: 'skibidibi-git',
      oid: commits[0].oid,
    });
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
    const localRows = fixture.nativeElement.querySelectorAll('[aria-label="Local branches"] .branch-row') as NodeListOf<HTMLElement>;
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

    (fixture.nativeElement.querySelector('.changed-file') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain('new line');
    expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain('src/history.ts');
    expect(fixture.nativeElement.querySelector('.commit-list')).toBeNull();

    (fixture.nativeElement.querySelector('.close-diff') as HTMLButtonElement).click();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelectorAll('.commit-row')).toHaveLength(2);
    expect(fixture.nativeElement.querySelector('.inspector').textContent).toContain('Add repository history');
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_commit_detail')).toHaveLength(1);
    expect(invoke.mock.calls.filter(([command]) => command === 'repository_file_diff')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('repository_file_diff', {
      repositoryId: 'skibidibi-git',
      oid: commits[0].oid,
      path: 'src/history.ts',
      oldPath: null,
    });
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
});
