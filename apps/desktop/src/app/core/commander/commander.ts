import { computed, inject, Injectable, signal } from '@angular/core';
import { Router } from '@angular/router';

import { AiSupportStore } from '../ai-support/ai-support.store';
import {
  DESKTOP_IPC,
  type AiCliProvider,
  type AiCommanderAction,
  type RepositoryCommitDetailResponse,
  type RepositoryCompareRefsResponse,
  type RepositoryHistoryResponse,
  type RepositoryNavigationResponse,
  type RepositoryStatusResponse,
} from '../ipc/desktop-ipc';
import { RepositoryCatalog } from '../repositories/repository-catalog';
import { CommanderContextStore } from './commander-context';

export interface CommanderMessage {
  readonly id: number;
  readonly role: 'user' | 'assistant';
  readonly text: string;
}

@Injectable({ providedIn: 'root' })
export class Commander {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly router = inject(Router);
  private readonly aiSupport = inject(AiSupportStore);
  private readonly context = inject(CommanderContextStore);
  private readonly catalog = inject(RepositoryCatalog);
  private nextMessageId = 1;

  readonly open = signal(false);
  readonly loading = signal(false);
  readonly error = signal('');
  readonly messages = signal<readonly CommanderMessage[]>([
    { id: 0, role: 'assistant', text: 'Hi, I’m Skibi-Bot. Ask me to navigate the app or explain what you want to do.' },
  ]);
  readonly availableProviders = computed(() =>
    this.aiSupport.providerStatuses().filter(({ available }) => available),
  );
  readonly available = computed(() => this.availableProviders().length > 0);

  constructor() {
    void this.aiSupport.loadAvailability();
  }

  toggle(): void {
    if (this.available()) this.open.update((open) => !open);
  }

  close(): void {
    this.open.set(false);
  }

  async send(rawMessage: string): Promise<void> {
    const message = rawMessage.trim();
    const provider = this.selectedProvider();
    if (message.length === 0 || provider === null || this.loading()) return;
    if (new TextEncoder().encode(message).byteLength > 8192) {
      this.error.set('Your message is too long. Shorten it and try again.');
      return;
    }
    const history = this.messages()
      .filter(({ id }) => id !== 0)
      .slice(-8)
      .map(({ role, text }) => ({ role, text: truncateUtf8(text, 4000) }));
    this.messages.update((messages) => [
      ...messages,
      { id: this.nextMessageId++, role: 'user', text: message },
    ]);
    this.loading.set(true);
    this.error.set('');
    try {
      const response = await this.ipc.invoke('ai_commander_turn', {
        provider,
        message,
        history,
        context: this.context.context(),
      });
      this.messages.update((messages) => [
        ...messages,
        { id: this.nextMessageId++, role: 'assistant', text: response.message },
      ]);
      for (const action of response.actions) {
        const result = await this.execute(action);
        if (result !== null) {
          this.messages.update((messages) => [
            ...messages,
            { id: this.nextMessageId++, role: 'assistant', text: result },
          ]);
        }
      }
    } catch (error) {
      this.error.set(error instanceof Error ? error.message : 'Skibi-Bot could not complete the request.');
    } finally {
      this.loading.set(false);
    }
  }

  private async execute(action: AiCommanderAction): Promise<string | null> {
    if (action.type === 'navigate') {
      await this.router.navigateByUrl(action.route);
      return null;
    }
    const repositoryId = this.context.context().repositoryId;
    if (repositoryId === null) {
      return 'Open a repository first so I can inspect it.';
    }
    if (action.type === 'repositoryStatus') {
      const repository = this.catalog.find(repositoryId);
      if (repository === undefined) {
        return 'The current repository is no longer available in the local catalog.';
      }
      const status = await this.ipc.invoke('repository_status', {
        repositoryPath: repository.path,
      });
      return formatRepositoryStatus(status);
    }
    if (action.type === 'recentCommits') {
      const history = await this.ipc.invoke('repository_history', {
        repositoryId,
        cursor: null,
        limit: Math.max(1, Math.min(20, action.limit)),
      });
      return formatRecentCommits(history);
    }
    if (action.type === 'fileHistory') {
      const repository = this.catalog.find(repositoryId);
      if (repository === undefined) {
        return 'The current repository is no longer available in the local catalog.';
      }
      const status = await this.ipc.invoke('repository_status', {
        repositoryPath: repository.path,
      });
      if (status.branch.oid === null) {
        return 'File history is unavailable because the repository does not have a current commit.';
      }
      const history = await this.ipc.invoke('repository_file_history', {
        repositoryId,
        startOid: status.branch.oid,
        path: action.path,
        cursor: null,
      });
      return formatFileHistory(action.path, history);
    }
    if (action.type === 'compareRefs') {
      const navigation = await this.ipc.invoke('repository_navigation', { repositoryId });
      const source = resolveBranch(navigation, action.source);
      const target = resolveBranch(navigation, action.target);
      if (source === null || target === null) {
        return 'One of those refs is unavailable or ambiguous. Use an exact local or remote branch name.';
      }
      const comparison = await this.ipc.invoke('repository_compare_refs', {
        repositoryId,
        sourceFullName: source.fullName,
        expectedSourceOid: source.oid,
        targetFullName: target.fullName,
        expectedTargetOid: target.oid,
      });
      return formatRefComparison(comparison);
    }
    const detail = await this.ipc.invoke('repository_commit_detail', {
      repositoryId,
      oid: action.oid,
    });
    return formatCommitDetail(detail);
  }

