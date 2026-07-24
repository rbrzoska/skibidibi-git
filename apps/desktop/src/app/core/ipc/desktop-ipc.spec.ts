import { TestBed } from '@angular/core/testing';

import { DesktopIpc, type RepositoryStatusResponse } from './desktop-ipc';

const repositoryStatusFixture: RepositoryStatusResponse = {
  indexFingerprint: 'browser-development-index',
  worktreeFingerprint: 'browser-development-worktree',
  branch: {
    oid: 'a1b2c3d4e5f6',
    head: 'main',
    upstream: 'origin/main',
    ahead: 0,
    behind: 0,
    detached: false,
    unborn: false,
  },
  entries: [
    {
      kind: 'ordinary',
      path: 'src/app/app.ts',
      originalPath: null,
      indexStatus: 'unmodified',
      worktreeStatus: 'modified',
      submodule: null,
    },
  ],
};

describe('DesktopIpc', () => {
  let service: DesktopIpc;

  beforeEach(() => {
    TestBed.configureTestingModule({});
    service = TestBed.inject(DesktopIpc);
  });

  it('echoes the requested application zoom in browser development mode', async () => {
    await expect(service.invoke('set_application_zoom', { scale: 1.2 })).resolves.toEqual({
      scale: 1.2,
    });
  });

  it('reports every AI CLI as unavailable outside the desktop application', async () => {
    await expect(service.invoke('ai_cli_status', {})).resolves.toEqual({
      statuses: [
        expect.objectContaining({ provider: 'codex', displayName: 'Codex', available: false, version: null }),
        expect.objectContaining({ provider: 'claude', displayName: 'Claude Code', available: false, version: null }),
        expect.objectContaining({ provider: 'cursor', displayName: 'Cursor', available: false, version: null }),
      ],
    });
  });

  it('does not fabricate AI-generated commit messages outside the desktop application', async () => {
    await expect(service.invoke('ai_generate_commit_message', {
      repositoryId: 'example-repository',
      provider: 'codex',
      promptTemplate: 'Write one concise English sentence.',
      expectedHead: 'abc123',
      indexFingerprint: 'fixture-index',
      worktreeFingerprint: 'fixture-worktree',
    })).rejects.toThrow('unavailable outside the desktop application');
  });

  it('provides bounded development diagnostics without inventing log entries', async () => {
    await expect(service.invoke('diagnostics_settings', {})).resolves.toEqual({
      dataDirectory: '~/.skibidibi-git',
      maxLogKilobytes: 256,
      logFile: '~/.skibidibi-git/diagnostics.jsonl',
    });
    await expect(service.invoke('diagnostics_read', {})).resolves.toEqual({
      entries: [], totalBytes: 0, truncated: false,
    });
    await expect(service.invoke('select_diagnostics_directory', { initialPath: null }))
      .resolves.toEqual({ path: null });
  });

  it('provides the repository-status DTO fallback outside Tauri', async () => {
    await expect(
      service.invoke('repository_status', { repositoryPath: '/work/skibidibi-git' }),
    ).resolves.toEqual(repositoryStatusFixture);
  });

  it('keeps the StatusEntry contract intact', async () => {
    const response = await service.invoke('repository_status', {
      repositoryPath: '/work/skibidibi-git',
    });

    expect(response.entries[0]).toEqual(repositoryStatusFixture.entries[0]);
  });

  it('treats directory selection as cancelled outside Tauri', async () => {
    await expect(
      service.invoke('select_repository_directory', { initialPath: null }),
    ).resolves.toEqual({ path: null });
  });

  it('starts with an empty remembered-repository catalog outside Tauri', async () => {
    await expect(service.invoke('list_remembered_repositories', {})).resolves.toEqual([]);
  });

  it('returns an honest empty history outside Tauri', async () => {
    await expect(
      service.invoke('repository_history', {
        repositoryId: 'example-repository',
        cursor: null,
        limit: 50,
      }),
    ).resolves.toEqual({ commits: [], nextCursor: null });
  });

  it('does not fabricate commit details outside Tauri', async () => {
    await expect(
      service.invoke('repository_commit_detail', {
        repositoryId: 'example-repository',
        oid: 'abc123',
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not fabricate file diffs outside Tauri', async () => {
    await expect(
      service.invoke('repository_file_diff', {
        repositoryId: 'example-repository',
        oid: 'abc123',
        path: 'src/app.ts',
        oldPath: null,
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not emulate mutating commit operations outside Tauri', async () => {
    const precondition = {
      expectedHead: 'a'.repeat(40),
      expectedHeadName: 'main',
      expectedDetached: false,
      expectedUnborn: false,
      expectedIndexFingerprint: 'index-v1:test',
      expectedWorktreeFingerprint: 'worktree-v1:test',
    };

    await expect(service.invoke('repository_cherry_pick_commit', {
      repositoryId: 'example-repository',
      operation: { targetOid: 'b'.repeat(40), precondition },
    })).rejects.toThrow('Commit operations are unavailable');
    await expect(service.invoke('repository_revert_commit', {
      repositoryId: 'example-repository',
      operation: { targetOid: 'b'.repeat(40), precondition },
    })).rejects.toThrow('Commit operations are unavailable');
    await expect(service.invoke('repository_reset_commit', {
      repositoryId: 'example-repository',
      operation: {
        targetOid: 'b'.repeat(40),
        mode: 'hard',
        confirmHardReset: true,
        precondition,
      },
    })).rejects.toThrow('Commit operations are unavailable');
  });

  it('does not fabricate stash details or stash file diffs outside Tauri', async () => {
    const oid = 'abcdefabcdefabcdefabcdefabcdefabcdefabcd';
    await expect(
      service.invoke('repository_stash_detail', {
        repositoryId: 'example-repository',
        oid,
      }),
    ).rejects.toThrow('unavailable outside the desktop application');

    await expect(
      service.invoke('repository_stash_file_diff', {
        repositoryId: 'example-repository',
        oid,
        source: 'tracked',
        path: 'new name.txt',
        oldPath: 'old name.txt',
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not fabricate working-tree file diffs outside Tauri', async () => {
    await expect(
      service.invoke('repository_working_tree_file_diff', {
        repositoryId: 'example-repository',
        path: 'src/app.ts',
        oldPath: null,
        entryKind: 'ordinary',
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to stage changes outside Tauri', async () => {
    await expect(
      service.invoke('repository_apply_index_change', {
        repositoryId: 'example-repository',
        operation: {
          action: 'stage',
          selection: {
            scope: 'selected',
            entries: [{ path: 'src/app.ts', oldPath: null, entryKind: 'ordinary' }],
          },
          expectedHead: 'abc123',
          expectedHeadName: 'main',
          expectedDetached: false,
          expectedUnborn: false,
          expectedIndexFingerprint: 'fixture-index',
          expectedWorktreeFingerprint: 'fixture-worktree',
        },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to discard working-tree changes outside Tauri', async () => {
    const precondition = {
      expectedHead: 'abc123', expectedHeadName: 'main', expectedDetached: false, expectedUnborn: false,
      expectedIndexFingerprint: 'fixture-index', expectedWorktreeFingerprint: 'fixture-worktree',
    };
    await expect(service.invoke('repository_discard_worktree_changes', {
      repositoryId: 'example-repository',
      operation: {
        ...precondition,
        entries: [{ path: 'src/app.ts', oldPath: null, entryKind: 'ordinary' }],
      },
    })).rejects.toThrow('unavailable outside the desktop application');
    await expect(service.invoke('repository_discard_worktree_hunk', {
      repositoryId: 'example-repository',
      operation: {
        ...precondition,
        entry: { path: 'src/app.ts', oldPath: null, entryKind: 'ordinary' },
        patch: '@@ -1 +1 @@\n-old\n+new\n',
      },
    })).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to create commits outside Tauri', async () => {
    await expect(
      service.invoke('repository_create_commit', {
        repositoryId: 'example-repository',
        operation: {
          message: 'A real commit',
          expectedHead: 'abc123',
          expectedHeadName: 'main',
          expectedDetached: false,
          expectedUnborn: false,
          expectedIndexFingerprint: 'fixture-index',
          expectedWorktreeFingerprint: 'fixture-worktree',
        },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to amend commits outside Tauri', async () => {
    await expect(
      service.invoke('repository_amend_commit', {
        repositoryId: 'example-repository',
        operation: {
          message: null,
          confirmUpstreamRewrite: false,
          expectedHead: 'abc123',
          expectedHeadName: 'main',
          expectedDetached: false,
          expectedUnborn: false,
          expectedIndexFingerprint: 'fixture-index',
          expectedWorktreeFingerprint: 'fixture-worktree',
        },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('returns honest empty repository navigation outside Tauri', async () => {
    await expect(
      service.invoke('repository_navigation', { repositoryId: 'example-repository' }),
    ).resolves.toEqual({ branches: [], worktrees: [], stashes: [] });
  });

  it('does not fabricate reference comparisons outside Tauri', async () => {
    const request = {
      repositoryId: 'example-repository',
      sourceFullName: 'refs/heads/feature',
      expectedSourceOid: '0123456789012345678901234567890123456789',
      targetFullName: 'refs/heads/main',
      expectedTargetOid: 'abcdefabcdefabcdefabcdefabcdefabcdefabcd',
    };
    await expect(service.invoke('repository_compare_refs', request))
      .rejects.toThrow('unavailable outside the desktop application');
    await expect(service.invoke('repository_compare_ref_file_diff', {
      ...request,
      path: 'src/app.ts',
      oldPath: null,
    })).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to switch branches outside Tauri', async () => {
    await expect(
      service.invoke('switch_repository_branch', {
        repositoryId: 'example-repository',
        operation: {
          fullName: 'refs/heads/feature',
          expectedOid: '0123456789012345678901234567890123456789',
          stashOnDirty: false,
          stashMessage: null,
        },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to create branches outside Tauri', async () => {
    await expect(
      service.invoke('create_repository_branch', {
        repositoryId: 'example-repository',
        operation: {
          name: 'feature/safe',
          source: {
            kind: 'remoteTracking',
            fullName: 'refs/remotes/origin/feature/safe',
            expectedOid: '0123456789012345678901234567890123456789',
          },
        },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to mutate stashes outside Tauri', async () => {
    const precondition = {
      expectedHead: '0123456789012345678901234567890123456789',
      expectedHeadName: 'main',
      expectedDetached: false,
      expectedUnborn: false,
      expectedIndexFingerprint: 'fixture-index',
      expectedWorktreeFingerprint: 'fixture-worktree',
    };
    const stash = {
      oid: 'abcdefabcdefabcdefabcdefabcdefabcdefabcd',
      selector: 'stash@{0}',
    };

    await expect(
      service.invoke('repository_push_stash', {
        repositoryId: 'example-repository',
        operation: { message: 'WIP safe', includeUntracked: true, precondition },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
    await expect(
      service.invoke('repository_apply_stash', {
        repositoryId: 'example-repository',
        operation: { stash, restoreIndex: true, precondition },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
    await expect(
      service.invoke('repository_pop_stash', {
        repositoryId: 'example-repository',
        operation: { stash, restoreIndex: false, precondition },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
    await expect(
      service.invoke('repository_drop_stash', {
        repositoryId: 'example-repository',
        operation: { stash },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not fabricate network operations or conflict resolution outside Tauri', async () => {
    const repositoryId = 'example-repository';
    const precondition = {
      expectedHead: '0123456789012345678901234567890123456789',
      expectedHeadName: 'main',
      expectedDetached: false,
      expectedUnborn: false,
      expectedIndexFingerprint: 'fixture-index',
      expectedWorktreeFingerprint: 'fixture-worktree',
    };

    await expect(
      service.invoke('repository_push_analysis', { repositoryId }),
    ).rejects.toThrow('unavailable outside the desktop application');
    await expect(
      service.invoke('repository_pull', {
        repositoryId,
        operation: { strategy: 'ffOnly', autoStash: null, precondition },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
    await expect(
      service.invoke('repository_conflicts', { repositoryId }),
    ).rejects.toThrow('unavailable outside the desktop application');
    await expect(
      service.invoke('repository_resolve_conflict', {
        repositoryId,
        operation: {
          path: 'conflicted.txt',
          expectedBase: null,
          expectedOurs: { oid: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', mode: '100644' },
          expectedTheirs: { oid: 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', mode: '100644' },
          resolution: { kind: 'ours' },
          precondition,
        },
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
  });

  it('does not pretend to delete branches, worktrees, or fetch outside Tauri', async () => {
    await expect(service.invoke('delete_repository_branch', {
      repositoryId: 'example-repository',
      fullName: 'refs/heads/feature',
      expectedOid: 'abc123',
    })).rejects.toThrow('unavailable outside the desktop application');
    await expect(service.invoke('remove_repository_worktree', {
      repositoryId: 'example-repository',
      path: '/work/feature',
      expectedHead: 'abc123',
      branchFullName: 'refs/heads/feature',
      mode: 'safe',
      stashMessage: null,
    })).rejects.toThrow('unavailable outside the desktop application');
    await expect(service.invoke('repository_fetch', {
      repositoryId: 'example-repository',
    })).rejects.toThrow('unavailable outside the desktop application');
  });

  it('creates a local remembered-repository DTO in browser development', async () => {
    const repository = await service.invoke('remember_repository', {
      repositoryPath: '/work/example-repository',
    });

    expect(repository).toMatchObject({
      canonicalPath: '/work/example-repository',
      displayName: 'example-repository',
      provider: 'local',
      transport: 'local',
      availability: 'available',
    });
    expect(repository.id).toContain('example-repository');
  });
});
