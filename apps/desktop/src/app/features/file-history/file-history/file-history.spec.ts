import { ActivatedRoute, convertToParamMap } from '@angular/router';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RepositoryFileBlameResponse,
  type RepositoryFileHistoryResponse,
} from '../../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../../core/repositories/repository-catalog';
import { FileHistory } from './file-history';

const oid = 'abcdefabcdefabcdefabcdefabcdefabcdefabcd';
const history: RepositoryFileHistoryResponse = {
  startOid: oid,
  path: 'src/app/feature.ts',
  commits: [{ oid, parents: [], author: { name: 'Ada', email: 'ada@example.com', authoredAt: '2026-07-23T09:00:00Z' }, summary: 'Add feature history', refs: [] }],
  nextCursor: 'next-page',
};
const blame: RepositoryFileBlameResponse = {
  oid,
  path: 'src/app/feature.ts',
  state: 'available',
  truncated: false,
  lines: [{ lineNumber: 1, oid, originalLineNumber: 1, finalLineNumber: 1, authorName: 'Ada', authorEmail: 'ada@example.com', authoredAt: '2026-07-23T09:00:00Z', summary: 'Add feature history', content: 'export const feature = true;' }],
};

interface FileHistoryTestApi {
  historyState(): { readonly kind: string };
  blameState(): { readonly kind: string };
  selectTab(tab: 'history' | 'blame'): void;
  loadMore(): Promise<void>;
}

function api(component: FileHistory): FileHistoryTestApi {
  return component as unknown as FileHistoryTestApi;
}

describe('FileHistory', () => {
  let fixture: ComponentFixture<FileHistory>;
  let component: FileHistory;
  let invoke: ReturnType<typeof vi.fn>;

  async function configure(
    query: Record<string, string> = { oid, path: history.path },
    expectedState: 'ready' | 'error' = 'ready',
  ): Promise<void> {
    invoke = vi.fn((command: string, request: { readonly cursor?: string | null }) => {
      if (command === 'repository_file_history') {
        return Promise.resolve(request.cursor === 'next-page'
          ? { ...history, commits: [], nextCursor: null }
          : history);
      }
      if (command === 'repository_file_blame') return Promise.resolve(blame);
      return Promise.resolve(undefined);
    });
    await TestBed.configureTestingModule({
      imports: [FileHistory],
      providers: [
        { provide: DESKTOP_IPC, useValue: { invoke: invoke as unknown as DesktopIpcClient['invoke'] } },
        { provide: RepositoryCatalog, useValue: { load: vi.fn().mockResolvedValue(undefined), find: vi.fn().mockReturnValue({ id: 'repo-1', name: 'Demo repository' }) } },
        { provide: ActivatedRoute, useValue: { snapshot: { paramMap: convertToParamMap({ repositoryId: 'repo-1' }), queryParamMap: convertToParamMap(query) } } },
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(FileHistory);
    component = fixture.componentInstance;
    fixture.detectChanges();
    await vi.waitFor(() => expect(api(component).historyState().kind).toBe(expectedState));
    fixture.detectChanges();
  }

  it('reads history from the exact query OID and literal path', async () => {
    await configure();

    expect(invoke).toHaveBeenCalledWith('repository_file_history', {
      repositoryId: 'repo-1', startOid: oid, path: 'src/app/feature.ts', cursor: null,
    });
    expect(fixture.nativeElement.textContent).toContain('Add feature history');
  });

  it('does not call native IPC for malformed snapshot parameters', async () => {
    await configure({ oid: 'main', path: '' }, 'error');

    expect(invoke).not.toHaveBeenCalledWith('repository_file_history', expect.anything());
    expect(fixture.nativeElement.textContent).toContain('Open file history from a file');
  });

  it('loads blame only after selecting its tab', async () => {
    await configure();
    expect(invoke).not.toHaveBeenCalledWith('repository_file_blame', expect.anything());

    api(component).selectTab('blame');
    await fixture.whenStable();
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_file_blame', {
      repositoryId: 'repo-1', oid, path: 'src/app/feature.ts',
    });
    expect(fixture.nativeElement.textContent).toContain('export const feature = true;');
  });

  it('keeps file history paginated and bounded by the opaque cursor', async () => {
    await configure();
    await api(component).loadMore();

    expect(invoke).toHaveBeenLastCalledWith('repository_file_history', {
      repositoryId: 'repo-1', startOid: oid, path: 'src/app/feature.ts', cursor: 'next-page',
    });
  });

  it('shows an explicit unavailable state for binary files', async () => {
    invoke = vi.fn((command: string) => {
      if (command === 'repository_file_history') return Promise.resolve(history);
      if (command === 'repository_file_blame') return Promise.resolve({ ...blame, state: 'binary', lines: [] });
      return Promise.resolve(undefined);
    });
    await TestBed.configureTestingModule({
      imports: [FileHistory],
      providers: [
        { provide: DESKTOP_IPC, useValue: { invoke: invoke as unknown as DesktopIpcClient['invoke'] } },
        { provide: RepositoryCatalog, useValue: { load: vi.fn().mockResolvedValue(undefined), find: vi.fn() } },
        { provide: ActivatedRoute, useValue: { snapshot: { paramMap: convertToParamMap({ repositoryId: 'repo-1' }), queryParamMap: convertToParamMap({ oid, path: history.path }) } } },
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(FileHistory);
    component = fixture.componentInstance;
    fixture.detectChanges();
    await vi.waitFor(() => expect(api(component).historyState().kind).not.toBe('loading'));
    api(component).selectTab('blame');
    await vi.waitFor(() => expect(api(component).blameState().kind).toBe('ready'));
    fixture.detectChanges();

    expect(fixture.nativeElement.textContent).toContain('This file is binary.');
  });
});
