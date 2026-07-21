import { Injectable, inject, signal } from '@angular/core';

import {
  GITHUB_BRIDGE,
  type GitHubAccount,
  type GitHubDeviceFlowStart,
} from './github-bridge';

export type GitHubAccountState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly accounts: readonly GitHubAccount[] }
  | { readonly kind: 'error'; readonly message: string };

export type GitHubDeviceFlowState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'starting' }
  | ({ readonly kind: 'waiting'; readonly nextPollAt: number } & GitHubDeviceFlowStart)
  | { readonly kind: 'expired' }
  | { readonly kind: 'denied' };

@Injectable()
export class GitHubAccountStore {
  private readonly bridge = inject(GITHUB_BRIDGE);
  private loadGeneration = 0;
  private mutationGeneration = 0;
  private deviceFlowTimer: ReturnType<typeof setTimeout> | null = null;

  readonly state = signal<GitHubAccountState>({ kind: 'idle' });
  readonly connecting = signal(false);
  readonly connectingAuthKind = signal<'personalAccessToken' | 'gitHubCli' | null>(null);
  readonly deviceFlow = signal<GitHubDeviceFlowState>({ kind: 'idle' });
  readonly disconnectingAccountId = signal<string | null>(null);
  readonly mutationError = signal('');

  async startDeviceFlow(): Promise<void> {
    if (this.connecting() || this.disconnectingAccountId() !== null || this.deviceFlow().kind === 'starting' || this.deviceFlow().kind === 'waiting') {
      return;
    }
    ++this.loadGeneration;
    const generation = ++this.mutationGeneration;
    this.clearDeviceFlowTimer();
    this.deviceFlow.set({ kind: 'starting' });
    this.mutationError.set('');
    try {
      const flow = await this.bridge.githubStartDeviceFlow();
      if (generation !== this.mutationGeneration) {
        void this.bridge.githubCancelDeviceFlow({ flowId: flow.flowId }).catch(() => undefined);
        return;
      }
      const nextPollAt = Date.now() + Math.max(1, flow.intervalSeconds) * 1_000;
      this.deviceFlow.set({ kind: 'waiting', ...flow, nextPollAt });
      this.scheduleDeviceFlowPoll(generation, nextPollAt);
    } catch (error) {
      if (generation === this.mutationGeneration) {
        this.deviceFlow.set({ kind: 'idle' });
        this.mutationError.set(errorMessage(error, 'GitHub sign-in could not be started.'));
      }
    }
  }

  async cancelDeviceFlow(): Promise<void> {
    const flow = this.deviceFlow();
    ++this.mutationGeneration;
    this.clearDeviceFlowTimer();
    this.deviceFlow.set({ kind: 'idle' });
    if (flow.kind !== 'waiting') {
      return;
    }
    try {
      await this.bridge.githubCancelDeviceFlow({ flowId: flow.flowId });
    } catch (error) {
      this.mutationError.set(errorMessage(error, 'GitHub sign-in could not be cancelled cleanly.'));
    }
  }

  async openDeviceVerification(): Promise<void> {
    const flow = this.deviceFlow();
    if (flow.kind !== 'waiting') {
      return;
    }
    try {
      await this.bridge.githubOpenDeviceVerification({ flowId: flow.flowId });
    } catch (error) {
      this.mutationError.set(errorMessage(error, 'The GitHub sign-in page could not be opened.'));
    }
  }

