import { ChangeDetectionStrategy, Component, computed, effect, inject, input, output } from '@angular/core';

import {
  GitHubAccountStore,
  GitHubRepositoryPullRequestStore,
  type GitHubPullRequestScope,
  type GitHubPullRequestSummary,
} from '../../../core/github';

@Component({
  selector: 'app-github-pull-request-list',
  imports: [],
  templateUrl: './github-pull-request-list.html',
  styleUrl: './github-pull-request-list.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GitHubPullRequestList {
  protected readonly store = inject(GitHubRepositoryPullRequestStore);
  protected readonly accounts = inject(GitHubAccountStore);
  protected readonly accountBusy = computed(() => {
    const flow = this.accounts.deviceFlow();
    return this.accounts.connecting() ||
      this.accounts.disconnectingAccountId() !== null ||
      flow.kind === 'starting' ||
      flow.kind === 'waiting';
  });

  readonly repositoryId = input.required<string>();
  readonly accountId = input<string | null>(null);
  readonly pullRequestSelected = output<number>();

  constructor() {
    effect(() => {
      if (this.store.configure(this.repositoryId(), this.accountId())) {
        void this.store.load();
      }
    });
  }

  protected select(pullRequest: GitHubPullRequestSummary): void {
    this.pullRequestSelected.emit(pullRequest.number);
    void this.store.select(pullRequest.number);
  }

  protected loadMore(): void {
    void this.store.loadMore();
  }

  protected retry(): void {
    void this.store.load();
  }

  protected setScope(scope: GitHubPullRequestScope): void {
    if (this.store.setScope(scope)) {
      void this.store.load();
    }
  }

  protected async refreshGitHubCli(): Promise<void> {
    if (this.accountBusy()) {
      return;
    }
    const connected = await this.accounts.connectCli();
    if (!connected) {
      return;
    }
    const state = this.accounts.state();
    if (state.kind !== 'ready') {
      return;
    }
    const currentAccount = state.accounts.find((account) => account.id === this.accountId());
    const cliAccount = state.accounts.find(
      (account) =>
        account.authKind === 'gitHubCli' &&
        account.state === 'connected' &&
        (currentAccount === undefined || account.host.toLowerCase() === currentAccount.host.toLowerCase()),
    );
    if (cliAccount === undefined) {
      return;
    }
    this.store.configure(this.repositoryId(), cliAccount.id);
    await this.store.load();
  }
}
