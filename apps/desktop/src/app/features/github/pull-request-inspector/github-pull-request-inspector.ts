import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import { GitHubRepositoryPullRequestStore } from '../../../core/github';
import { SafeMarkdown } from './safe-markdown/safe-markdown';

@Component({
  selector: 'app-github-pull-request-inspector',
  imports: [SafeMarkdown],
  templateUrl: './github-pull-request-inspector.html',
  styleUrl: './github-pull-request-inspector.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GitHubPullRequestInspector {
  protected readonly store = inject(GitHubRepositoryPullRequestStore);

  protected retry(number: number): void {
    void this.store.select(number);
  }
}
