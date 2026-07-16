import { Injectable, inject, signal } from '@angular/core';

import { GITHUB_BRIDGE, type GitHubAccount } from './github-bridge';

export type GitHubAccountState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly accounts: readonly GitHubAccount[] }
  | { readonly kind: 'error'; readonly message: string };

@Injectable()
export class GitHubAccountStore {
  private readonly bridge = inject(GITHUB_BRIDGE);
  private loadGeneration = 0;
  private mutationGeneration = 0;

  readonly state = signal<GitHubAccountState>({ kind: 'idle' });
  readonly connecting = signal(false);
  readonly disconnectingAccountId = signal<string | null>(null);
  readonly mutationError = signal('');

  async load(): Promise<void> {
    if (this.connecting() || this.disconnectingAccountId() !== null) {
      return;
    }
    const generation = ++this.loadGeneration;
    this.state.set({ kind: 'loading' });
    try {
      const accounts = await this.bridge.githubListAccounts();
      if (generation === this.loadGeneration) {
        this.state.set({ kind: 'ready', accounts });
      }
    } catch (error) {
      if (generation === this.loadGeneration) {
        this.state.set({ kind: 'error', message: errorMessage(error, 'GitHub accounts could not be loaded.') });
      }
    }
  }

  async connectPat(token: string): Promise<boolean> {
    const normalized = token.trim();
    if (normalized.length === 0 || this.connecting() || this.disconnectingAccountId() !== null) {
      return false;
    }
    ++this.loadGeneration;
    const generation = ++this.mutationGeneration;
    this.connecting.set(true);
    this.mutationError.set('');
    try {
      const account = await this.bridge.githubConnectPat({ token: normalized });
      if (generation !== this.mutationGeneration) {
        return false;
      }
      const current = this.state();
      const accounts = current.kind === 'ready' ? current.accounts : [];
      this.state.set({
        kind: 'ready',
        accounts: [account, ...accounts.filter((candidate) => candidate.id !== account.id)],
      });
      return true;
    } catch (error) {
      if (generation === this.mutationGeneration) {
        this.mutationError.set(errorMessage(error, 'The GitHub account could not be connected.'));
      }
      return false;
    } finally {
      if (generation === this.mutationGeneration) {
        this.connecting.set(false);
      }
    }
  }

  async disconnect(accountId: string): Promise<void> {
    if (accountId.length === 0 || this.connecting() || this.disconnectingAccountId() !== null) {
      return;
    }
    ++this.loadGeneration;
    const generation = ++this.mutationGeneration;
    this.disconnectingAccountId.set(accountId);
    this.mutationError.set('');
    try {
      const result = await this.bridge.githubDisconnectAccount({ accountId });
      if (generation === this.mutationGeneration && result.disconnected) {
        const current = this.state();
        this.state.set({
          kind: 'ready',
          accounts: current.kind === 'ready'
            ? current.accounts.filter((account) => account.id !== accountId)
            : [],
        });
      }
    } catch (error) {
      if (generation === this.mutationGeneration) {
        this.mutationError.set(errorMessage(error, 'The GitHub account could not be disconnected.'));
      }
    } finally {
      if (generation === this.mutationGeneration) {
        this.disconnectingAccountId.set(null);
      }
    }
  }

  invalidate(): void {
    ++this.loadGeneration;
    ++this.mutationGeneration;
    this.connecting.set(false);
    this.disconnectingAccountId.set(null);
  }
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  if (typeof error === 'object' && error !== null && 'message' in error) {
    const message = (error as { readonly message?: unknown }).message;
    if (typeof message === 'string' && message.trim().length > 0) {
      return message;
    }
  }
  return fallback;
}
