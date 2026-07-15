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
  private requestGeneration = 0;

  readonly repositoryPath = signal('');
  readonly state = signal<RepositoryStatusState>({ kind: 'idle' });

  setRepositoryPath(repositoryPath: string): void {
    this.repositoryPath.set(repositoryPath.trim());
  }

  async refresh(): Promise<void> {
    const repositoryPath = this.repositoryPath();
    if (repositoryPath.length === 0) {
      this.state.set({ kind: 'idle' });
      return;
    }

    const generation = ++this.requestGeneration;
    this.state.set({ kind: 'loading' });

    try {
      const status = await this.ipc.invoke('repository_status', {
        repositoryPath,
      });
      if (generation === this.requestGeneration) {
        this.state.set({ kind: 'ready', status });
      }
    } catch {
      if (generation === this.requestGeneration) {
        this.state.set({
          kind: 'error',
          message: 'Repository status is unavailable. Check the path and desktop bridge.',
        });
      }
    }
  }
}
