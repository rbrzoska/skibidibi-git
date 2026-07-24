import { computed, inject, Injectable, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type AiCliProvider,
  type AiCliStatus,
  type AiTaskReviewPreflightRequest,
  type AiTaskReviewPreflightResponse,
  type AiCodeReviewDocument,
  type GenerateAiCommitMessageResponse,
} from '../ipc/desktop-ipc';

const STORAGE_KEY = 'skibidibi-git.ai-support.v1';
const MAX_PROMPT_TEMPLATE_LENGTH = 4096;

export const DEFAULT_AI_COMMIT_PROMPT = 'Write one concise English sentence describing the staged changes as a commit message. Return only that single sentence with no body, bullets, prefixes, descriptions, signatures, or metadata.';
export const DEFAULT_AI_REVIEW_PROMPT = 'Review the current task diff as a senior engineer. Find actionable correctness, security, data-loss, performance, maintainability, and test-coverage problems. Produce concise Markdown with a verdict, findings ranked by severity, file references, and residual testing risks. Do not praise the implementation or restate the diff.';

const PROVIDERS = [
  { provider: 'codex', displayName: 'Codex' },
  { provider: 'claude', displayName: 'Claude Code' },
  { provider: 'cursor', displayName: 'Cursor' },
] as const satisfies readonly { readonly provider: AiCliProvider; readonly displayName: string }[];

interface PersistedAiSupportSettings {
  readonly enabledProviders?: unknown;
  readonly promptTemplate?: unknown;
  readonly reviewPromptTemplate?: unknown;
}

export interface GenerateCommitMessageInput {
  readonly repositoryId: string;
  readonly expectedHead: string | null;
  readonly indexFingerprint: string;
  readonly worktreeFingerprint: string;
}

@Injectable({ providedIn: 'root' })
export class AiSupportStore {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly restored = restoreSettings();
  private readonly enabledProviders = signal<readonly AiCliProvider[]>(this.restored.enabledProviders);

  readonly promptTemplate = signal(this.restored.promptTemplate);
  readonly reviewPromptTemplate = signal(this.restored.reviewPromptTemplate);
  readonly providerStatuses = signal<readonly AiCliStatus[]>(defaultStatuses());
  readonly availabilityLoading = signal(false);
  readonly availabilityError = signal<string | null>(null);
  readonly enabledAvailableProviders = computed(() => {
    const enabled = new Set(this.enabledProviders());
    return this.providerStatuses()
      .filter((status) => status.available && enabled.has(status.provider))
      .map((status) => status.provider);
  });

  async loadAvailability(): Promise<void> {
    if (this.availabilityLoading()) {
      return;
    }
    this.availabilityLoading.set(true);
    this.availabilityError.set(null);
    try {
      const response = await this.ipc.invoke('ai_cli_status', {});
      const statuses = normalizeStatuses(response.statuses);
      this.providerStatuses.set(statuses);
    } catch (error) {
      this.providerStatuses.set(defaultStatuses());
      this.availabilityError.set(messageFrom(error, 'AI CLI availability could not be checked.'));
    } finally {
      this.availabilityLoading.set(false);
    }
  }

  isProviderEnabled(provider: AiCliProvider): boolean {
    return this.enabledAvailableProviders().includes(provider);
  }

  setProviderEnabled(provider: AiCliProvider, enabled: boolean): void {
    const status = this.providerStatuses().find((candidate) => candidate.provider === provider);
    if (enabled && status?.available !== true) {
      return;
    }

    const next = enabled
      ? [...new Set([...this.enabledProviders(), provider])]
      : this.enabledProviders().filter((candidate) => candidate !== provider);
    this.enabledProviders.set(next);
    this.persist();
  }

  updatePromptTemplate(template: string): void {
    const normalized = normalizePromptTemplate(template);
    this.promptTemplate.set(normalized);
    this.persist();
  }

  resetPromptTemplate(): void {
    this.promptTemplate.set(DEFAULT_AI_COMMIT_PROMPT);
    this.persist();
  }

  updateReviewPromptTemplate(template: string): void {
    this.reviewPromptTemplate.set(normalizeReviewPromptTemplate(template));
    this.persist();
  }

  resetReviewPromptTemplate(): void {
    this.reviewPromptTemplate.set(DEFAULT_AI_REVIEW_PROMPT);
    this.persist();
  }

