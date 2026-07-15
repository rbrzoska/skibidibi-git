import { Injectable, inject, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type RepositoryStatusResponse,
} from '../../core/ipc/desktop-ipc';

export type RepositoryStatusState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly status: RepositoryStatusResponse }
  | { readonly kind: 'error'; readonly message: string };

@Injectable({ providedIn: 'root' })
export class RepositoryStatusStore {
  private readonly ipc = inject(DESKTOP_IPC);
  private foregroundGeneration = 0;
  private backgroundGeneration = 0;

  readonly repositoryPath = signal('');
  readonly isSelectingDirectory = signal(false);
  readonly state = signal<RepositoryStatusState>({ kind: 'idle' });
  readonly backgroundError = signal('');

  setRepositoryPath(repositoryPath: string): void {
    ++this.foregroundGeneration;
    ++this.backgroundGeneration;
    this.repositoryPath.set(repositoryPath.trim());
    this.backgroundError.set('');
  }

  acceptMutationResult(status: RepositoryStatusResponse): void {
    ++this.foregroundGeneration;
    ++this.backgroundGeneration;
    this.backgroundError.set('');
    this.state.set({ kind: 'ready', status });
  }

  async selectRepositoryDirectory(): Promise<void> {
    if (this.isSelectingDirectory()) {
      return;
    }

    this.isSelectingDirectory.set(true);
    try {
      const selection = await this.ipc.invoke('select_repository_directory', {
        initialPath: this.repositoryPath() || null,
      });

      if (selection.path !== null) {
        this.setRepositoryPath(selection.path);
        await this.refresh();
      }
    } catch {
      this.state.set({
        kind: 'error',
        message: 'The directory picker is unavailable. Enter the repository path manually.',
      });
    } finally {
      this.isSelectingDirectory.set(false);
    }
  }

  async refresh(options: { readonly silent?: boolean } = {}): Promise<void> {
    const repositoryPath = this.repositoryPath();
    if (repositoryPath.length === 0) {
      this.state.set({ kind: 'idle' });
      return;
    }

    if (options.silent) {
      if (this.state().kind !== 'ready') {
        return;
      }
      const foregroundGeneration = this.foregroundGeneration;
      const backgroundGeneration = ++this.backgroundGeneration;
      try {
        const status = await this.ipc.invoke('repository_status', { repositoryPath });
        if (
          foregroundGeneration === this.foregroundGeneration &&
          backgroundGeneration === this.backgroundGeneration &&
          repositoryPath === this.repositoryPath() &&
          this.state().kind === 'ready'
        ) {
          this.backgroundError.set('');
          this.state.set({ kind: 'ready', status });
        }
      } catch {
        if (
          foregroundGeneration === this.foregroundGeneration &&
          backgroundGeneration === this.backgroundGeneration &&
          repositoryPath === this.repositoryPath()
        ) {
          this.backgroundError.set('Live working-tree refresh failed. The displayed status may be stale.');
        }
      }
      return;
    }

    const generation = ++this.foregroundGeneration;
    ++this.backgroundGeneration;
    this.backgroundError.set('');
    this.state.set({ kind: 'loading' });

    try {
      const status = await this.ipc.invoke('repository_status', {
        repositoryPath,
      });
      if (generation === this.foregroundGeneration && repositoryPath === this.repositoryPath()) {
        this.state.set({ kind: 'ready', status });
      }
    } catch {
      if (generation === this.foregroundGeneration && repositoryPath === this.repositoryPath()) {
        this.state.set({
          kind: 'error',
          message: 'Repository status is unavailable. Check the path and desktop bridge.',
        });
      }
    }
  }
}
