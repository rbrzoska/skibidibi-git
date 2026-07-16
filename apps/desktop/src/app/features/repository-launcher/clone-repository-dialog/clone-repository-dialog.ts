import { ChangeDetectionStrategy, Component, inject, output, signal } from '@angular/core';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubRepository } from '../../../core/github';
import { DESKTOP_IPC } from '../../../core/ipc/desktop-ipc';
import { RepositoryCatalog, type RepositoryCatalogEntry } from '../../../core/repositories/repository-catalog';

type CloneMode = 'url' | 'github';
type CloneTransport = 'ssh' | 'https';

@Component({
  selector: 'app-clone-repository-dialog',
  imports: [],
  templateUrl: './clone-repository-dialog.html',
  styleUrl: './clone-repository-dialog.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CloneRepositoryDialog {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly github = inject(GITHUB_BRIDGE);
  private readonly catalog = inject(RepositoryCatalog);
  protected readonly accounts = inject(GitHubAccountStore);

  readonly cloned = output<RepositoryCatalogEntry>();
  protected readonly mode = signal<CloneMode>('url');
  protected readonly transport = signal<CloneTransport>('ssh');
  protected readonly sourceUrl = signal('');
  protected readonly destinationParent = signal('');
  protected readonly directoryName = signal('');
  protected readonly selectedAccountId = signal('');
  protected readonly selectedRepositoryId = signal('');
  protected readonly repositories = signal<readonly GitHubRepository[]>([]);
  protected readonly loadingRepositories = signal(false);
  protected readonly selectingParent = signal(false);
  protected readonly cloning = signal(false);
  protected readonly error = signal('');

  protected setMode(mode: CloneMode): void {
    this.mode.set(mode);
    this.error.set('');
    if (mode !== 'github') {
      return;
    }
    const state = this.accounts.state();
    if (state.kind === 'idle') {
      void this.accounts.load();
    }
    const account = state.kind === 'ready' ? state.accounts[0] : undefined;
    if (account !== undefined && this.selectedAccountId().length === 0) {
      this.selectedAccountId.set(account.id);
      void this.loadRepositories();
    }
  }

  protected updateSourceUrl(value: string): void {
    this.sourceUrl.set(value);
    if (this.directoryName().length === 0) {
      this.directoryName.set(suggestDirectoryName(value));
    }
  }

  protected async selectParent(): Promise<void> {
    if (this.selectingParent()) {
      return;
    }
    this.selectingParent.set(true);
    this.error.set('');
    try {
      const selected = await this.ipc.invoke('select_clone_parent_directory', {
        initialPath: this.destinationParent().trim() || null,
      });
      if (selected.path !== null) {
        this.destinationParent.set(selected.path);
      }
    } catch (error) {
      this.error.set(errorMessage(error, 'The destination folder could not be selected.'));
    } finally {
      this.selectingParent.set(false);
    }
  }

  protected async selectAccount(accountId: string): Promise<void> {
    this.selectedAccountId.set(accountId);
    this.selectedRepositoryId.set('');
    this.repositories.set([]);
    await this.loadRepositories();
  }

  protected selectRepository(repositoryId: string): void {
    this.selectedRepositoryId.set(repositoryId);
    const repository = this.repositories().find(({ id }) => id === repositoryId);
    if (repository !== undefined) {
      this.directoryName.set(repository.name);
    }
  }

  protected async loadRepositories(): Promise<void> {
    const accountId = this.selectedAccountId();
    if (accountId.length === 0 || this.loadingRepositories()) {
      return;
    }
    this.loadingRepositories.set(true);
    this.error.set('');
    try {
      const page = await this.github.githubListRepositories({ accountId, cursor: null, pageSize: 50 });
      this.repositories.set(page.repositories);
    } catch (error) {
      this.error.set(errorMessage(error, 'GitHub repositories could not be loaded.'));
    } finally {
      this.loadingRepositories.set(false);
    }
  }

  protected canClone(): boolean {
    return !this.cloning()
      && this.destinationParent().trim().length > 0
      && this.directoryName().trim().length > 0
      && this.selectedSourceUrl() !== null;
  }

  protected async clone(): Promise<void> {
    const sourceUrl = this.selectedSourceUrl();
    if (!this.canClone() || sourceUrl === null) {
      return;
    }
    this.cloning.set(true);
    this.error.set('');
    try {
      const remembered = await this.ipc.invoke('clone_repository', {
        sourceUrl,
        destinationParent: this.destinationParent().trim(),
        directoryName: this.directoryName().trim(),
      });
      this.cloned.emit(this.catalog.acceptRemembered(remembered));
    } catch (error) {
      this.error.set(errorMessage(error, 'The repository could not be cloned.'));
    } finally {
      this.cloning.set(false);
    }
  }

  protected updateDestinationParent(value: string): void { this.destinationParent.set(value); }
  protected updateDirectoryName(value: string): void { this.directoryName.set(value); }
  protected updateTransport(value: CloneTransport): void { this.transport.set(value); }

  private selectedSourceUrl(): string | null {
    if (this.mode() === 'url') {
      const url = this.sourceUrl().trim();
      return url.length === 0 ? null : url;
    }
    const repository = this.repositories().find(({ id }) => id === this.selectedRepositoryId());
    if (repository === undefined) {
      return null;
    }
    return this.transport() === 'ssh' ? repository.sshCloneUrl : repository.httpsCloneUrl;
  }
}

export function suggestDirectoryName(sourceUrl: string): string {
  const withoutQuery = sourceUrl.trim().split(/[?#]/, 1)[0] ?? '';
  const segment = withoutQuery.replace(/\/+$/, '').split(/[/:]/).at(-1) ?? '';
  return segment.replace(/\.git$/i, '');
}

function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    const message = (error as { readonly message?: unknown }).message;
    if (typeof message === 'string' && message.trim().length > 0) {
      return message;
    }
  }
  return fallback;
}
