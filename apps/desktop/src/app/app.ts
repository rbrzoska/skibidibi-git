import { ChangeDetectionStrategy, Component } from '@angular/core';

import { RepositoryStatus } from './features/repository-status/repository-status/repository-status';

@Component({
  selector: 'app-root',
  imports: [RepositoryStatus],
  templateUrl: './app.html',
  styleUrl: './app.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {}
