import { TestBed } from '@angular/core/testing';

import { DesktopIpc, type RepositoryStatusResponse } from './desktop-ipc';

const repositoryStatusFixture: RepositoryStatusResponse = {
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

  it('returns honest empty repository navigation outside Tauri', async () => {
    await expect(
      service.invoke('repository_navigation', { repositoryId: 'example-repository' }),
    ).resolves.toEqual({ branches: [], worktrees: [], stashes: [] });
  });

  it('does not pretend to switch branches outside Tauri', async () => {
    await expect(
      service.invoke('switch_repository_branch', {
        repositoryId: 'example-repository',
        fullName: 'refs/heads/feature',
      }),
    ).rejects.toThrow('unavailable outside the desktop application');
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
