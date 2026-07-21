import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, inject, signal, viewChild } from '@angular/core';
import { Router } from '@angular/router';

import { DESKTOP_IPC } from '../../core/ipc/desktop-ipc';
import {
  RepositoryCatalog,
  type RepositoryCatalogEntry,
  type RepositoryCatalogGroup,
} from '../../core/repositories/repository-catalog';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { CloneRepositoryDialog } from './clone-repository-dialog/clone-repository-dialog';

@Component({
  selector: 'app-repository-launcher',
  imports: [CloneRepositoryDialog, NgTemplateOutlet],
  templateUrl: './repository-launcher.html',
  styleUrl: './repository-launcher.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepositoryLauncher {
  private readonly router = inject(Router);
  private readonly ipc = inject(DESKTOP_IPC);
  protected readonly catalog = inject(RepositoryCatalog);
  protected readonly statusStore = inject(RepositoryStatusStore);
  protected readonly cloneDialog = viewChild(CloneRepositoryDialog);
  private readonly collapsedGroups = signal<ReadonlySet<string>>(new Set());
  protected readonly localActionBusy = signal(false);
  protected readonly localActionError = signal('');

  constructor() {
    void this.catalog.load();
  }

  protected async openRepository(repository: RepositoryCatalogEntry): Promise<void> {
    const remembered = await this.catalog.rememberPath(repository.path);
    this.statusStore.setRepositoryPath(remembered.path);
    await this.router.navigate(['/workspace', remembered.id, 'history']);
  }

  protected async openGroup(group: RepositoryCatalogGroup): Promise<void> {
    if (group.defaultRepository !== null) {
      await this.openRepository(group.defaultRepository);
    }
  }

  protected isExpanded(group: RepositoryCatalogGroup): boolean {
    return (group.grouped || group.submodules.length > 0) && !this.collapsedGroups().has(group.id);
  }

  protected toggleGroup(group: RepositoryCatalogGroup): void {
    if (!group.grouped && group.submodules.length === 0) {
      return;
    }
    this.collapsedGroups.update((collapsed) => {
      const next = new Set(collapsed);
      if (next.has(group.id)) {
        next.delete(group.id);
      } else {
        next.add(group.id);
      }
      return next;
    });
  }

  protected handleDisclosureKeydown(event: KeyboardEvent, group: RepositoryCatalogGroup): void {
    if (event.key === 'ArrowRight' && !this.isExpanded(group)) {
      event.preventDefault();
      this.toggleGroup(group);
    } else if (event.key === 'ArrowLeft' && this.isExpanded(group)) {
      event.preventDefault();
      this.toggleGroup(group);
    }
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

  protected openCloneDialog(): void {
    this.cloneDialog()?.open();
  }

  protected async selectLocalRepository(openAfterSelection: boolean): Promise<void> {
    if (this.localActionBusy()) {
      return;
    }
    this.localActionBusy.set(true);
    this.localActionError.set('');
    try {
      const selection = await this.ipc.invoke('select_repository_directory', {
        initialPath: this.statusStore.repositoryPath() || null,
      });
      if (selection.path === null) {
        return;
      }
      const repository = await this.catalog.rememberPath(selection.path);
      this.statusStore.setRepositoryPath(repository.path);
      if (openAfterSelection) {
        await this.router.navigate(['/workspace', repository.id, 'history']);
      }
    } catch (error) {
      this.localActionError.set(errorMessage(error, 'The repository folder could not be opened.'));
    } finally {
      this.localActionBusy.set(false);
    }
  }
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  return fallback;
}