  private selectedProvider(): AiCliProvider | null {
    return this.aiSupport.enabledAvailableProviders()[0]
      ?? this.availableProviders()[0]?.provider
      ?? null;
  }
}

function resolveBranch(
  navigation: RepositoryNavigationResponse,
  requested: string,
): RepositoryNavigationResponse['branches'][number] | null {
  const matches = navigation.branches.filter(
    (branch) => branch.fullName === requested || branch.name === requested,
  );
  return matches.length === 1 ? matches[0] : null;
}

function formatRefComparison(comparison: RepositoryCompareRefsResponse): string {
  const commits = comparison.commits
    .slice(0, 20)
    .map((commit) => `${commit.oid.slice(0, 8)}  ${commit.summary}`)
    .join('\n');
  return [
    `${comparison.sourceFullName} compared with ${comparison.targetFullName}`,
    `Ahead: ${comparison.ahead} · Behind: ${comparison.behind}`,
    `Changed files: ${comparison.files.length}${comparison.filesTruncated ? '+' : ''}`,
    commits.length > 0 ? `Source-only commits\n${commits}` : 'No source-only commits.',
  ].join('\n');
}

function formatFileHistory(
  path: string,
  history: RepositoryHistoryResponse,
): string {
  if (history.commits.length === 0) return `No commits changing ${path} were found.`;
  return `Commits changing ${path}\n${history.commits
    .slice(0, 20)
    .map((commit) => `${commit.oid.slice(0, 8)}  ${commit.summary} — ${commit.author.name}`)
    .join('\n')}`;
}

function formatRepositoryStatus(status: RepositoryStatusResponse): string {
  const branch = status.branch.head ?? (status.branch.detached ? 'detached HEAD' : 'unborn branch');
  const upstream = status.branch.upstream === null
    ? 'no upstream'
    : `${status.branch.upstream} (↑${status.branch.ahead} ↓${status.branch.behind})`;
  return `Repository status\nBranch: ${branch}\nUpstream: ${upstream}\nChanged entries: ${status.entries.length}`;
}

function truncateUtf8(value: string, maxBytes: number): string {
  const encoder = new TextEncoder();
  if (encoder.encode(value).byteLength <= maxBytes) return value;
  let low = 0;
  let high = value.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (encoder.encode(value.slice(0, middle)).byteLength <= maxBytes) {
      low = middle;
    } else {
      high = middle - 1;
    }
  }
  return value.slice(0, low);
}

function formatRecentCommits(history: RepositoryHistoryResponse): string {
  if (history.commits.length === 0) return 'No commits were found in the current repository.';
  return `Recent commits\n${history.commits
    .map((commit) => `${commit.oid.slice(0, 8)}  ${commit.summary} — ${commit.author.name}`)
    .join('\n')}`;
}

function formatCommitDetail(detail: RepositoryCommitDetailResponse): string {
  const files = detail.files
    .slice(0, 30)
    .map((file) => `${file.status}: ${file.path}`)
    .join('\n');
  const truncated = detail.files.length > 30 ? `\n… and ${detail.files.length - 30} more files` : '';
  return `Commit ${detail.oid.slice(0, 8)}\n${detail.fullMessage}\n\nChanged files (${detail.files.length})\n${files}${truncated}`;
}