  taskReviewPreflight(input: AiTaskReviewPreflightRequest): Promise<AiTaskReviewPreflightResponse> {
    return this.ipc.invoke('ai_task_review_preflight', input);
  }

  generateTaskReview(
    provider: AiCliProvider,
    input: AiTaskReviewPreflightRequest,
  ): Promise<AiCodeReviewDocument> {
    if (!this.isProviderEnabled(provider)) {
      return Promise.reject(new Error(`${provider} is not enabled for AI task review.`));
    }
    return this.ipc.invoke('ai_generate_task_review', {
      ...input,
      provider,
      promptTemplate: this.reviewPromptTemplate(),
    });
  }

  generateCommitMessage(
    provider: AiCliProvider,
    input: GenerateCommitMessageInput,
  ): Promise<GenerateAiCommitMessageResponse> {
    if (!this.isProviderEnabled(provider)) {
      return Promise.reject(new Error(`${provider} is not enabled for AI commit-message generation.`));
    }
    return this.ipc.invoke('ai_generate_commit_message', {
      ...input,
      provider,
      promptTemplate: this.promptTemplate(),
    });
  }

  private persist(): void {
    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify({
        enabledProviders: this.enabledProviders(),
        promptTemplate: this.promptTemplate(),
        reviewPromptTemplate: this.reviewPromptTemplate(),
      }));
    } catch {
      // Settings continue to apply for this session when storage is unavailable.
    }
  }
}

function defaultStatuses(): readonly AiCliStatus[] {
  return PROVIDERS.map(({ provider, displayName }) => ({
    provider,
    displayName,
    available: false,
    version: null,
    detail: null,
  }));
}

function normalizeStatuses(statuses: readonly AiCliStatus[]): readonly AiCliStatus[] {
  return PROVIDERS.map(({ provider, displayName }) => {
    const status = statuses.find((candidate) => candidate.provider === provider);
    return {
      provider,
      displayName: typeof status?.displayName === 'string' && status.displayName.trim().length > 0
        ? status.displayName
        : displayName,
      available: status?.available === true,
      version: typeof status?.version === 'string' ? status.version : null,
      detail: typeof status?.detail === 'string' ? status.detail : null,
    };
  });
}

function restoreSettings(): { readonly enabledProviders: readonly AiCliProvider[]; readonly promptTemplate: string; readonly reviewPromptTemplate: string } {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (raw === null || raw === undefined) {
      return { enabledProviders: [], promptTemplate: DEFAULT_AI_COMMIT_PROMPT, reviewPromptTemplate: DEFAULT_AI_REVIEW_PROMPT };
    }
    const parsed = JSON.parse(raw) as PersistedAiSupportSettings;
    const storedProviders: readonly unknown[] = Array.isArray(parsed.enabledProviders)
      ? parsed.enabledProviders
      : [];
    const enabledProviders = PROVIDERS.map(({ provider }) => provider)
      .filter((provider) => storedProviders.includes(provider));
    return {
      enabledProviders,
      promptTemplate: typeof parsed.promptTemplate === 'string'
        ? normalizePromptTemplate(parsed.promptTemplate)
        : DEFAULT_AI_COMMIT_PROMPT,
      reviewPromptTemplate: typeof parsed.reviewPromptTemplate === 'string'
        ? normalizeReviewPromptTemplate(parsed.reviewPromptTemplate)
        : DEFAULT_AI_REVIEW_PROMPT,
    };
  } catch {
    return { enabledProviders: [], promptTemplate: DEFAULT_AI_COMMIT_PROMPT, reviewPromptTemplate: DEFAULT_AI_REVIEW_PROMPT };
  }
}

function normalizeReviewPromptTemplate(template: string): string {
  const bounded = template.slice(0, 16_384);
  return bounded.trim().length > 0 ? bounded : DEFAULT_AI_REVIEW_PROMPT;
}

function normalizePromptTemplate(template: string): string {
  const bounded = template.slice(0, MAX_PROMPT_TEMPLATE_LENGTH);
  return bounded.trim().length > 0 ? bounded : DEFAULT_AI_COMMIT_PROMPT;
}

function messageFrom(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim().length > 0 ? error.message : fallback;
}
