import { TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import { DESKTOP_IPC, type DesktopIpcClient } from '../ipc/desktop-ipc';
import { AiSupportStore, DEFAULT_AI_COMMIT_PROMPT, DEFAULT_AI_REVIEW_PROMPT } from './ai-support.store';

describe('AiSupportStore', () => {
  const invoke = vi.fn<DesktopIpcClient['invoke']>();

  beforeEach(() => {
    globalThis.localStorage.clear();
    invoke.mockReset();
    invoke.mockImplementation((command) => {
      if (command === 'ai_cli_status') {
        return Promise.resolve({
          statuses: [
            { provider: 'codex', displayName: 'Codex', available: true, version: '1.0.0', detail: null },
            { provider: 'claude', displayName: 'Claude Code', available: false, version: null, detail: 'Not installed' },
            { provider: 'cursor', displayName: 'Cursor', available: true, version: '2.0.0', detail: null },
          ],
        });
      }
      return Promise.resolve({
        message: 'Add AI support settings',
        indexFingerprint: 'index',
        worktreeFingerprint: 'worktree',
      });
    });
    TestBed.configureTestingModule({
      providers: [{ provide: DESKTOP_IPC, useValue: { invoke } }],
    });
  });

  afterEach(() => globalThis.localStorage.clear());

  it('starts with the default one-sentence prompt and keeps detected providers disabled', async () => {
    const store = TestBed.inject(AiSupportStore);

    expect(store.promptTemplate()).toBe(DEFAULT_AI_COMMIT_PROMPT);
    expect(store.reviewPromptTemplate()).toBe(DEFAULT_AI_REVIEW_PROMPT);
    expect(store.enabledAvailableProviders()).toEqual([]);
    await store.loadAvailability();
    expect(store.enabledAvailableProviders()).toEqual([]);
  });

  it('persists an enabled available provider and the prompt template', async () => {
    const store = TestBed.inject(AiSupportStore);
    await store.loadAvailability();
    store.setProviderEnabled('codex', true);
    store.updatePromptTemplate('Create a concise English commit summary.');

    expect(store.enabledAvailableProviders()).toEqual(['codex']);
    expect(JSON.parse(globalThis.localStorage.getItem('skibidibi-git.ai-support.v1') ?? '{}')).toEqual({
      enabledProviders: ['codex'],
      promptTemplate: 'Create a concise English commit summary.',
      reviewPromptTemplate: DEFAULT_AI_REVIEW_PROMPT,
    });
  });

  it('persists, bounds, and resets the task-review prompt', () => {
    const store = TestBed.inject(AiSupportStore);
    store.updateReviewPromptTemplate('Review only correctness risks.');
    expect(store.reviewPromptTemplate()).toBe('Review only correctness risks.');

    store.updateReviewPromptTemplate('x'.repeat(17_000));
    expect(store.reviewPromptTemplate()).toHaveLength(16_384);

    store.resetReviewPromptTemplate();
    expect(store.reviewPromptTemplate()).toBe(DEFAULT_AI_REVIEW_PROMPT);
  });

  it('does not enable unavailable providers and removes providers which become unavailable', async () => {
    const store = TestBed.inject(AiSupportStore);
    store.setProviderEnabled('claude', true);
    expect(store.enabledAvailableProviders()).toEqual([]);

    await store.loadAvailability();
    store.setProviderEnabled('codex', true);
    expect(store.enabledAvailableProviders()).toEqual(['codex']);

    invoke.mockResolvedValueOnce({
      statuses: [
        { provider: 'codex', displayName: 'Codex', available: false, version: null, detail: 'Not installed' },
        { provider: 'claude', displayName: 'Claude Code', available: false, version: null, detail: 'Not installed' },
        { provider: 'cursor', displayName: 'Cursor', available: false, version: null, detail: 'Not installed' },
      ],
    });
    await store.loadAvailability();

    expect(store.enabledAvailableProviders()).toEqual([]);
  });

  it('resets blank or explicitly reset prompts to the safe default', () => {
    const store = TestBed.inject(AiSupportStore);
    store.updatePromptTemplate('   ');
    expect(store.promptTemplate()).toBe(DEFAULT_AI_COMMIT_PROMPT);

    store.updatePromptTemplate('Use the staged diff.');
    store.resetPromptTemplate();
    expect(store.promptTemplate()).toBe(DEFAULT_AI_COMMIT_PROMPT);

    store.updatePromptTemplate('x'.repeat(4_100));
    expect(store.promptTemplate()).toHaveLength(4_096);

    store.updatePromptTemplate('Write one sentence ');
    expect(store.promptTemplate()).toBe('Write one sentence ');
  });

  it('only sends a generation request for an enabled available provider', async () => {
    const store = TestBed.inject(AiSupportStore);
    await expect(store.generateCommitMessage('codex', {
      repositoryId: 'repo-1', expectedHead: 'head', indexFingerprint: 'index', worktreeFingerprint: 'worktree',
    })).rejects.toThrow('not enabled');

    await store.loadAvailability();
    store.setProviderEnabled('codex', true);
    await store.generateCommitMessage('codex', {
      repositoryId: 'repo-1', expectedHead: 'head', indexFingerprint: 'index', worktreeFingerprint: 'worktree',
    });

    expect(invoke).toHaveBeenLastCalledWith('ai_generate_commit_message', {
      repositoryId: 'repo-1',
      provider: 'codex',
      promptTemplate: DEFAULT_AI_COMMIT_PROMPT,
      expectedHead: 'head',
      indexFingerprint: 'index',
      worktreeFingerprint: 'worktree',
    });
  });

  it('runs task-review preflight and generation with the saved review prompt', async () => {
    const store = TestBed.inject(AiSupportStore);
    const request = {
      repositoryId: 'repo-1', targetFullName: 'refs/heads/main', targetOid: 'target',
      expectedHead: 'head', indexFingerprint: 'index', worktreeFingerprint: 'worktree',
    };

    await store.taskReviewPreflight(request);
    expect(invoke).toHaveBeenLastCalledWith('ai_task_review_preflight', request);

    await store.loadAvailability();
    store.setProviderEnabled('codex', true);
    store.updateReviewPromptTemplate('Focus on regressions.');
    await store.generateTaskReview('codex', request);
    expect(invoke).toHaveBeenLastCalledWith('ai_generate_task_review', {
      ...request,
      provider: 'codex',
      promptTemplate: 'Focus on regressions.',
    });
  });
});
