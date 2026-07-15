import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RememberedRepositoryResponse,
} from '../ipc/desktop-ipc';
import { RepositoryCatalog } from './repository-catalog';

const rememberedRepository: RememberedRepositoryResponse = {
  id: 'repository-id',
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
    expect(service.find('repository-id')).toMatchObject({
      provider: 'github',
      transport: 'ssh',
      integration: 'connected',
    });
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
});
