import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';

import { Commander } from '../../core/commander/commander';
import { SkibiBot } from '../skibi-bot/skibi-bot';

@Component({
  selector: 'app-commander-panel',
  imports: [SkibiBot],
  templateUrl: './commander-panel.html',
  styleUrl: './commander-panel.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CommanderPanel {
  protected readonly commander = inject(Commander);
  protected readonly draft = signal('');

  protected async submit(): Promise<void> {
    const message = this.draft();
    if (message.trim().length === 0) return;
    this.draft.set('');
    await this.commander.send(message);
  }
}
