import { computed, inject, Injectable, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type AiCliProvider,
  type AiCliStatus,
  type GenerateAiCommitMessageResponse,
} from '../ipc/desktop-ipc';

const STORAGE_KEY = 'skibidibi-git.ai-support.v1';
const MAX_PROMPT_TEMPLATE_LENGTH = 4096;

export const DEFAULT_AI_COMMIT_PROMPT = 'Write one concise English sentence describing the staged changes as a commit message. Return only that single sentence with no body, bullets, prefixes, descriptions, signatures, or metadata.';

const PROVIDERS = [
  { provider: 'codex', displayName: 'Codex' },
  { provider: 'claude', displayName: 'Claude Code' },
  { provider: 'cursor', displayName: 'Cursor' },
] as const satisfies readonly { readonly provider: AiCliProvider; readonly displayName: string }[];

interface PersistedAiSupportSettings {
  readonly enabledProviders?: unknown;
  readonly promptTemplate?: unknown;
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

function restoreSettings(): { readonly enabledProviders: readonly AiCliProvider[]; readonly promptTemplate: string } {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (raw === null || raw === undefined) {
      return { enabledProviders: [], promptTemplate: DEFAULT_AI_COMMIT_PROMPT };
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
    };
  } catch {
    return { enabledProviders: [], promptTemplate: DEFAULT_AI_COMMIT_PROMPT };
  }
}

function normalizePromptTemplate(template: string): string {
  const bounded = template.slice(0, MAX_PROMPT_TEMPLATE_LENGTH);
  return bounded.trim().length > 0 ? bounded : DEFAULT_AI_COMMIT_PROMPT;
}

function messageFrom(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim().length > 0 ? error.message : fallback;
}
