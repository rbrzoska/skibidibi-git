import { Injectable, computed, inject, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type RememberedRepositoryResponse,
  type RepositoryGroupRelationResponse,
  type RepositoryAvailability,
  type RepositoryTransport,
} from '../ipc/desktop-ipc';

export type RepositoryProvider = 'github' | 'local' | 'other';

export interface RepositoryCatalogEntry {
  readonly id: string;
  readonly repositoryGroupId: string | null;
  readonly worktreeRole: RememberedRepositoryResponse['worktreeRole'];
  readonly name: string;
  readonly path: string;
  readonly provider: RepositoryProvider;
  readonly transport: RepositoryTransport;
  readonly hostedIdentity: RememberedRepositoryResponse['hostedIdentity'];
  readonly remote: string | null;
  readonly integration: 'connected' | 'local-only' | 'attention' | 'unchecked';
  readonly availability: RepositoryAvailability;
  readonly pinned: boolean;
  readonly lastOpenedAt: number | null;
  readonly lastOpenedLabel: string;
}

export interface RepositoryCatalogGroup {
  readonly id: string;
  readonly repositoryGroupId: string | null;
  readonly grouped: boolean;
  readonly displayName: string;
  readonly representative: RepositoryCatalogEntry;
  readonly defaultRepository: RepositoryCatalogEntry | null;
  readonly worktrees: readonly RepositoryCatalogEntry[];
  /** Child repository groups opened through a Git submodule relation. */
  readonly submodules: readonly RepositoryCatalogGroup[];
  /** Relative path in the parent repository when this is a nested submodule. */
  readonly submodulePath: string | null;
  readonly pinned: boolean;
  readonly lastOpenedAt: number | null;
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
  private readonly relations = signal<readonly RepositoryGroupRelationResponse[]>([]);
  private loadPromise: Promise<void> | null = null;

  readonly repositories = this.entries.asReadonly();
  readonly groups = computed(() => buildRepositoryGroups(this.entries(), this.relations()));
  readonly count = computed(() => this.entries().length);
  readonly groupCount = computed(() => this.groups().length);
  readonly state = signal<RepositoryCatalogState>({ kind: 'idle' });

  async load(): Promise<void> {
    if (this.loadPromise !== null) {
      return this.loadPromise;
    }

    this.state.set({ kind: 'loading' });
    // Keep the existing catalogue available as soon as its records arrive.
    // Relationship metadata is optional enrichment and must not delay opening
    // a repository (nor break older desktop builds that do not expose it).
    void this.ipc.invoke('list_repository_relations', {})
      .then((response) => {
        if (Array.isArray(response)) {
          this.relations.set(response.filter(isRepositoryGroupRelation));
        }
      })
      .catch(() => undefined);
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
    return this.acceptRemembered(remembered);
  }

  async openSubmodule(parentRepositoryId: string, path: string): Promise<RepositoryCatalogEntry> {
    const remembered = await this.ipc.invoke('open_submodule_repository', {
      parentRepositoryId,
      path,
    });
    const repository = this.acceptRemembered(remembered);
    await this.refreshRelations();
    return repository;
  }

  private async refreshRelations(): Promise<void> {
    try {
      const response = await this.ipc.invoke('list_repository_relations', {});
      if (Array.isArray(response)) {
        this.relations.set(response.filter(isRepositoryGroupRelation));
      }
    } catch {
      // Relation metadata is optional enrichment. Opening the repository must
      // still succeed when talking to an older desktop backend.
    }
  }

  acceptRemembered(remembered: RememberedRepositoryResponse): RepositoryCatalogEntry {
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
    repositoryGroupId: repository.repositoryGroupId,
    worktreeRole: repository.worktreeRole,
    name: repository.displayName,
    path: repository.canonicalPath,
    provider: repository.provider,
    transport: repository.transport,
    hostedIdentity: repository.hostedIdentity,
    remote: remote === null ? null : `${remote.host}/${remote.owner}/${remote.name}`,
    integration,
    availability: repository.availability,
    pinned: repository.pinned,
    lastOpenedAt: repository.lastOpenedAt,
    lastOpenedLabel: formatLastOpened(repository.lastOpenedAt),
  };
}

function isRepositoryGroupRelation(value: unknown): value is RepositoryGroupRelationResponse {
  return (
    typeof value === 'object' &&
    value !== null &&
    'kind' in value && value.kind === 'submodule' &&
    'parentRepositoryGroupId' in value && typeof value.parentRepositoryGroupId === 'string' &&
    'childRepositoryGroupId' in value && typeof value.childRepositoryGroupId === 'string' &&
    'relativePath' in value && typeof value.relativePath === 'string'
  );
}

