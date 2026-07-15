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
});
