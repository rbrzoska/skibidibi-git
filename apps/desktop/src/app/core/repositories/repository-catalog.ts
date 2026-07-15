import { Injectable, computed, inject, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type RememberedRepositoryResponse,
  type RepositoryAvailability,
  type RepositoryTransport,
} from '../ipc/desktop-ipc';

export type RepositoryProvider = 'github' | 'local' | 'other';

export interface RepositoryCatalogEntry {
  readonly id: string;
  readonly name: string;
  readonly path: string;
  readonly provider: RepositoryProvider;
  readonly transport: RepositoryTransport;
  readonly remote: string | null;
  readonly integration: 'connected' | 'local-only' | 'attention' | 'unchecked';
  readonly availability: RepositoryAvailability;
  readonly pinned: boolean;
  readonly lastOpenedLabel: string;
}

export type RepositoryCatalogState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready' }
  | { readonly kind: 'error'; readonly message: string };

@Injectable({ providedIn: 'root' })
export class RepositoryCatalog {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly entries = signal<readonly RepositoryCatalogEntry[]>([]);
  private loadPromise: Promise<void> | null = null;

  readonly repositories = this.entries.asReadonly();
  readonly count = computed(() => this.entries().length);
  readonly state = signal<RepositoryCatalogState>({ kind: 'idle' });

  async load(): Promise<void> {
    if (this.loadPromise !== null) {
      return this.loadPromise;
    }

    this.state.set({ kind: 'loading' });
    this.loadPromise = this.ipc
      .invoke('list_remembered_repositories', {})
      .then((repositories) => {
        this.entries.set(repositories.map(mapRepository));
        this.state.set({ kind: 'ready' });
      })
      .catch(() => {
        this.state.set({
          kind: 'error',
          message: 'Remembered repositories could not be loaded.',
        });
      })
      .finally(() => {
        this.loadPromise = null;
      });
    return this.loadPromise;
  }

  find(repositoryId: string): RepositoryCatalogEntry | undefined {
    return this.entries().find(({ id }) => id === repositoryId);
  }

  async rememberPath(path: string): Promise<RepositoryCatalogEntry> {
    const remembered = await this.ipc.invoke('remember_repository', { repositoryPath: path });
    const repository = mapRepository(remembered);
    this.entries.update((entries) => [
      repository,
      ...entries.filter(({ id }) => id !== repository.id),
    ]);
    this.state.set({ kind: 'ready' });
    return repository;
  }

  async setPinned(repository: RepositoryCatalogEntry, pinned: boolean): Promise<void> {
    const updated = await this.ipc.invoke('set_repository_pinned', {
      repositoryId: repository.id,
      pinned,
    });
    if (updated) {
      this.entries.update((entries) =>
        entries.map((entry) => (entry.id === repository.id ? { ...entry, pinned } : entry)),
      );
    }
  }

  async forget(repository: RepositoryCatalogEntry): Promise<void> {
    const forgotten = await this.ipc.invoke('forget_repository', {
      repositoryId: repository.id,
    });
    if (forgotten) {
      this.entries.update((entries) => entries.filter(({ id }) => id !== repository.id));
    }
  }
}

function mapRepository(repository: RememberedRepositoryResponse): RepositoryCatalogEntry {
  const remote = repository.hostedIdentity;
  const integration =
    repository.githubHealth.state === 'healthy'
      ? 'connected'
      : repository.provider === 'local'
        ? 'local-only'
        : repository.gitHealth.state === 'unknown'
          ? 'unchecked'
          : 'attention';
  return {
    id: repository.id,
    name: repository.displayName,
    path: repository.canonicalPath,
    provider: repository.provider,
    transport: repository.transport,
    remote: remote === null ? null : `${remote.host}/${remote.owner}/${remote.name}`,
    integration,
    availability: repository.availability,
    pinned: repository.pinned,
    lastOpenedLabel: formatLastOpened(repository.lastOpenedAt),
  };
}

function formatLastOpened(timestamp: number | null): string {
  if (timestamp === null) {
    return 'Not opened yet';
  }
  const elapsedSeconds = Math.max(0, Math.floor(Date.now() / 1000) - timestamp);
  if (elapsedSeconds < 60) {
    return 'Just now';
  }
  if (elapsedSeconds < 3600) {
    return `${Math.floor(elapsedSeconds / 60)} min ago`;
  }
  if (elapsedSeconds < 86_400) {
    return `${Math.floor(elapsedSeconds / 3600)} hr ago`;
  }
  return `${Math.floor(elapsedSeconds / 86_400)} days ago`;
}
