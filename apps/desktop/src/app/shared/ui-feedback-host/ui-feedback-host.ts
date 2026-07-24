import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import { UiFeedback, type UiFeedbackKind } from '../../core/ui-feedback/ui-feedback';
import { SkibiBot, type SkibiBotMood } from '../skibi-bot/skibi-bot';

@Component({
  selector: 'app-ui-feedback-host',
  imports: [SkibiBot],
  templateUrl: './ui-feedback-host.html',
  styleUrl: './ui-feedback-host.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class UiFeedbackHost {
  protected readonly feedback = inject(UiFeedback);

  protected mood(kind: UiFeedbackKind): SkibiBotMood {
    if (kind === 'success') return 'happy';
    if (kind === 'error') return 'scared';
    return 'neutral';
  }
}
