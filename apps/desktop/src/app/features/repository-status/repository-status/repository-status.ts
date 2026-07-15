import { ChangeDetectionStrategy, Component, inject } from '@angular/core';

import { RepositoryStatusStore } from '../repository-status';

@Component({
  selector: 'app-repository-status',
  imports: [],
  templateUrl: './repository-status.html',
  styleUrl: './repository-status.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepositoryStatus {
  protected readonly statusStore = inject(RepositoryStatusStore);
}
