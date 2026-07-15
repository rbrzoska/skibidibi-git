import { Injectable, inject, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type RepositoryStatusResponse,
} from '../../core/ipc/desktop-ipc';

export type RepositoryStatusState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly status: RepositoryStatusResponse }
  | { readonly kind: 'error'; readonly message: string };

@Injectable({ providedIn: 'root' })
export class RepositoryStatusStore {
  private readonly ipc = inject(DESKTOP_IPC);

  readonly repositoryPath = signal('~/projects/skibidibi-git');
  readonly state = signal<RepositoryStatusState>({ kind: 'loading' });

  constructor() {
    void this.refresh();
  }

  async refresh(): Promise<void> {
    this.state.set({ kind: 'loading' });

    try {
      const status = await this.ipc.invoke('repository_status', {
        repositoryPath: this.repositoryPath(),
      });
      this.state.set({ kind: 'ready', status });
    } catch {
      this.state.set({
        kind: 'error',
        message: 'Repository status is unavailable. Check the desktop bridge and try again.',
      });
    }
  }
}
