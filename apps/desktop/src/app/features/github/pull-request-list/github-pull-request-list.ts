import { ChangeDetectionStrategy, Component, effect, inject, input, output } from '@angular/core';

import {
  GitHubRepositoryPullRequestStore,
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
}
