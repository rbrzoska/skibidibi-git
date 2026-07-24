import { signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AiSupportStore } from '../../core/ai-support/ai-support.store';
import {
  DESKTOP_IPC,
  type AiCodeReviewSummary,
  type AiTaskReviewPreflightResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { writeReleaseBranch } from '../workspace-history/release-branch-state';
import { CodeReviewDashboard } from './code-review-dashboard';

describe('CodeReviewDashboard', () => {
  let fixture: ComponentFixture<CodeReviewDashboard>;
  const invoke = vi.fn();
  const generateTaskReview = vi.fn();
  const preflight: AiTaskReviewPreflightResponse = {
    branch: 'feature/task', head: '1'.repeat(40), targetFullName: 'refs/heads/release/2026.7',
    targetOid: '2'.repeat(40), mergeBase: '3'.repeat(40), targetMerged: false,
    uncommittedFiles: 2, changedFiles: 5, myCommits: 3,
    indexFingerprint: 'index', worktreeFingerprint: 'worktree',
  };
  const aiSupport = {
    enabledAvailableProviders: signal(['codex' as const]),
    loadAvailability: vi.fn().mockResolvedValue(undefined),
    taskReviewPreflight: vi.fn().mockResolvedValue(preflight),
    generateTaskReview,
  };
  const catalog = {
    load: vi.fn().mockResolvedValue(undefined),
    find: vi.fn().mockReturnValue({ id: 'repo-1', name: 'project', path: '/repo/project' }),
  };

  beforeEach(async () => {
    globalThis.localStorage.clear();
    writeReleaseBranch(globalThis.localStorage, 'repo-1', 'refs/heads/release/2026.7');
    invoke.mockReset().mockImplementation((command: string) => {
      if (command === 'code_review_list') return Promise.resolve({ reviews: [] });
      if (command === 'repository_status') {
        return Promise.resolve({
          branch: { oid: '1'.repeat(40), head: 'feature/task', upstream: null, ahead: 0, behind: 0, detached: false, unborn: false },
          entries: [{ kind: 'ordinary', path: 'src/app.ts', originalPath: null, indexStatus: 'modified', worktreeStatus: 'modified', submodule: null }],
          indexFingerprint: 'index', worktreeFingerprint: 'worktree',
        });
      }
      if (command === 'repository_navigation') {
        return Promise.resolve({
          branches: [
            { kind: 'local', fullName: 'refs/heads/feature/task', name: 'feature/task', oid: '1'.repeat(40), current: true, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'local', fullName: 'refs/heads/release/2026.7', name: 'release/2026.7', oid: '2'.repeat(40), current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
            { kind: 'local', fullName: 'refs/heads/main', name: 'main', oid: '4'.repeat(40), current: false, upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null },
          ],
          worktrees: [], stashes: [],
        });
      }
      if (command === 'repository_merge_branch') return Promise.resolve({ state: 'succeeded', errorMessage: null });
      throw new Error(`Unexpected command ${command}`);
    });
    generateTaskReview.mockReset().mockResolvedValue({
      summary: {
        id: 'review-1', repositoryId: 'repo-1', repositoryName: 'project', branch: 'feature/task',
        targetBranch: 'refs/heads/release/2026.7', provider: 'codex', createdAtMs: 1,
        changedFiles: 5, myCommits: 3, uncommittedFiles: 2, markdownFile: '/data/review-1.md',
      },
      markdown: '# Review\n\nNo actionable issues.',
    });
    aiSupport.taskReviewPreflight.mockClear().mockResolvedValue(preflight);
    await TestBed.configureTestingModule({
      imports: [CodeReviewDashboard],
      providers: [
        provideRouter([]),
        { provide: ActivatedRoute, useValue: { snapshot: { queryParamMap: convertToParamMap({ repositoryId: 'repo-1', target: 'refs/heads/main' }) } } },
        { provide: DESKTOP_IPC, useValue: { invoke } },
        { provide: RepositoryCatalog, useValue: catalog },
        { provide: AiSupportStore, useValue: aiSupport },
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(CodeReviewDashboard);
    await fixture.whenStable();
    fixture.detectChanges();
  });

  it('always prefers the remembered release branch over a target passed in the URL', () => {
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(text).toContain('Committed branch diff + staged, unstaged and untracked files');
    expect(text).toContain('Target branch is not fully merged');
    expect(text).toContain('5 changed files');
    expect(text).toContain('3 my commits');
    expect(aiSupport.taskReviewPreflight).toHaveBeenCalledWith(expect.objectContaining({
      targetFullName: 'refs/heads/release/2026.7', targetOid: '2'.repeat(40),
    }));
  });

  it('generates and displays a persisted Markdown review without merging', async () => {
    const button = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('button')]
      .find((candidate) => candidate.textContent?.includes('Analyze without merge'));
    button?.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(generateTaskReview).toHaveBeenCalledWith('codex', expect.objectContaining({ repositoryId: 'repo-1' }));
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('No actionable issues.');
  });

  it('keeps only the preflight response for the target selected most recently', async () => {
    const component = fixture.componentInstance as unknown as {
      selectTarget(fullName: string): Promise<void>;
      preflight(): AiTaskReviewPreflightResponse | null;
      canAnalyze(): boolean;
    };
    const first = deferred<AiTaskReviewPreflightResponse>();
    const second = deferred<AiTaskReviewPreflightResponse>();
    aiSupport.taskReviewPreflight.mockReset()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const selectRelease = component.selectTarget('refs/heads/release/2026.7');
    const selectMain = component.selectTarget('refs/heads/main');
    second.resolve({ ...preflight, targetFullName: 'refs/heads/main', targetOid: '4'.repeat(40) });
    await selectMain;

    expect(component.preflight()).toEqual(expect.objectContaining({ targetFullName: 'refs/heads/main' }));
    expect(component.canAnalyze()).toBe(true);

    first.resolve(preflight);
    await selectRelease;

    expect(component.preflight()).toEqual(expect.objectContaining({ targetFullName: 'refs/heads/main' }));
    expect(component.canAnalyze()).toBe(true);
  });

  it('does not accept a preflight response after the repository head changes', async () => {
    const component = fixture.componentInstance as unknown as {
      selectTarget(fullName: string): Promise<void>;
      status: ReturnType<typeof signal>;
      currentBranch: ReturnType<typeof signal>;
      preflight(): AiTaskReviewPreflightResponse | null;
      canAnalyze(): boolean;
    };
    const oldRequest = deferred<AiTaskReviewPreflightResponse>();
    aiSupport.taskReviewPreflight.mockReset()
      .mockReturnValueOnce(oldRequest.promise);

    const oldPreflight = component.selectTarget('refs/heads/release/2026.7');
    component.status.set({
      branch: { oid: '9'.repeat(40), head: 'feature/task', upstream: null, ahead: 0, behind: 0, detached: false, unborn: false },
      entries: [], indexFingerprint: 'new-index', worktreeFingerprint: 'new-worktree',
    });
    component.currentBranch.set({
      kind: 'local', fullName: 'refs/heads/feature/task', name: 'feature/task', oid: '9'.repeat(40), current: true,
      upstream: null, ahead: 0, behind: 0, upstreamGone: false, symbolicTarget: null,
    });

    oldRequest.resolve(preflight);
    await oldPreflight;

    expect(component.preflight()).toBeNull();
    expect(component.canAnalyze()).toBe(false);
  });

  it('refuses a merge when its preflight no longer belongs to the current context', async () => {
    const component = fixture.componentInstance as unknown as {
      preflight: ReturnType<typeof signal>;
      preflightFingerprint: ReturnType<typeof signal>;
      mergeTargetAndAnalyze(): Promise<void>;
    };
    component.preflight.set(preflight);
    component.preflightFingerprint.set('stale-context');

    await component.mergeTargetAndAnalyze();

    expect(invoke).not.toHaveBeenCalledWith('repository_merge_branch', expect.anything());
  });

  it('does not replace a review selected while AI generation is still running', async () => {
    const component = fixture.componentInstance as unknown as {
      analyzeCurrentState(): Promise<void>;
      selectReview(review: AiCodeReviewSummary): Promise<void>;
      selectedReview(): { summary: AiCodeReviewSummary } | null;
    };
    const generated = deferred<Awaited<ReturnType<typeof aiSupport.generateTaskReview>>>();
    generateTaskReview.mockReset().mockReturnValueOnce(generated.promise);
    const analyze = component.analyzeCurrentState();
    await Promise.resolve();

    const savedSummary: AiCodeReviewSummary = {
      id: 'saved-review', repositoryId: 'repo-1', repositoryName: 'project', branch: 'feature/task',
      targetBranch: 'refs/heads/release/2026.7', provider: 'codex', createdAtMs: 2,
      changedFiles: 1, myCommits: 1, uncommittedFiles: 0, markdownFile: '/data/saved.md',
    };
    invoke.mockResolvedValueOnce({ summary: savedSummary, markdown: '# Saved review' });
    await component.selectReview(savedSummary);

    generated.resolve({
      summary: {
        id: 'generated-review', repositoryId: 'repo-1', repositoryName: 'project', branch: 'feature/task',
        targetBranch: 'refs/heads/release/2026.7', provider: 'codex', createdAtMs: 3,
        changedFiles: 5, myCommits: 3, uncommittedFiles: 2, markdownFile: '/data/generated.md',
      },
      markdown: '# Generated review',
    });
    await analyze;

    expect(component.selectedReview()?.summary.id).toBe('saved-review');
  });
});

function deferred<T>(): { promise: Promise<T>; resolve(value: T): void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}
