import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, RouterLink } from '@angular/router';

import { AiSupportStore } from '../../core/ai-support/ai-support.store';
import {
  DESKTOP_IPC,
  type AiCliProvider,
  type AiCodeReviewDocument,
  type AiCodeReviewSummary,
  type AiTaskReviewPreflightRequest,
  type AiTaskReviewPreflightResponse,
  type RepositoryBranch,
  type RepositoryStatusResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { readReleaseBranch } from '../workspace-history/release-branch-state';
import { SafeMarkdown } from '../github/pull-request-inspector/safe-markdown/safe-markdown';

@Component({
  selector: 'app-code-review-dashboard',
  imports: [RouterLink, SafeMarkdown],
  templateUrl: './code-review-dashboard.html',
  styleUrl: './code-review-dashboard.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CodeReviewDashboard {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly route = inject(ActivatedRoute);
  protected readonly catalog = inject(RepositoryCatalog);
  protected readonly aiSupport = inject(AiSupportStore);

  protected readonly repositoryId = this.route.snapshot.queryParamMap.get('repositoryId') ?? '';
  protected readonly repository = computed(() => this.catalog.find(this.repositoryId));
  protected readonly reviews = signal<readonly AiCodeReviewSummary[]>([]);
  protected readonly selectedReview = signal<AiCodeReviewDocument | null>(null);
  protected readonly listLoading = signal(true);
  protected readonly reviewLoading = signal(false);
  protected readonly workspaceLoading = signal(this.repositoryId.length > 0);
  protected readonly preflightLoading = signal(false);
  protected readonly generating = signal(false);
  protected readonly merging = signal(false);
  protected readonly error = signal('');
  protected readonly status = signal<RepositoryStatusResponse | null>(null);
  protected readonly localBranches = signal<readonly RepositoryBranch[]>([]);
  protected readonly currentBranch = signal<RepositoryBranch | null>(null);
  protected readonly selectedTargetFullName = signal('');
  protected readonly selectedProvider = signal<AiCliProvider | null>(null);
  protected readonly preflight = signal<AiTaskReviewPreflightResponse | null>(null);
  /** Fingerprint of the request that produced `preflight`. */
  protected readonly preflightFingerprint = signal<string | null>(null);
  protected readonly availableProviders = this.aiSupport.enabledAvailableProviders;
  protected readonly selectedTarget = computed(() =>
    this.localBranches().find(({ fullName }) => fullName === this.selectedTargetFullName()) ?? null,
  );
  protected readonly currentReviewFingerprint = computed(() => {
    const request = this.reviewRequest();
    return request === null ? null : reviewRequestFingerprint(request);
  });
  protected readonly canAnalyze = computed(() => {
    const request = this.reviewRequest();
    const preflight = this.preflight();
    return !this.generating()
      && !this.merging()
      && this.selectedProvider() !== null
      && request !== null
      && this.preflightFingerprint() === reviewRequestFingerprint(request)
      && preflightMatchesRequest(preflight, request);
  });

  private preflightGeneration = 0;
  private preflightInFlight = 0;
  private reviewSelectionGeneration = 0;

  constructor() {
    void this.initialize();
  }

  protected async selectReview(review: AiCodeReviewSummary): Promise<void> {
    if (this.reviewLoading()) return;
    const generation = ++this.reviewSelectionGeneration;
    this.reviewLoading.set(true);
    this.error.set('');
    try {
      const document = await this.ipc.invoke('code_review_read', { id: review.id });
      if (generation === this.reviewSelectionGeneration) {
        this.selectedReview.set(document);
      }
    } catch (error) {
      if (generation === this.reviewSelectionGeneration) {
        this.error.set(messageFrom(error, 'The code review could not be opened.'));
      }
    } finally {
      if (generation === this.reviewSelectionGeneration) {
        this.reviewLoading.set(false);
      }
    }
  }

  protected async selectTarget(fullName: string): Promise<void> {
    this.selectedTargetFullName.set(fullName);
    this.invalidatePreflight();
    await this.runPreflight();
  }

  protected selectProvider(provider: AiCliProvider): void {
    this.selectedProvider.set(provider);
  }

  protected async analyzeCurrentState(): Promise<void> {
    const provider = this.selectedProvider();
    const request = this.reviewRequest();
    if (provider === null || request === null || this.generating() || !this.hasCurrentPreflight(request)) return;
    const selectionGeneration = this.reviewSelectionGeneration;
    this.generating.set(true);
    this.error.set('');
    try {
      const document = await this.aiSupport.generateTaskReview(provider, request);
      this.reviews.update((reviews) => [document.summary, ...reviews.filter(({ id }) => id !== document.summary.id)]);
      if (selectionGeneration === this.reviewSelectionGeneration) {
        this.selectedReview.set(document);
      }
    } catch (error) {
      this.error.set(messageFrom(error, 'The AI task review failed.'));
    } finally {
      this.generating.set(false);
    }
  }

  protected async mergeTargetAndAnalyze(): Promise<void> {
    const source = this.selectedTarget();
    const target = this.currentBranch();
    const status = this.status();
    const request = this.reviewRequest();
    if (source === null
      || target === null
      || status === null
      || request === null
      || this.merging()
      || !this.hasCurrentPreflight(request)
      || this.preflight()?.targetMerged !== false) return;
    this.merging.set(true);
    this.error.set('');
    try {
      const result = await this.ipc.invoke('repository_merge_branch', {
        repositoryId: this.repositoryId,
        operation: {
          sourceFullName: source.fullName,
          expectedSourceOid: source.oid,
          targetFullName: target.fullName,
          expectedTargetOid: target.oid,
          autoStash: status.entries.length > 0
            ? { message: `WIP ${new Date().toISOString().slice(0, 19)} ${target.name}` }
            : null,
        },
      });
      if (result.state !== 'succeeded') {
        this.error.set(result.errorMessage ?? (result.state === 'conflicted'
          ? 'The target merge stopped with conflicts. Resolve them in Workspace before reviewing.'
          : 'The target branch could not be merged.'));
        return;
      }
      await this.loadWorkspace();
      if (this.preflight()?.targetMerged === true) {
        await this.analyzeCurrentState();
      }
    } catch (error) {
      this.error.set(messageFrom(error, 'The target branch could not be merged.'));
    } finally {
      this.merging.set(false);
    }
  }

  protected formatDate(timestamp: number): string {
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(timestamp));
  }

  protected providerLabel(provider: AiCliProvider): string {
    return provider === 'claude' ? 'Claude' : provider === 'cursor' ? 'Cursor' : 'Codex';
  }

  private async initialize(): Promise<void> {
    await Promise.all([this.catalog.load(), this.aiSupport.loadAvailability(), this.loadReviews()]);
    this.selectedProvider.set(this.availableProviders()[0] ?? null);
    if (this.repositoryId.length > 0) {
      await this.loadWorkspace();
    }
  }

  private async loadReviews(): Promise<void> {
    this.listLoading.set(true);
    try {
      const response = await this.ipc.invoke('code_review_list', {});
      this.reviews.set(response.reviews);
      if (response.reviews[0]) await this.selectReview(response.reviews[0]);
    } catch (error) {
      this.error.set(messageFrom(error, 'Saved code reviews could not be loaded.'));
    } finally {
      this.listLoading.set(false);
    }
  }

  private async loadWorkspace(): Promise<void> {
    this.invalidatePreflight();
    const repository = this.repository();
    if (repository === undefined) {
      this.workspaceLoading.set(false);
      this.error.set('The selected repository is no longer available.');
      return;
    }
    this.workspaceLoading.set(true);
    try {
      const [status, navigation] = await Promise.all([
        this.ipc.invoke('repository_status', { repositoryPath: repository.path }),
        this.ipc.invoke('repository_navigation', { repositoryId: this.repositoryId }),
      ]);
      const local = navigation.branches.filter(({ kind }) => kind === 'local');
      const current = local.find(({ current }) => current)
        ?? local.find(({ name }) => name === status.branch.head)
        ?? null;
      this.status.set(status);
      this.localBranches.set(local);
      this.currentBranch.set(current);
      const available = new Set(local.map(({ fullName }) => fullName));
      const requested = this.route.snapshot.queryParamMap.get('target');
      const release = readReleaseBranch(globalThis.localStorage, this.repositoryId, available);
      const target = [release, requested, 'refs/heads/main', 'refs/heads/master']
        .filter((candidate): candidate is string => candidate !== null)
        .map((fullName) => local.find((branch) => branch.fullName === fullName))
        .find((branch) => branch !== undefined && branch.fullName !== current?.fullName)
        ?? local.find((branch) => branch.fullName !== current?.fullName)
        ?? null;
      this.selectedTargetFullName.set(target?.fullName ?? '');
      await this.runPreflight();
    } catch (error) {
      this.error.set(messageFrom(error, 'The repository could not be prepared for review.'));
    } finally {
      this.workspaceLoading.set(false);
    }
  }

  private async runPreflight(): Promise<void> {
    const request = this.reviewRequest();
    if (request === null) return;
    const fingerprint = reviewRequestFingerprint(request);
    const generation = ++this.preflightGeneration;
    this.preflight.set(null);
    this.preflightFingerprint.set(null);
    this.preflightInFlight += 1;
    this.preflightLoading.set(true);
    this.error.set('');
    try {
      const response = await this.aiSupport.taskReviewPreflight(request);
      // Loading a different target or refreshing the workspace must never let a
      // late response make the new context actionable.
      if (generation !== this.preflightGeneration || fingerprint !== this.currentReviewFingerprint()) return;
      if (!preflightMatchesRequest(response, request)) {
        this.error.set('The repository changed while the task review was being prepared. Please retry.');
        return;
      }
      this.preflight.set(response);
      this.preflightFingerprint.set(fingerprint);
    } catch (error) {
      if (generation === this.preflightGeneration && fingerprint === this.currentReviewFingerprint()) {
        this.error.set(messageFrom(error, 'The task review preflight failed.'));
      }
    } finally {
      this.preflightInFlight -= 1;
      this.preflightLoading.set(this.preflightInFlight > 0);
    }
  }

  private invalidatePreflight(): void {
    this.preflightGeneration += 1;
    this.preflight.set(null);
    this.preflightFingerprint.set(null);
  }

  private hasCurrentPreflight(request: AiTaskReviewPreflightRequest): boolean {
    return this.preflightFingerprint() === reviewRequestFingerprint(request)
      && preflightMatchesRequest(this.preflight(), request);
  }

  private reviewRequest(): AiTaskReviewPreflightRequest | null {
    const status = this.status();
    const target = this.selectedTarget();
    if (status?.branch.oid === null || status?.branch.oid === undefined || target === null) return null;
    return {
      repositoryId: this.repositoryId,
      targetFullName: target.fullName,
      targetOid: target.oid,
      expectedHead: status.branch.oid,
      indexFingerprint: status.indexFingerprint,
      worktreeFingerprint: status.worktreeFingerprint,
    };
  }
}

function reviewRequestFingerprint(request: AiTaskReviewPreflightRequest): string {
  return [
    request.repositoryId,
    request.targetFullName,
    request.targetOid,
    request.expectedHead,
    request.indexFingerprint,
    request.worktreeFingerprint,
  ].join('\u0000');
}

function preflightMatchesRequest(
  response: AiTaskReviewPreflightResponse | null,
  request: AiTaskReviewPreflightRequest,
): response is AiTaskReviewPreflightResponse {
  return response !== null
    && response.targetFullName === request.targetFullName
    && response.targetOid === request.targetOid
    && response.head === request.expectedHead
    && response.indexFingerprint === request.indexFingerprint
    && response.worktreeFingerprint === request.worktreeFingerprint;
}

function messageFrom(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string') return error.message;
  return fallback;
}
