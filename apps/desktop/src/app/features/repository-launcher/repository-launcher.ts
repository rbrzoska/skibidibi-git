import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { Router } from '@angular/router';

import {
  RepositoryCatalog,
  type RepositoryCatalogEntry,
} from '../../core/repositories/repository-catalog';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { RepositoryStatus } from '../repository-status/repository-status/repository-status';
import { CloneRepositoryDialog } from './clone-repository-dialog/clone-repository-dialog';

@Component({
  selector: 'app-repository-launcher',
  imports: [CloneRepositoryDialog, RepositoryStatus],
  templateUrl: './repository-launcher.html',
  styleUrl: './repository-launcher.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepositoryLauncher {
  private readonly router = inject(Router);
  protected readonly catalog = inject(RepositoryCatalog);
  protected readonly statusStore = inject(RepositoryStatusStore);

  constructor() {
    void this.catalog.load();
  }

  protected async openRepository(repository: RepositoryCatalogEntry): Promise<void> {
    const remembered = await this.catalog.rememberPath(repository.path);
    this.statusStore.setRepositoryPath(remembered.path);
    await this.router.navigate(['/workspace', remembered.id, 'history']);
  }

  protected async openSelectedRepository(): Promise<void> {
    const path = this.statusStore.repositoryPath();
    if (path.length === 0) {
      return;
    }
    const repository = await this.catalog.rememberPath(path);
    this.statusStore.setRepositoryPath(repository.path);
    await this.router.navigate(['/workspace', repository.id, 'history']);
  }

  protected async openClonedRepository(repository: RepositoryCatalogEntry): Promise<void> {
    this.statusStore.setRepositoryPath(repository.path);
    await this.router.navigate(['/workspace', repository.id, 'history']);
  }
}
