import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import { ApplicationUpdate } from '../../../core/application-update/application-update';
import { SafeMarkdown } from '../../../features/github/pull-request-inspector/safe-markdown/safe-markdown';

@Component({
  selector: 'app-application-update-prompt',
  imports: [SafeMarkdown],
  templateUrl: './application-update-prompt.html',
  styleUrl: './application-update-prompt.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ApplicationUpdatePrompt {
  protected readonly updater = inject(ApplicationUpdate);
}
