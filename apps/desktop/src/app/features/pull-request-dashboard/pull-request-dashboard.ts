import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';

import {
  GithubPullRequestDashboard,
  GitHubRepositoryPullRequestStore,
  type DashboardPullRequest,
} from '../../core/github';
import { CommanderContextStore } from '../../core/commander/commander-context';
import { GitHubPullRequestInspector } from '../github/pull-request-inspector/github-pull-request-inspector';

@Component({
  selector: 'app-pull-request-dashboard',
  imports: [GitHubPullRequestInspector],
  providers: [GitHubRepositoryPullRequestStore],
  templateUrl: './pull-request-dashboard.html',
  styleUrls: ['./pull-request-dashboard.css', './pull-request-dashboard-review.css'],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PullRequestDashboard {
  protected readonly dashboard = inject(GithubPullRequestDashboard);
  protected readonly detailStore = inject(GitHubRepositoryPullRequestStore);
  private readonly commanderContext = inject(CommanderContextStore);
  protected readonly detailTab = signal<'description' | 'files'>('description');

  constructor() {
    void this.dashboard.load();
  }

  protected refresh(): void {
    this.detailStore.clearSelection();
    this.commanderContext.select(null);
    void this.dashboard.load();
  }

  protected toggleReviewRequested(): void {
    this.dashboard.setReviewRequestedVisible(!this.dashboard.showReviewRequested());
  }

  protected toggleAuthored(): void {
    this.dashboard.setAuthoredVisible(!this.dashboard.showAuthored());
  }

  protected select(pullRequest: DashboardPullRequest): void {
    this.dashboard.select(pullRequest);
    this.commanderContext.select(
      `pull-request:${pullRequest.repositoryFullName}#${pullRequest.number}|title:${pullRequest.title}`,
    );
    this.detailTab.set('description');
    const account = this.dashboard.account();
    if (account === null) {
      return;
    }
    this.detailStore.configure(pullRequest.repositoryId, account.id);
    void this.detailStore.select(pullRequest.number);
  }

  protected showDescription(): void {
    this.detailTab.set('description');
  }

  protected showFiles(): void {
    this.detailTab.set('files');
    const selected = this.dashboard.selected();
    const files = this.detailStore.filesState();
    if (selected !== null && (files.kind === 'idle' || ('number' in files && files.number !== selected.number))) {
      void this.detailStore.loadFiles(selected.number);
    }
  }

  protected approve(): void {
    const selected = this.dashboard.selected();
    if (selected !== null) {
      void this.detailStore.approve(selected.number);
    }
  }

  protected patchLines(patch: string): readonly { readonly kind: 'hunk' | 'addition' | 'deletion' | 'context'; readonly text: string }[] {
    return patch.split('\n').map((text) => ({
      text,
      kind: text.startsWith('@@') ? 'hunk' : text.startsWith('+') ? 'addition' : text.startsWith('-') ? 'deletion' : 'context',
    }));
  }

  protected formatUpdatedAt(value: string): string {
    const timestamp = Date.parse(value);
    if (!Number.isFinite(timestamp)) {
      return value;
    }
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(timestamp);
  }
}
