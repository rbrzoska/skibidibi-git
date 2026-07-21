import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RememberedRepositoryResponse,
} from '../ipc/desktop-ipc';
import { RepositoryCatalog, type RepositoryCatalogGroup } from './repository-catalog';

const rememberedRepository: RememberedRepositoryResponse = {
  id: 'repository-id',
  repositoryGroupId: 'group-id',
  worktreeRole: 'main',
  canonicalPath: '/tmp/example-repository',
  displayName: 'example-repository',
  provider: 'github',
  transport: 'ssh',
  hostedIdentity: { host: 'github.com', owner: 'owner', name: 'example-repository' },
  availability: 'available',
  gitHealth: { state: 'healthy', issue: null, checkedAt: 10 },
  githubHealth: { state: 'healthy', issue: null, checkedAt: 10 },
  pinned: false,
  openCount: 1,
  lastOpenedAt: Math.floor(Date.now() / 1000),
  createdAt: 1,
  updatedAt: 10,
};

describe('RepositoryCatalog', () => {
  it('loads remembered repositories from desktop IPC', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue([rememberedRepository]),
    };
    TestBed.configureTestingModule({ providers: [{ provide: DESKTOP_IPC, useValue: ipc }] });
    const service = TestBed.inject(RepositoryCatalog);

    await service.load();

    expect(service.count()).toBe(1);
    expect(service.groupCount()).toBe(1);
    expect(service.find('repository-id')).toMatchObject({
      provider: 'github',
      transport: 'ssh',
      integration: 'connected',
    });
  });

  it('groups a main repository with linked worktrees and opens the newest available entry', async () => {
    const main = {
      ...rememberedRepository,
      id: 'main-id',
      canonicalPath: '/tmp/example-repository',
      worktreeRole: 'main' as const,
      lastOpenedAt: 100,
    };
    const linked = {
      ...rememberedRepository,
      id: 'linked-id',
      canonicalPath: '/tmp/example-worktree',
      displayName: 'feature-worktree',
      worktreeRole: 'linked' as const,
      lastOpenedAt: 200,
    };
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue([linked, main]),
    };
    TestBed.configureTestingModule({ providers: [{ provide: DESKTOP_IPC, useValue: ipc }] });
    const service = TestBed.inject(RepositoryCatalog);

    await service.load();

    expect(service.groups()).toHaveLength(1);
    expect(service.groups()[0].representative.id).toBe('main-id');
    expect(service.groups()[0].defaultRepository?.id).toBe('linked-id');
    expect(service.groups()[0].worktrees.map(({ id }) => id)).toEqual(['main-id', 'linked-id']);
  });

  it('keeps ungrouped entries and separate clones as distinct launcher rows', async () => {
    const repositories: RememberedRepositoryResponse[] = [
      { ...rememberedRepository, id: 'clone-a', repositoryGroupId: 'clone-group-a' },
      { ...rememberedRepository, id: 'clone-b', repositoryGroupId: 'clone-group-b' },
      { ...rememberedRepository, id: 'legacy-a', repositoryGroupId: null },
      { ...rememberedRepository, id: 'legacy-b', repositoryGroupId: null },
    ];
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue(repositories),
    };
    TestBed.configureTestingModule({ providers: [{ provide: DESKTOP_IPC, useValue: ipc }] });
    const service = TestBed.inject(RepositoryCatalog);

    await service.load();

    expect(service.groups()).toHaveLength(4);
    expect(new Set(service.groups().map(({ id }) => id)).size).toBe(4);
  });

  it('remembers a selected path without duplicating the returned repository', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue(rememberedRepository),
    };
    TestBed.configureTestingModule({ providers: [{ provide: DESKTOP_IPC, useValue: ipc }] });
    const service = TestBed.inject(RepositoryCatalog);

    const first = await service.rememberPath('/tmp/example-repository');
    const second = await service.rememberPath('/tmp/example-repository');

    expect(first.id).toBe(second.id);
    expect(service.repositories()[0].path).toBe('/tmp/example-repository');
    expect(service.count()).toBe(1);
  });

  it('nests a submodule group below its parent while retaining that child group worktrees', async () => {
    const childMain = {
      ...rememberedRepository,
      id: 'child-main',
      repositoryGroupId: 'child-group',
      canonicalPath: '/tmp/example-repository/vendor/module',
      displayName: 'module',
    };
    const childWorktree = {
      ...childMain,
      id: 'child-linked',
      canonicalPath: '/tmp/module-worktree',
      worktreeRole: 'linked' as const,
    };
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockImplementation((command: string) => {
        if (command === 'list_remembered_repositories') {
          return Promise.resolve([rememberedRepository, childMain, childWorktree]);
        }
        if (command === 'list_repository_relations') {
          return Promise.resolve([{
            parentRepositoryGroupId: 'group-id',
            childRepositoryGroupId: 'child-group',
            kind: 'submodule',
            relativePath: 'vendor/module',
          }]);
        }
        return Promise.resolve(undefined);
      }),
    };
    TestBed.configureTestingModule({ providers: [{ provide: DESKTOP_IPC, useValue: ipc }] });
    const service = TestBed.inject(RepositoryCatalog);

    await service.load();

    expect(service.groups()).toHaveLength(1);
    expect(service.groups()[0].submodules[0]).toMatchObject({
      repositoryGroupId: 'child-group',
      submodulePath: 'vendor/module',
    });
    expect(service.groups()[0].submodules[0].worktrees.map(({ id }) => id))
      .toEqual(['child-main', 'child-linked']);
  });

  it('cuts cyclic submodule relationships and sends the exact request to open a submodule', async () => {
    const child = {
      ...rememberedRepository,
      id: 'child-id',
      repositoryGroupId: 'child-group',
      canonicalPath: '/tmp/example-repository/child',
      displayName: 'child',
    };
    const opened = { ...child, id: 'opened-child' };
    const invoke = vi.fn().mockImplementation((command: string) => {
      if (command === 'list_remembered_repositories') {
        return Promise.resolve([rememberedRepository, child]);
      }
      if (command === 'list_repository_relations') {
        return Promise.resolve([
          { parentRepositoryGroupId: 'group-id', childRepositoryGroupId: 'child-group', kind: 'submodule', relativePath: 'child' },
          { parentRepositoryGroupId: 'child-group', childRepositoryGroupId: 'group-id', kind: 'submodule', relativePath: 'parent' },
        ]);
      }
      if (command === 'open_submodule_repository') {
        return Promise.resolve(opened);
      }
      return Promise.resolve(undefined);
    });
    TestBed.configureTestingModule({ providers: [{ provide: DESKTOP_IPC, useValue: { invoke } }] });
    const service = TestBed.inject(RepositoryCatalog);

    await service.load();
    const totalVisibleNodes = (groups: readonly RepositoryCatalogGroup[]): number =>
      groups.reduce((total, group) => total + 1 + totalVisibleNodes(group.submodules), 0);
    expect(totalVisibleNodes(service.groups())).toBeLessThanOrEqual(2);

    await service.openSubmodule('repository-id', 'child');
    expect(invoke).toHaveBeenCalledWith('open_submodule_repository', {
      parentRepositoryId: 'repository-id',
      path: 'child',
    });
  });
});
