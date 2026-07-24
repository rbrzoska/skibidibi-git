import { ChangeDetectionStrategy, Component, input } from '@angular/core';

export type SkibiBotMood = 'happy' | 'neutral' | 'sad' | 'scared';

@Component({
  selector: 'app-skibi-bot',
  imports: [],
  templateUrl: './skibi-bot.html',
  styleUrl: './skibi-bot.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SkibiBot {
  readonly mood = input<SkibiBotMood>('happy');
}
