import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';

import { GitHubAccountStore, type GitHubAccount } from '../../../core/github';

@Component({
  selector: 'app-github-account-control',
  imports: [],
  templateUrl: './github-account-control.html',
  styleUrl: './github-account-control.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GitHubAccountControl {
  protected readonly store = inject(GitHubAccountStore);
  protected readonly token = signal('');
  protected readonly codeCopied = signal(false);
  protected readonly oauthBusy = computed(() => {
    const state = this.store.deviceFlow();
    return state.kind === 'starting' || state.kind === 'waiting';
  });
  protected readonly cliConnected = computed(() => {
    const state = this.store.state();
    return state.kind === 'ready' && state.accounts.some(
      (account) => account.authKind === 'gitHubCli' && account.state === 'connected',
    );
  });

  constructor() {
    if (this.store.state().kind === 'idle') {
      void this.store.load();
    }
  }

  protected updateToken(value: string): void {
    this.token.set(value);
  }

  protected connectWithGitHub(): void {
    this.codeCopied.set(false);
    void this.store.startDeviceFlow();
  }

  protected cancelGitHubConnection(): void {
    this.codeCopied.set(false);
    void this.store.cancelDeviceFlow();
  }

  protected openGitHubVerification(): void {
    void this.store.openDeviceVerification();
  }

  protected async copyUserCode(code: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(code);
      this.codeCopied.set(true);
    } catch {
      this.codeCopied.set(false);
    }
  }

  protected async connectPat(): Promise<void> {
    const token = this.token();
    this.token.set('');
    await this.store.connectPat(token);
  }

  protected connectCli(): void {
    void this.store.connectCli();
  }

  protected authKindLabel(authKind: GitHubAccount['authKind']): string {
    switch (authKind) {
      case 'gitHubCli':
        return 'GitHub CLI';
      case 'oAuthDevice':
        return 'OAuth';
      case 'personalAccessToken':
        return 'Token';
    }
  }

  protected disconnect(accountId: string): void {
    void this.store.disconnect(accountId);
  }
}