  async load(): Promise<void> {
    if (this.connecting() || this.disconnectingAccountId() !== null || this.deviceFlowActive()) {
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
    if (normalized.length === 0 || this.connecting() || this.disconnectingAccountId() !== null || this.deviceFlowActive()) {
      return false;
    }
    ++this.loadGeneration;
    const generation = ++this.mutationGeneration;
    this.connecting.set(true);
    this.connectingAuthKind.set('personalAccessToken');
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
        this.connectingAuthKind.set(null);
      }
    }
  }

  async connectCli(): Promise<boolean> {
    if (this.connecting() || this.disconnectingAccountId() !== null || this.deviceFlowActive()) {
      return false;
    }
    ++this.loadGeneration;
    const generation = ++this.mutationGeneration;
    this.connecting.set(true);
    this.connectingAuthKind.set('gitHubCli');
    this.mutationError.set('');
    try {
      const account = await this.bridge.githubConnectCli();
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
        this.mutationError.set(errorMessage(
          error,
          'GitHub CLI could not be detected. Install gh and run “gh auth login”, then try again.',
        ));
      }
      return false;
    } finally {
      if (generation === this.mutationGeneration) {
        this.connecting.set(false);
        this.connectingAuthKind.set(null);
      }
    }
  }

  async disconnect(accountId: string): Promise<void> {
    if (accountId.length === 0 || this.connecting() || this.disconnectingAccountId() !== null || this.deviceFlowActive()) {
      return;
    }
    const currentAccountState = this.state();
    if (
      currentAccountState.kind === 'ready' &&
      currentAccountState.accounts.some(
        (account) => account.id === accountId && account.authKind === 'gitHubCli',
      )
    ) {
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
    this.connectingAuthKind.set(null);
    this.disconnectingAccountId.set(null);
    this.clearDeviceFlowTimer();
    this.deviceFlow.set({ kind: 'idle' });
  }

  private scheduleDeviceFlowPoll(generation: number, nextPollAt: number): void {
    this.clearDeviceFlowTimer();
    this.deviceFlowTimer = setTimeout(() => {
      this.deviceFlowTimer = null;
      void this.pollDeviceFlow(generation);
    }, Math.max(0, nextPollAt - Date.now()));
  }

  private async pollDeviceFlow(generation: number): Promise<void> {
    const flow = this.deviceFlow();
    if (generation !== this.mutationGeneration || flow.kind !== 'waiting') {
      return;
    }
    if (Date.now() >= flow.expiresAt * 1_000) {
      this.deviceFlow.set({ kind: 'expired' });
      void this.bridge.githubCancelDeviceFlow({ flowId: flow.flowId }).catch(() => undefined);
      return;
    }
    try {
      const result = await this.bridge.githubPollDeviceFlow({ flowId: flow.flowId });
      if (generation !== this.mutationGeneration) {
        return;
      }
      if (result.state === 'authorized' && result.account !== null) {
        const current = this.state();
        const accounts = current.kind === 'ready' ? current.accounts : [];
        this.state.set({
          kind: 'ready',
          accounts: [result.account, ...accounts.filter((candidate) => candidate.id !== result.account?.id)],
        });
        this.deviceFlow.set({ kind: 'idle' });
        return;
      }
      if (result.state === 'authorized') {
        this.deviceFlow.set({ kind: 'idle' });
        this.mutationError.set('GitHub authorized the sign-in but returned no account. Try again.');
        return;
      }
      if (result.state === 'expired' || result.state === 'denied') {
        this.deviceFlow.set({ kind: result.state });
        return;
      }
      const fallbackNextPollAt = Date.now() + Math.max(1, flow.intervalSeconds) * 1_000;
      const nextPollAt = result.nextPollAt === null
        ? fallbackNextPollAt
        : Math.max(fallbackNextPollAt, result.nextPollAt * 1_000);
      const updated = { ...flow, nextPollAt };
      this.deviceFlow.set(updated);
      this.scheduleDeviceFlowPoll(generation, nextPollAt);
    } catch (error) {
      if (generation === this.mutationGeneration) {
        void this.bridge.githubCancelDeviceFlow({ flowId: flow.flowId }).catch(() => undefined);
        this.deviceFlow.set({ kind: 'idle' });
        this.mutationError.set(errorMessage(error, 'GitHub sign-in could not be completed.'));
      }
    }
  }

  private clearDeviceFlowTimer(): void {
    if (this.deviceFlowTimer !== null) {
      clearTimeout(this.deviceFlowTimer);
      this.deviceFlowTimer = null;
    }
  }

  private deviceFlowActive(): boolean {
    const state = this.deviceFlow();
    return state.kind === 'starting' || state.kind === 'waiting';
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
