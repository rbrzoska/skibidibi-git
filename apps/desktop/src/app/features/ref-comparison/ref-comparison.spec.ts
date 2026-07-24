import { ActivatedRoute, convertToParamMap } from '@angular/router';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RepositoryBranch,
  type RepositoryCompareRefsResponse,
  type RepositoryNavigationResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { releaseBranchStorageKey } from '../workspace-history/release-branch-state';
import { RefComparison } from './ref-comparison';

const navigation: RepositoryNavigationResponse = {
  branches: [
    { kind: 'local', fullName: 'refs/heads/feature', name: 'feature', oid: 'feature-oid', current: true, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
    { kind: 'local', fullName: 'refs/heads/release', name: 'release', oid: 'release-oid', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
    { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: 'main-oid', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
    { kind: 'remote', fullName: 'refs/remotes/origin/feature', name: 'origin/feature', oid: 'origin-feature-oid', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
    { kind: 'remote', fullName: 'refs/remotes/origin/HEAD', name: 'origin/HEAD', oid: 'main-oid', current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: 'refs/remotes/origin/main' },
  ],
  worktrees: [],
  stashes: [],
};

const comparison: RepositoryCompareRefsResponse = {
  sourceFullName: 'refs/heads/feature',
  sourceOid: 'feature-oid',
  targetFullName: 'refs/heads/main',
  targetOid: 'main-oid',
  mergeBaseOid: 'base-oid',
  ahead: 2,
  behind: 1,
  commits: [{ oid: 'feature-commit', parents: ['base-oid'], author: { name: 'Ada', email: 'ada@example.com', authoredAt: '2026-07-22T10:00:00Z' }, summary: 'Add comparison', refs: [] }],
  commitsTruncated: false,
  files: [{ path: 'src/compare.ts', oldPath: null, status: 'modified', additions: 1, deletions: 1, binary: false }],
  filesTruncated: false,
};

interface RefComparisonTestApi {
  sourceFullName(): string;
  targetFullName(): string;
  references(): readonly RepositoryBranch[];
  comparisonState(): { readonly kind: string };
  compare(): Promise<void>;
  selectTarget(fullName: string): void;
  swapReferences(): void;
  selectTab(tab: 'commits' | 'files'): void;
  openFileDiff(file: RepositoryCompareRefsResponse['files'][number]): Promise<void>;
}

function testApi(component: RefComparison): RefComparisonTestApi {
  return component as unknown as RefComparisonTestApi;
}

describe('RefComparison', () => {
  let fixture: ComponentFixture<RefComparison>;
  let component: RefComparison;
  let invoke: ReturnType<typeof vi.fn>;

  async function configure(query: Record<string, string> = {}): Promise<void> {
    invoke = vi.fn((command: string) => {
      if (command === 'repository_navigation') return Promise.resolve(navigation);
      if (command === 'repository_compare_refs') return Promise.resolve(comparison);
      if (command === 'repository_compare_ref_file_diff') {
        return Promise.resolve({
          sourceFullName: 'refs/heads/feature', sourceOid: 'feature-oid',
          targetFullName: 'refs/heads/main', targetOid: 'main-oid',
          path: 'src/compare.ts', oldPath: null, binary: false, truncated: false,
          patch: 'diff --git a/src/compare.ts b/src/compare.ts\n--- a/src/compare.ts\n+++ b/src/compare.ts\n@@ -1 +1 @@\n-old\n+new\n',
        });
      }
      return Promise.resolve(undefined);
    });
    await TestBed.configureTestingModule({
      imports: [RefComparison],
      providers: [
        { provide: DESKTOP_IPC, useValue: { invoke: invoke as unknown as DesktopIpcClient['invoke'] } },
        { provide: RepositoryCatalog, useValue: { load: vi.fn().mockResolvedValue(undefined), find: vi.fn().mockReturnValue({ id: 'repo-1', name: 'Demo repository', path: '/repo' }) } },
        { provide: ActivatedRoute, useValue: { snapshot: { paramMap: convertToParamMap({ repositoryId: 'repo-1' }), queryParamMap: convertToParamMap(query) } } },
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(RefComparison);
    component = fixture.componentInstance;
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
  }

  beforeEach(() => {
    globalThis.localStorage.clear();
  });

  it('defaults to the active local branch and a remembered release branch', async () => {
    globalThis.localStorage.setItem(releaseBranchStorageKey('repo-1'), 'refs/heads/release');
    await configure();

    expect(testApi(component).sourceFullName()).toBe('refs/heads/feature');
    expect(testApi(component).targetFullName()).toBe('refs/heads/release');
    expect(testApi(component).references().map((branch) => branch.fullName))
      .not.toContain('refs/remotes/origin/HEAD');
  });

  it('prefers the target reference from the URL over the remembered release branch', async () => {
    globalThis.localStorage.setItem(releaseBranchStorageKey('repo-1'), 'refs/heads/release');
    await configure({ target: 'refs/heads/main' });

    expect(testApi(component).targetFullName()).toBe('refs/heads/main');
  });

  it('does not publish a stale comparison after the selected ref changes', async () => {
    let resolveComparison: ((value: RepositoryCompareRefsResponse) => void) | undefined;
    await configure();
    invoke.mockImplementation((command: string) => {
      if (command === 'repository_compare_refs') {
        return new Promise<RepositoryCompareRefsResponse>((resolve) => { resolveComparison = resolve; });
      }
      return Promise.resolve(command === 'repository_navigation' ? navigation : undefined);
    });

    const pending = testApi(component).compare();
    testApi(component).selectTarget('refs/heads/main');
    resolveComparison?.(comparison);
    await pending;
    fixture.detectChanges();

    expect(testApi(component).comparisonState().kind).toBe('idle');
    expect(fixture.nativeElement.textContent).not.toContain('Changes source would introduce into target');
  });

  it('swaps source and target and invalidates the previous result', async () => {
    await configure();
    await testApi(component).compare();
    testApi(component).swapReferences();

    expect(testApi(component).sourceFullName()).toBe('refs/heads/main');
    expect(testApi(component).targetFullName()).toBe('refs/heads/feature');
    expect(testApi(component).comparisonState().kind).toBe('idle');
  });

  it('loads a bounded file diff using the selected immutable ref identities', async () => {
    await configure();
    await testApi(component).compare();
    testApi(component).selectTab('files');
    await testApi(component).openFileDiff(comparison.files[0]);
    fixture.detectChanges();

    expect(invoke).toHaveBeenCalledWith('repository_compare_ref_file_diff', {
      repositoryId: 'repo-1',
      sourceFullName: 'refs/heads/feature', expectedSourceOid: 'feature-oid',
      targetFullName: 'refs/heads/main', expectedTargetOid: 'main-oid',
      path: 'src/compare.ts', oldPath: null,
    });
    expect(fixture.nativeElement.querySelector('.diff-table')?.textContent).toContain('+new');
  });
});
