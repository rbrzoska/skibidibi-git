import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';

import { GitHubAccountStore } from '../../../core/github';

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

  constructor() {
    if (this.store.state().kind === 'idle') {
      void this.store.load();
    }
  }

  protected updateToken(value: string): void {
    this.token.set(value);
  }

  protected async connect(): Promise<void> {
    const token = this.token();
    this.token.set('');
    await this.store.connectPat(token);
  }

  protected disconnect(accountId: string): void {
    void this.store.disconnect(accountId);
  }
}
