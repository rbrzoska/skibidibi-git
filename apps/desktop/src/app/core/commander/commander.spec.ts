import { TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';

import { DESKTOP_IPC, type DesktopIpcClient } from '../ipc/desktop-ipc';
import { RepositoryCatalog } from '../repositories/repository-catalog';
import { CommanderContextStore } from './commander-context';
import { Commander } from './commander';

describe('Commander', () => {
  let service: Commander;
  let invoke: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invoke = vi.fn().mockImplementation((command: string) => {
      if (command === 'ai_cli_status') {
        return Promise.resolve({
          statuses: [{ provider: 'codex', displayName: 'Codex', available: true, version: '1', detail: null }],
        });
      }
      if (command === 'ai_commander_turn') {
        return Promise.resolve({
          message: 'Opening repositories.',
          actions: [{ type: 'navigate', route: '/repositories' }],
        });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    const ipc = { invoke } as unknown as DesktopIpcClient;
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        { provide: DESKTOP_IPC, useValue: ipc },
        { provide: RepositoryCatalog, useValue: { find: vi.fn() } },
      ],
    });
    service = TestBed.inject(Commander);
  });

  it('should be created', () => {
    expect(service).toBeTruthy();
  });

  it('executes a validated navigation action returned by the desktop bridge', async () => {
    const router = TestBed.inject(Router);
    const navigate = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true);
    await vi.waitFor(() => expect(service.available()).toBe(true));

    await service.send('Open repositories');

    expect(navigate).toHaveBeenCalledWith('/repositories');
    expect(service.messages().at(-1)?.text).toBe('Opening repositories.');
  });

  it('executes bounded read-only repository actions and renders their result', async () => {
    TestBed.inject(RepositoryCatalog).find = vi.fn().mockReturnValue({ path: '/work/repo' });
    invoke.mockImplementation((command: string) => {
      if (command === 'ai_cli_status') {
        return Promise.resolve({
          statuses: [{ provider: 'codex', displayName: 'Codex', available: true, version: '1', detail: null }],
        });
      }
      if (command === 'ai_commander_turn') {
        return Promise.resolve({
          message: 'I’ll inspect the current repository.',
          actions: [{ type: 'repositoryStatus' }],
        });
      }
      if (command === 'repository_status') {
        return Promise.resolve({
          branch: {
            oid: 'abc', head: 'feature/task', upstream: 'origin/feature/task',
            ahead: 2, behind: 1, detached: false, unborn: false,
          },
          entries: [{}, {}, {}],
          indexFingerprint: 'index',
          worktreeFingerprint: 'worktree',
        });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    TestBed.inject(CommanderContextStore).updateRoute('/workspace/repo-1/history');
    await vi.waitFor(() => expect(service.available()).toBe(true));

    await service.send('What is the current status?');

    expect(invoke).toHaveBeenCalledWith('repository_status', { repositoryPath: '/work/repo' });
    expect(service.messages().at(-1)?.text).toContain('Branch: feature/task');
    expect(service.messages().at(-1)?.text).toContain('Changed entries: 3');
  });

  it('finds commits that changed an exact repository-relative file', async () => {
    TestBed.inject(RepositoryCatalog).find = vi.fn().mockReturnValue({ path: '/work/repo' });
    invoke.mockImplementation((command: string) => {
      if (command === 'ai_cli_status') {
        return Promise.resolve({
          statuses: [{ provider: 'codex', displayName: 'Codex', available: true, version: '1', detail: null }],
        });
      }
      if (command === 'ai_commander_turn') {
        return Promise.resolve({
          message: 'I’ll inspect that file history.',
          actions: [{ type: 'fileHistory', path: 'src/app.ts' }],
        });
      }
      if (command === 'repository_status') {
        return Promise.resolve({
          branch: {
            oid: 'a'.repeat(40), head: 'feature/task', upstream: null,
            ahead: 0, behind: 0, detached: false, unborn: false,
          },
          entries: [],
          indexFingerprint: 'index',
          worktreeFingerprint: 'worktree',
        });
      }
      if (command === 'repository_file_history') {
        return Promise.resolve({
          startOid: 'a'.repeat(40),
          path: 'src/app.ts',
          commits: [{
            oid: 'b'.repeat(40),
            parents: [],
            author: { name: 'Ada', email: 'ada@example.test', authoredAt: '2026-07-24T00:00:00Z' },
            summary: 'Update the app',
            refs: [],
          }],
          nextCursor: null,
        });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    TestBed.inject(CommanderContextStore).updateRoute('/workspace/repo-1/history');
    await vi.waitFor(() => expect(service.available()).toBe(true));

    await service.send('Find commits that changed src/app.ts');

    expect(invoke).toHaveBeenCalledWith('repository_file_history', {
      repositoryId: 'repo-1',
      startOid: 'a'.repeat(40),
      path: 'src/app.ts',
      cursor: null,
    });
    expect(service.messages().at(-1)?.text).toContain('bbbbbbbb  Update the app — Ada');
  });
});