function buildRepositoryGroups(
  repositories: readonly RepositoryCatalogEntry[],
  relations: readonly RepositoryGroupRelationResponse[],
): readonly RepositoryCatalogGroup[] {
  const grouped = new Map<string, RepositoryCatalogEntry[]>();

  for (const repository of repositories) {
    const key =
      repository.repositoryGroupId === null
        ? `repository:${repository.id}`
        : `group:${repository.repositoryGroupId}`;
    const entries = grouped.get(key) ?? [];
    entries.push(repository);
    grouped.set(key, entries);
  }

  const baseGroups = [...grouped.entries()]
    .map(([id, entries]): RepositoryCatalogGroup => {
      const worktrees = [...entries].sort(compareWorktrees);
      const main = worktrees.find(({ worktreeRole }) => worktreeRole === 'main');
      const representative = main ?? newest(worktrees.filter(isAvailable)) ?? newest(worktrees)!;
      const defaultRepository = newest(worktrees.filter(isAvailable)) ?? null;

      return {
        id,
        repositoryGroupId: representative.repositoryGroupId,
        grouped: worktrees.length > 1 || representative.worktreeRole === 'linked',
        displayName: representative.hostedIdentity?.name ?? representative.name,
        representative,
        defaultRepository,
        worktrees,
        submodules: [],
        submodulePath: null,
        pinned: worktrees.some(({ pinned }) => pinned),
        lastOpenedAt: newest(worktrees)?.lastOpenedAt ?? null,
      };
    });

  const groupsByRepositoryGroupId = new Map(
    baseGroups
      .filter((group): group is RepositoryCatalogGroup & { readonly repositoryGroupId: string } =>
        group.repositoryGroupId !== null,
      )
      .map((group) => [group.repositoryGroupId, group]),
  );
  const childrenByParent = new Map<string, readonly RepositoryGroupRelationResponse[]>();
  for (const relation of relations) {
    if (
      relation.kind !== 'submodule' ||
      !groupsByRepositoryGroupId.has(relation.parentRepositoryGroupId) ||
      !groupsByRepositoryGroupId.has(relation.childRepositoryGroupId)
    ) {
      continue;
    }
    childrenByParent.set(
      relation.parentRepositoryGroupId,
      [...(childrenByParent.get(relation.parentRepositoryGroupId) ?? []), relation],
    );
  }

  const childGroupIds = new Set(
    [...childrenByParent.values()].flat().map(({ childRepositoryGroupId }) => childRepositoryGroupId),
  );
  const roots = baseGroups.filter(
    (group) => group.repositoryGroupId === null || !childGroupIds.has(group.repositoryGroupId),
  );
  // A malformed relationship cycle has no natural root. Pick one stable entry
  // point instead of rendering every member both at top level and as a child;
  // recursion below cuts the closing edge.
  const topLevel = roots.length > 0 ? roots : baseGroups.slice(0, 1);

  return topLevel
    .map((group) => buildNestedGroup(group, childrenByParent, groupsByRepositoryGroupId, new Set(), 0, null))
    .sort(compareGroups);
}

const MAX_SUBMODULE_DEPTH = 8;

function buildNestedGroup(
  group: RepositoryCatalogGroup,
  childrenByParent: ReadonlyMap<string, readonly RepositoryGroupRelationResponse[]>,
  groupsByRepositoryGroupId: ReadonlyMap<string, RepositoryCatalogGroup>,
  ancestors: ReadonlySet<string>,
  depth: number,
  submodulePath: string | null,
): RepositoryCatalogGroup {
  const groupId = group.repositoryGroupId;
  if (groupId === null || depth >= MAX_SUBMODULE_DEPTH || ancestors.has(groupId)) {
    return { ...group, submodulePath, submodules: [] };
  }
  const nextAncestors = new Set(ancestors).add(groupId);
  const submodules = (childrenByParent.get(groupId) ?? [])
    .map((relation) => {
      const child = groupsByRepositoryGroupId.get(relation.childRepositoryGroupId);
      if (child === undefined || nextAncestors.has(relation.childRepositoryGroupId)) {
        return null;
      }
      return buildNestedGroup(
        child,
        childrenByParent,
        groupsByRepositoryGroupId,
        nextAncestors,
        depth + 1,
        relation.relativePath,
      );
    })
    .filter((child): child is RepositoryCatalogGroup => child !== null)
    .sort(compareGroups);
  return { ...group, submodulePath, submodules };
}

function isAvailable(repository: RepositoryCatalogEntry): boolean {
  return repository.availability === 'available';
}

function newest(
  repositories: readonly RepositoryCatalogEntry[],
): RepositoryCatalogEntry | undefined {
  return [...repositories].sort((left, right) => compareTimestamps(right, left))[0];
}

function compareWorktrees(left: RepositoryCatalogEntry, right: RepositoryCatalogEntry): number {
  const roleOrder: Record<RememberedRepositoryResponse['worktreeRole'], number> = {
    main: 0,
    linked: 1,
    bare: 2,
    unknown: 3,
  };
  return (
    roleOrder[left.worktreeRole] - roleOrder[right.worktreeRole] ||
    compareTimestamps(right, left) ||
    left.path.localeCompare(right.path)
  );
}

function compareGroups(left: RepositoryCatalogGroup, right: RepositoryCatalogGroup): number {
  return (
    Number(right.pinned) - Number(left.pinned) ||
    compareNullableTimestamps(right.lastOpenedAt, left.lastOpenedAt) ||
    left.representative.name.localeCompare(right.representative.name)
  );
}

function compareTimestamps(left: RepositoryCatalogEntry, right: RepositoryCatalogEntry): number {
  return compareNullableTimestamps(left.lastOpenedAt, right.lastOpenedAt);
}

function compareNullableTimestamps(left: number | null, right: number | null): number {
  return (left ?? Number.NEGATIVE_INFINITY) - (right ?? Number.NEGATIVE_INFINITY);
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
