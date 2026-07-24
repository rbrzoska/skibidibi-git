import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import {
  GitHubRepositoryPullRequestStore,
  type GitHubPullRequestComment,
} from '../../../core/github';
import { SafeMarkdown } from './safe-markdown/safe-markdown';
import { parseReviewDiffHunk } from './review-diff-hunk';

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

  protected threadDiffHunk(comments: readonly GitHubPullRequestComment[]): string | null {
    return comments.find((comment) => comment.diffHunk?.trim())?.diffHunk ?? null;
  }

  protected readonly reviewDiffRows = parseReviewDiffHunk;
}
