import {
  ChangeDetectionStrategy,
  Component,
  type OnDestroy,
  computed,
  inject,
  signal,
} from '@angular/core';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';

import {
  DESKTOP_IPC,
  type CommitChangedFile,
  type ChangeSelection,
  type IndexAction,
  type RepositoryCommitDetailResponse,
  type RepositoryFileDiffResponse,
  type RepositoryCommitSummary,
  type RepositoryBranch,
  type RepositoryNavigationResponse,
  type RepositoryStatusResponse,
  type RepositoryStash,
  type RepositoryWorktree,
  type SwitchRepositoryBranchResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import {
  BranchExpansionState,
  branchFolderPaths,
  type BranchExpansionScope,
} from './branch-expansion-state';
import { buildBranchTree, visibleBranchTree } from './branch-tree';
import { createContextualDiffRows } from './contextual-diff';
import { parseUnifiedDiff } from './unified-diff';
import {
  planWorkingTreeMutation,
  reconcileWorkingTreeSelection,
  workingTreeActionCapabilities,
  workingTreeFileKey,
} from './working-tree-actions';
import {
  createWorkingTreeSummary,
  type WorkingTreeFile,
  type WorkingTreePrimaryStatus,
} from './working-tree-summary';
import {
  AUTO_FETCH_INTERVAL_MS,
  LIVE_STATUS_INTERVAL_MS,
  browserWorkspaceRefreshStorage,
  buildWipStashMessage,
  readWorkspaceRefreshPreferences,
  writeWorkspaceRefreshSetting,
} from './workspace-refresh-policy';

type HistoryPhase = 'idle' | 'loading' | 'ready' | 'error';
type FileDiffDisplayMode = 'contextual' | 'full';
type HistorySelection = 'none' | 'working-tree' | 'commit';
type WorkingTreeMutation =
  | { readonly action: IndexAction; readonly scope: 'selected' | 'all' }
  | { readonly action: 'commit' | 'amend'; readonly scope: null };
type BranchCreationTarget =
  | { readonly kind: 'current'; readonly label: string }
  | { readonly kind: 'remote'; readonly branch: RepositoryBranch };
type DetailState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly detail: RepositoryCommitDetailResponse }
  | { readonly kind: 'error'; readonly message: string };
type NavigationState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly navigation: RepositoryNavigationResponse }
  | { readonly kind: 'error'; readonly message: string };
type FileDiffState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly path: string; readonly oldPath: string | null }
  | {
      readonly kind: 'ready';
      readonly response: Pick<RepositoryFileDiffResponse, 'path' | 'patch' | 'binary' | 'truncated'>;
    }
  | { readonly kind: 'error'; readonly path: string; readonly oldPath: string | null; readonly message: string };

const HISTORY_PAGE_SIZE = 50;
const MAX_RENDERED_DIFF_ROWS = 20_000;
const SIDEBAR_WIDTH_KEY = 'skibidibi-git.workspace.sidebar-width';
const SIDEBAR_MIN_WIDTH = 220;
const SIDEBAR_DEFAULT_WIDTH = 288;
const SIDEBAR_KEYBOARD_STEP = 16;
const ROOT_FONT_SIZE = 16;
const COMPACT_LAYOUT_BREAKPOINT = 48 * ROOT_FONT_SIZE;
const INSPECTOR_LAYOUT_BREAKPOINT = 68 * ROOT_FONT_SIZE;
const HISTORY_MIN_WIDTH = 24 * ROOT_FONT_SIZE;
const INSPECTOR_MIN_WIDTH = 17 * ROOT_FONT_SIZE;
const SIDEBAR_RESIZER_WIDTH = 0.4 * ROOT_FONT_SIZE;

@Component({
  selector: 'app-workspace-history',
  imports: [RouterLink],
  providers: [RepositoryStatusStore],
  templateUrl: './workspace-history.html',
  styleUrl: './workspace-history.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class WorkspaceHistory implements OnDestroy {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly catalog = inject(RepositoryCatalog);
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly branchExpansionState = new BranchExpansionState();
  private historyRequestGeneration = 0;
  private detailRequestGeneration = 0;
  private navigationRequestGeneration = 0;
  private fileDiffRequestGeneration = 0;
  private mutationRequestGeneration = 0;
  private destroyed = false;
  private fileDiffReturnFocus: HTMLElement | null = null;
  private activeResizePointer: number | null = null;
  private autoFetchTimer: ReturnType<typeof globalThis.setInterval> | null = null;
  private liveChangesTimer: ReturnType<typeof globalThis.setInterval> | null = null;
  private liveRefreshInFlight = false;
  private branchCreationReturnFocus: HTMLElement | null = null;
  private amendReturnFocus: HTMLElement | null = null;

  protected readonly statusStore = inject(RepositoryStatusStore);
  protected readonly repositoryId = this.route.snapshot.paramMap.get('repositoryId') ?? '';
  protected readonly repository = computed(() => this.catalog.find(this.repositoryId));
  private readonly refreshStorage = browserWorkspaceRefreshStorage();
  private readonly initialRefreshPreferences = readWorkspaceRefreshPreferences(
    this.refreshStorage,
    this.repositoryId,
  );
  protected readonly historyPhase = signal<HistoryPhase>('idle');
  protected readonly historyError = signal('');
  protected readonly commits = signal<readonly RepositoryCommitSummary[]>([]);
  protected readonly nextCursor = signal<string | null>(null);
  protected readonly isLoadingMore = signal(false);
  protected readonly selectedOid = signal<string | null>(null);
  protected readonly historySelection = signal<HistorySelection>('none');
  protected readonly detailState = signal<DetailState>({ kind: 'idle' });
  protected readonly navigationState = signal<NavigationState>({ kind: 'loading' });
  protected readonly navigationFilter = signal('');
  protected readonly expandedLocalBranchFolders = signal<ReadonlySet<string>>(new Set());
  protected readonly expandedRemoteBranchFolders = signal<ReadonlySet<string>>(new Set());
  protected readonly sidebarWidth = signal(this.readSidebarWidth());
  protected readonly resizingSidebar = signal(false);
  protected readonly switchingBranch = signal<string | null>(null);
  protected readonly openingWorktree = signal<string | null>(null);
  protected readonly refreshingWorkspace = signal(false);
  protected readonly navigationActionError = signal('');
  protected readonly navigationActionNotice = signal('');
  protected readonly currentOnly = signal(this.initialRefreshPreferences.currentOnly);
  protected readonly autoFetch = signal(this.initialRefreshPreferences.autoFetch);
  protected readonly liveChanges = signal(this.initialRefreshPreferences.liveChanges);
  protected readonly fetchingRepository = signal(false);
  protected readonly deletingBranch = signal<string | null>(null);
  protected readonly removingWorktree = signal<string | null>(null);
  protected readonly branchCreationTarget = signal<BranchCreationTarget | null>(null);
  protected readonly newBranchName = signal('');
  protected readonly creatingBranch = signal(false);
  protected readonly stashMessage = signal('');
  protected readonly stashIncludeUntracked = signal(true);
  protected readonly stashMutation = signal<string | null>(null);
  protected readonly selectedFilePath = signal<string | null>(null);
  protected readonly selectedWorkingTreeEntryKind = signal<WorkingTreeFile['entryKind'] | null>(null);
  protected readonly fileDiffState = signal<FileDiffState>({ kind: 'idle' });
  protected readonly fileDiffDisplayMode = signal<FileDiffDisplayMode>('contextual');
  protected readonly selectedWorkingTreeFiles = signal<ReadonlySet<string>>(new Set());
  protected readonly commitMessage = signal('');
  protected readonly amendMode = signal(false);
  protected readonly workingTreeMutation = signal<WorkingTreeMutation | null>(null);
  protected readonly workingTreeMutationError = signal('');
  protected readonly workspaceActionBusy = computed(
    () =>
      this.workingTreeMutation() !== null ||
      this.switchingBranch() !== null ||
      this.openingWorktree() !== null ||
      this.refreshingWorkspace() ||
      this.fetchingRepository() ||
      this.deletingBranch() !== null ||
      this.removingWorktree() !== null ||
      this.creatingBranch() ||
      this.stashMutation() !== null,
  );
  protected readonly parsedFileDiff = computed(() => {
    const state = this.fileDiffState();
    return state.kind === 'ready' ? parseUnifiedDiff(state.response.patch) : null;
  });
  protected readonly contextualFileDiffRows = computed(() =>
    createContextualDiffRows(this.parsedFileDiff()?.rows ?? []),
  );
  protected readonly displayedFileDiffRows = computed(() =>
    this.fileDiffDisplayMode() === 'full'
      ? (this.parsedFileDiff()?.rows ?? [])
      : this.contextualFileDiffRows(),
  );
  protected readonly visibleFileDiffRows = computed(
    () => this.displayedFileDiffRows().slice(0, MAX_RENDERED_DIFF_ROWS),
  );
  protected readonly isFileDiffVisuallyTruncated = computed(
    () => this.displayedFileDiffRows().length > MAX_RENDERED_DIFF_ROWS,
  );

  protected readonly localBranches = computed(() => this.branchesOfKind('local'));
  protected readonly remoteBranches = computed(() => this.branchesOfKind('remote'));
  protected readonly currentLocalBranch = computed(
    () => this.localBranches().find((branch) => branch.current) ?? null,
  );
  protected readonly otherLocalBranches = computed(() =>
    this.localBranches().filter((branch) => !branch.current),
  );
  protected readonly localBranchHierarchy = computed(() =>
    buildBranchTree(this.otherLocalBranches()),
  );
  protected readonly remoteBranchHierarchy = computed(() =>
    buildBranchTree(this.remoteBranches()),
  );
  protected readonly localBranchFolderPaths = computed(() =>
    branchFolderPaths(this.localBranchHierarchy()),
  );
  protected readonly remoteBranchFolderPaths = computed(() =>
    branchFolderPaths(this.remoteBranchHierarchy()),
  );
  protected readonly orderedWorktrees = computed(() => {
    const state = this.navigationState();
    if (state.kind !== 'ready') {
      return [];
    }
    return [...state.navigation.worktrees].sort(
      (left, right) => Number(this.isCurrentWorktree(right)) - Number(this.isCurrentWorktree(left)),
    );
  });
  protected readonly visibleWorktrees = computed(() =>
    this.currentOnly()
      ? this.orderedWorktrees().filter((worktree) => this.isCurrentWorktree(worktree))
      : this.orderedWorktrees(),
  );
  protected readonly localBranchTree = computed(() =>
    visibleBranchTree(
      this.localBranchHierarchy(),
      this.collapsedFoldersFor('local'),
      this.navigationFilter(),
    ),
  );
  protected readonly remoteBranchTree = computed(() =>
    visibleBranchTree(
      this.remoteBranchHierarchy(),
      this.collapsedFoldersFor('remote'),
      this.navigationFilter(),
    ),
  );

  protected readonly branchName = computed(() => {
    const state = this.statusStore.state();
    return state.kind === 'ready' ? (state.status.branch.head ?? 'detached HEAD') : 'loading';
  });

  protected readonly workingTreeSummary = computed(() => {
    const state = this.statusStore.state();
    if (state.kind !== 'ready') {
      return {
        files: [],
        counts: { added: 0, modified: 0, renamed: 0, deleted: 0, conflicted: 0, total: 0 },
      };
    }
    return createWorkingTreeSummary(state.status);
  });
  protected readonly reconciledWorkingTreeSelection = computed(() =>
    reconcileWorkingTreeSelection(
      this.selectedWorkingTreeFiles(),
      this.workingTreeSummary().files,
    ),
  );
  protected readonly workingTreeCapabilities = computed(() =>
    workingTreeActionCapabilities(
      this.workingTreeSummary().files,
      this.reconciledWorkingTreeSelection(),
    ),
  );
  protected readonly commitDisabled = computed(
    () =>
      this.workspaceActionBusy() ||
      this.commitMessage().trim().length === 0 ||
      !this.workingTreeCapabilities().canCommit,
  );
  protected readonly amendUnavailable = computed(() => {
    const state = this.statusStore.state();
    return (
      state.kind !== 'ready' ||
      state.status.branch.oid === null ||
      state.status.branch.unborn ||
      this.workspaceActionBusy()
    );
  });
  protected readonly amendWithMessageDisabled = computed(
    () => this.amendUnavailable() || this.commitMessage().trim().length === 0,
  );
  protected readonly amendRewritesUpstream = computed(() => {
    const state = this.statusStore.state();
    return (
      state.kind === 'ready' &&
      state.status.branch.upstream !== null &&
      state.status.branch.ahead === 0
    );
  });

  protected readonly selectedFileSummary = computed(() => {
    const state = this.detailState();
    if (state.kind !== 'ready') {
      return { files: 0, additions: 0, deletions: 0 };
    }
    return state.detail.files.reduce(
      (summary, file) => ({
        files: summary.files + 1,
        additions: summary.additions + (file.additions ?? 0),
        deletions: summary.deletions + (file.deletions ?? 0),
      }),
      { files: 0, additions: 0, deletions: 0 },
    );
  });

  constructor() {
    globalThis.addEventListener('resize', this.clampSidebarToViewport);
    this.configureAutoFetchTimer();
    this.configureLiveChangesTimer();
    void this.loadRepository();
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    ++this.historyRequestGeneration;
    ++this.detailRequestGeneration;
    ++this.navigationRequestGeneration;
    ++this.fileDiffRequestGeneration;
    ++this.mutationRequestGeneration;
    this.clearAutoFetchTimer();
    this.clearLiveChangesTimer();
    this.stopSidebarResize();
    globalThis.removeEventListener('resize', this.clampSidebarToViewport);
  }

  protected setCurrentOnly(enabled: boolean): void {
    this.currentOnly.set(enabled);
    if (enabled && this.branchCreationTarget()?.kind === 'remote') {
      this.branchCreationTarget.set(null);
      this.newBranchName.set('');
      this.branchCreationReturnFocus = null;
    }
    writeWorkspaceRefreshSetting(this.refreshStorage, this.repositoryId, 'currentOnly', enabled);
  }

  protected setAutoFetch(enabled: boolean): void {
    this.autoFetch.set(enabled);
    writeWorkspaceRefreshSetting(this.refreshStorage, this.repositoryId, 'autoFetch', enabled);
    this.configureAutoFetchTimer();
    if (enabled) {
      void this.fetchRepository();
    }
  }

  protected setLiveChanges(enabled: boolean): void {
    this.liveChanges.set(enabled);
    writeWorkspaceRefreshSetting(this.refreshStorage, this.repositoryId, 'liveChanges', enabled);
    this.configureLiveChangesTimer();
    if (enabled && !this.workspaceActionBusy()) {
      void this.refreshLiveChanges();
    }
  }

  protected async fetchRepository(): Promise<void> {
    if (this.destroyed || this.workspaceActionBusy()) {
      return;
    }
    this.navigationActionError.set('');
    this.fetchingRepository.set(true);
    try {
      await this.ipc.invoke('repository_fetch', { repositoryId: this.repositoryId });
      if (!this.destroyed) {
        await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
      }
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The repository could not be fetched.'));
      }
    } finally {
      if (!this.destroyed) {
        this.fetchingRepository.set(false);
      }
    }
  }

  protected async refreshWorkspace(): Promise<void> {
    if (this.workspaceActionBusy()) {
      return;
    }
    this.navigationActionError.set('');
    this.refreshingWorkspace.set(true);
    try {
      await Promise.all([
        this.statusStore.refresh(),
        this.reloadHistory(),
        this.loadNavigation(),
      ]);
    } finally {
      if (!this.destroyed) {
        this.refreshingWorkspace.set(false);
      }
    }
  }

  protected updateNavigationFilter(value: string): void {
    this.navigationFilter.set(value);
  }

  protected startBranchFromCurrent(trigger: HTMLElement): void {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || state.status.branch.oid === null || this.workspaceActionBusy()) {
      return;
    }
    this.branchCreationTarget.set({ kind: 'current', label: state.status.branch.head ?? 'HEAD' });
    this.branchCreationReturnFocus = trigger;
    this.newBranchName.set('');
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.focusAfterRender('new-branch-name');
  }

  protected startBranchFromRemote(branch: RepositoryBranch, trigger: HTMLElement): void {
    if (branch.kind !== 'remote' || this.workspaceActionBusy()) {
      return;
    }
    this.branchCreationTarget.set({ kind: 'remote', branch });
    this.branchCreationReturnFocus = trigger;
    this.newBranchName.set('');
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.focusAfterRender('new-branch-name');
  }

  protected updateNewBranchName(name: string): void {
    this.newBranchName.set(name);
  }

  protected cancelBranchCreation(): void {
    if (this.creatingBranch()) {
      return;
    }
    this.branchCreationTarget.set(null);
    this.newBranchName.set('');
    this.restoreFocusAfterRender(this.branchCreationReturnFocus);
    this.branchCreationReturnFocus = null;
  }

  protected async createBranch(): Promise<void> {
    const target = this.branchCreationTarget();
    const name = this.newBranchName().trim();
    const state = this.statusStore.state();
    if (target === null || name.length === 0 || state.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }
    const source = target.kind === 'remote'
      ? {
          kind: 'remoteTracking' as const,
          fullName: target.branch.fullName,
          expectedOid: target.branch.oid,
        }
      : state.status.branch.oid === null
        ? null
        : { kind: 'current' as const, expectedOid: state.status.branch.oid };
    if (source === null) {
      return;
    }

    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.creatingBranch.set(true);
    let restoreFocusOnSuccess = false;
    try {
      const created = await this.ipc.invoke('create_repository_branch', {
        repositoryId: this.repositoryId,
        operation: { name, source },
      });
      if (this.destroyed) {
        return;
      }
      this.branchCreationTarget.set(null);
      this.newBranchName.set('');
      restoreFocusOnSuccess = true;
      this.navigationActionNotice.set(
        created.upstream === null
          ? `Created local branch “${created.name}”.`
          : `Created local branch “${created.name}” tracking ${created.upstream}.`,
      );
      await this.loadNavigation();
    } catch (error) {
      if (!this.destroyed) {
        const message = this.errorMessage(error, 'The branch could not be created.');
        this.navigationActionError.set(
          message.includes('manualCleanupRequired')
            ? `The branch may have been created, but final verification failed. Inspect the refreshed branch list and delete it manually if it is not wanted. ${message}`
            : message,
        );
        await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
      }
    } finally {
      if (!this.destroyed) {
        this.creatingBranch.set(false);
        if (restoreFocusOnSuccess) {
          this.restoreFocusAfterRender(this.branchCreationReturnFocus);
          this.branchCreationReturnFocus = null;
        }
      }
    }
  }

  protected updateStashMessage(message: string): void {
    this.stashMessage.set(message);
  }

  protected setStashIncludeUntracked(include: boolean): void {
    this.stashIncludeUntracked.set(include);
  }

  protected async pushStash(): Promise<void> {
    const state = this.statusStore.state();
    const message = this.stashMessage().trim();
    if (state.kind !== 'ready' || message.length === 0 || this.workspaceActionBusy()) {
      return;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.stashMutation.set('create');
    try {
      const result = await this.ipc.invoke('repository_push_stash', {
        repositoryId: this.repositoryId,
        operation: {
          message,
          includeUntracked: this.stashIncludeUntracked(),
          precondition: this.repositoryStatePrecondition(state.status),
        },
      });
      if (this.destroyed) {
        return;
      }
      if (result.status !== null) {
        this.statusStore.acceptMutationResult(result.status);
      }
      const verifiedCreated =
        result.state === 'created' &&
        result.status !== null &&
        result.stash !== null &&
        result.errorMessage === null;
      const verifiedNoChanges =
        result.state === 'noChanges' && result.status !== null && result.errorMessage === null;
      if (verifiedCreated) {
        this.stashMessage.set('');
        this.navigationActionNotice.set(
          `Created ${result.stash.selector}.`,
        );
      } else if (verifiedNoChanges) {
        this.navigationActionNotice.set('There were no changes to stash.');
      } else {
        const recovery = result.mutationMayHaveOccurred
          ? ' A mutating command was attempted; inspect the refreshed working tree and stash list before retrying.'
          : '';
        this.navigationActionError.set(
          result.state === 'partial'
            ? `The stash operation completed only partially${result.stash ? `; ${result.stash.selector} was retained` : ''}. ${result.errorMessage ?? 'Review the repository state before continuing.'}${recovery}`
            : `${result.errorMessage ?? 'The stash result could not be verified.'}${recovery}`,
        );
      }
      this.clearWorkingTreeMutationView();
      await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The stash could not be created.'));
        await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
      }
    } finally {
      if (!this.destroyed) {
        this.stashMutation.set(null);
      }
    }
  }

  protected async applyStash(stash: RepositoryStash): Promise<void> {
    await this.restoreStash('apply', stash);
  }

  protected async popStash(stash: RepositoryStash): Promise<void> {
    await this.restoreStash('pop', stash);
  }

  protected async dropStash(stash: RepositoryStash): Promise<void> {
    if (this.workspaceActionBusy()) {
      return;
    }
    if (!globalThis.confirm(`Drop ${stash.selector} “${stash.message}”? This cannot be undone.`)) {
      return;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.stashMutation.set(`drop:${stash.oid}`);
    try {
      const result = await this.ipc.invoke('repository_drop_stash', {
        repositoryId: this.repositoryId,
        operation: { stash: { oid: stash.oid, selector: stash.selector } },
      });
      if (this.destroyed) {
        return;
      }
      const verifiedDrop = result.cleanup === 'dropped' && result.errorMessage === null;
      if (verifiedDrop) {
        this.navigationActionNotice.set(`Dropped ${stash.selector}.`);
      } else {
        const recovery = result.mutationMayHaveOccurred
          ? ' A mutating command was attempted; inspect the refreshed stash list before retrying.'
          : '';
        this.navigationActionError.set(
          `${stash.selector} was not confirmed dropped (${result.cleanup}). ${result.errorMessage ?? 'It may remain available in the stash list.'}${recovery}`,
        );
      }
      await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, `${stash.selector} could not be dropped.`));
        await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
      }
    } finally {
      if (!this.destroyed) {
        this.stashMutation.set(null);
      }
    }
  }

  protected toggleBranchFolder(kind: RepositoryBranch['kind'], path: string): void {
    const expanded = this.expandedFoldersFor(kind);
    expanded.set(
      this.branchExpansionState.toggle(
        this.repositoryId,
        kind,
        path,
        expanded(),
        this.availableFolderPathsFor(kind),
      ),
    );
  }

  protected async switchBranch(branch: RepositoryBranch): Promise<void> {
    if (branch.kind !== 'local' || branch.current || this.workspaceActionBusy()) {
      return;
    }
    if (!globalThis.confirm(`Switch the active worktree to “${branch.name}”?`)) {
      return;
    }

    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.switchingBranch.set(branch.fullName);
    try {
      const result = await this.ipc.invoke('switch_repository_branch', {
        repositoryId: this.repositoryId,
        operation: {
          fullName: branch.fullName,
          expectedOid: branch.oid,
          stashOnDirty: false,
          stashMessage: null,
        },
      });
      if (this.destroyed) {
        return;
      }
      this.recordBranchSwitchOutcome(result);
      await this.refreshAfterBranchSwitch();
    } catch (error) {
      const message = this.errorMessage(error, 'The branch could not be switched.');
      if (!this.destroyed && message.includes('dirtyWorkingTree')) {
        const currentBranch = this.currentLocalBranch()?.name ?? this.branchName();
        if (globalThis.confirm(`The working tree has uncommitted changes. Stash them and switch to “${branch.name}”?`)) {
          try {
            const result = await this.ipc.invoke('switch_repository_branch', {
              repositoryId: this.repositoryId,
              operation: {
                fullName: branch.fullName,
                expectedOid: branch.oid,
                stashOnDirty: true,
                stashMessage: buildWipStashMessage(
                  currentBranch,
                  new Date(Date.now() - new Date().getTimezoneOffset() * 60_000),
                ),
              },
            });
            if (!this.destroyed) {
              this.recordBranchSwitchOutcome(result);
              await this.refreshAfterBranchSwitch();
            }
          } catch (retryError) {
            if (!this.destroyed) {
              this.navigationActionError.set(this.errorMessage(retryError, 'The branch could not be switched after stashing.'));
              await this.refreshAfterBranchSwitch();
            }
          }
        }
      } else if (!this.destroyed) {
        this.navigationActionError.set(message);
      }
    } finally {
      if (!this.destroyed) {
        this.switchingBranch.set(null);
      }
    }
  }

  private recordBranchSwitchOutcome(result: SwitchRepositoryBranchResponse): void {
    const operationStage = result.operationSucceeded
      ? `Operation: switched to “${result.name}”.`
      : `Operation: switch failed — ${result.operationError ?? 'the branch switch did not complete'}.`;
    const restoreStage = result.autoStash.restore === 'applied'
      ? 'Restore: auto-stashed changes were applied.'
      : result.autoStash.restore === 'conflicted'
        ? `Restore: changes were applied with conflicts${result.autoStash.restoreError ? ` — ${result.autoStash.restoreError}` : ''}.`
        : result.autoStash.restore === 'notRequired'
          ? 'Restore: not required.'
          : `Restore: ${result.autoStash.restore}${result.autoStash.restoreError ? ` — ${result.autoStash.restoreError}` : ''}.`;
    const cleanupStage = result.autoStash.cleanup === 'dropped'
      ? 'Cleanup: auto-stash dropped.'
      : result.autoStash.cleanup === 'retained'
        ? `Cleanup: auto-stash${result.autoStash.stash ? ` ${result.autoStash.stash.selector}` : ''} retained${result.autoStash.cleanupError ? ` — ${result.autoStash.cleanupError}` : ''}.`
        : result.autoStash.cleanup === 'notRequired'
          ? 'Cleanup: not required.'
          : `Cleanup: failed; the stash may remain available${result.autoStash.cleanupError ? ` — ${result.autoStash.cleanupError}` : ''}.`;
    const createFailed =
      result.autoStash.create === 'failed' || result.autoStash.create === 'partial';
    const hasPartialOutcome =
      !result.operationSucceeded ||
      createFailed ||
      result.autoStash.restore === 'conflicted' ||
      result.autoStash.restore === 'failed' ||
      result.autoStash.restore === 'skippedUnsafe' ||
      result.autoStash.cleanup === 'retained' ||
      result.autoStash.cleanup === 'failed';
    const creationStage = createFailed
      ? ` Auto-stash: ${result.autoStash.create}${result.autoStash.createError ? ` — ${result.autoStash.createError}` : ''}.`
      : '';
    const message = `${operationStage}${creationStage} ${restoreStage} ${cleanupStage}`;
    if (hasPartialOutcome) {
      this.navigationActionError.set(message);
      this.navigationActionNotice.set('');
    } else {
      this.navigationActionNotice.set(message);
    }
  }

  private async refreshAfterBranchSwitch(): Promise<void> {
    await Promise.all([this.statusStore.refresh(), this.reloadHistory(), this.loadNavigation()]);
  }

  protected async deleteBranch(branch: RepositoryBranch): Promise<void> {
    if (branch.kind !== 'local' || branch.current || this.workspaceActionBusy()) {
      return;
    }
    if (!globalThis.confirm(`Delete local branch “${branch.name}”? This cannot be undone.`)) {
      return;
    }
    this.navigationActionError.set('');
    this.deletingBranch.set(branch.fullName);
    try {
      await this.ipc.invoke('delete_repository_branch', {
        repositoryId: this.repositoryId,
        fullName: branch.fullName,
        expectedOid: branch.oid,
      });
      if (!this.destroyed) {
        await this.loadNavigation();
      }
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The local branch could not be deleted.'));
      }
    } finally {
      if (!this.destroyed) {
        this.deletingBranch.set(null);
      }
    }
  }

  protected worktreeRemovalDisabledReason(worktree: RepositoryWorktree): string | null {
    if (this.isCurrentWorktree(worktree)) {
      return 'current';
    }
    if (worktree.bare) {
      return 'bare';
    }
    if (worktree.locked) {
      return 'locked';
    }
    return null;
  }

  protected async removeWorktree(worktree: RepositoryWorktree): Promise<void> {
    if (this.workspaceActionBusy() || this.worktreeRemovalDisabledReason(worktree) !== null) {
      return;
    }
    const branchFullName = this.worktreeBranchFullName(worktree);
    const branchWarning = branchFullName === null
      ? ''
      : `\n\nThe associated branch “${this.worktreeBranchLabel(worktree)}” will also be deleted.`;
    if (!globalThis.confirm(`Remove worktree “${worktree.path}”?${branchWarning}\n\nUnmerged commits may be lost. This cannot be undone.`)) {
      return;
    }
    this.navigationActionError.set('');
    this.removingWorktree.set(worktree.path);
    try {
      const result = await this.ipc.invoke('remove_repository_worktree', {
        repositoryId: this.repositoryId,
        path: worktree.path,
        expectedHead: worktree.head,
        branchFullName,
      });
      if (!this.destroyed) {
        if (result.branchDeletionError !== null) {
          this.navigationActionError.set(
            `The worktree was removed, but its branch was kept: ${result.branchDeletionError}`,
          );
        }
        await this.loadNavigation();
      }
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The worktree could not be removed.'));
        await this.loadNavigation();
      }
    } finally {
      if (!this.destroyed) {
        this.removingWorktree.set(null);
      }
    }
  }

  protected async openWorktree(worktree: RepositoryWorktree): Promise<void> {
    if (this.workspaceActionBusy() || this.worktreeDisabledReason(worktree) !== null) {
      return;
    }
    this.navigationActionError.set('');
    this.openingWorktree.set(worktree.path);
    try {
      const repository = await this.catalog.rememberPath(worktree.path);
      if (this.destroyed) {
        return;
      }
      if (repository.id !== this.repositoryId) {
        await this.router.navigateByUrl('/repositories', { skipLocationChange: true });
      }
      await this.router.navigate(['/workspace', repository.id, 'history']);
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The worktree could not be opened.'));
      }
    } finally {
      if (!this.destroyed) {
        this.openingWorktree.set(null);
      }
    }
  }

  protected startSidebarResize(event: PointerEvent): void {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture can be unavailable in tests and older embedded webviews.
    }
    this.activeResizePointer = event.pointerId;
    this.resizingSidebar.set(true);
    globalThis.addEventListener('pointermove', this.resizeSidebar);
    globalThis.addEventListener('pointerup', this.finishSidebarResize);
    globalThis.addEventListener('pointercancel', this.finishSidebarResize);
    globalThis.addEventListener('blur', this.cancelSidebarResize);
  }

  protected resizeSidebarWithKeyboard(event: KeyboardEvent): void {
    let width: number | null = null;
    if (event.key === 'ArrowLeft') {
      width = this.sidebarWidth() - SIDEBAR_KEYBOARD_STEP;
    } else if (event.key === 'ArrowRight') {
      width = this.sidebarWidth() + SIDEBAR_KEYBOARD_STEP;
    } else if (event.key === 'Home') {
      width = SIDEBAR_MIN_WIDTH;
    } else if (event.key === 'End') {
      width = this.maximumSidebarWidth();
    }
    if (width === null) {
      return;
    }
    event.preventDefault();
    this.setSidebarWidth(width, true);
  }

  protected sidebarMaximumWidth(): number {
    return this.maximumSidebarWidth();
  }

  protected worktreeDisabledReason(worktree: RepositoryWorktree): string | null {
    if (this.isCurrentWorktree(worktree)) {
      return 'current';
    }
    if (worktree.bare) {
      return 'bare';
    }
    if (worktree.prunable) {
      return 'prunable';
    }
    return null;
  }

  protected worktreeBranchLabel(worktree: RepositoryWorktree): string {
    if (worktree.branch !== null) {
      return worktree.branch.replace(/^refs\/heads\//, '');
    }
    return worktree.detached ? 'detached HEAD' : worktree.path;
  }

  protected async loadNavigation(): Promise<void> {
    const generation = ++this.navigationRequestGeneration;
    this.navigationState.set({ kind: 'loading' });
    try {
      const navigation = await this.ipc.invoke('repository_navigation', {
        repositoryId: this.repositoryId,
      });
      if (generation === this.navigationRequestGeneration) {
        this.navigationState.set({ kind: 'ready', navigation });
        const localTree = buildBranchTree(
          navigation.branches.filter((branch) => branch.kind === 'local' && !branch.current),
        );
        const remoteTree = buildBranchTree(
          navigation.branches.filter((branch) => branch.kind === 'remote'),
        );
        this.expandedLocalBranchFolders.set(
          this.branchExpansionState.read(
            this.repositoryId,
            'local',
            branchFolderPaths(localTree),
          ),
        );
        this.expandedRemoteBranchFolders.set(
          this.branchExpansionState.read(
            this.repositoryId,
            'remote',
            branchFolderPaths(remoteTree),
          ),
        );
      }
    } catch {
      if (generation === this.navigationRequestGeneration) {
        this.navigationState.set({
          kind: 'error',
          message: 'Repository references could not be loaded.',
        });
      }
    }
  }

  protected async reloadHistory(): Promise<void> {
    const generation = ++this.historyRequestGeneration;
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.commits.set([]);
    this.nextCursor.set(null);
    this.isLoadingMore.set(false);
    this.selectedOid.set(null);
    this.historySelection.set('none');
    this.detailState.set({ kind: 'idle' });
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.historyError.set('');
    this.historyPhase.set('loading');

    try {
      const response = await this.ipc.invoke('repository_history', {
        repositoryId: this.repositoryId,
        cursor: null,
        limit: HISTORY_PAGE_SIZE,
      });
      if (generation !== this.historyRequestGeneration) {
        return;
      }
      this.commits.set(response.commits);
      this.nextCursor.set(response.nextCursor);
      this.historyPhase.set('ready');
    } catch {
      if (generation === this.historyRequestGeneration) {
        this.historyError.set('Commit history could not be loaded. Try again.');
        this.historyPhase.set('error');
      }
    }
  }

  protected async loadMore(): Promise<void> {
    const cursor = this.nextCursor();
    if (cursor === null || this.isLoadingMore()) {
      return;
    }

    const generation = ++this.historyRequestGeneration;
    this.isLoadingMore.set(true);
    this.historyError.set('');
    try {
      const response = await this.ipc.invoke('repository_history', {
        repositoryId: this.repositoryId,
        cursor,
        limit: HISTORY_PAGE_SIZE,
      });
      if (generation !== this.historyRequestGeneration) {
        return;
      }
      this.commits.update((commits) => [...commits, ...response.commits]);
      this.nextCursor.set(response.nextCursor);
    } catch {
      if (generation === this.historyRequestGeneration) {
        this.historyError.set('More commits could not be loaded. Try again.');
      }
    } finally {
      if (generation === this.historyRequestGeneration) {
        this.isLoadingMore.set(false);
      }
    }
  }

  protected async selectCommit(commit: RepositoryCommitSummary): Promise<void> {
    if (
      this.historySelection() === 'commit' &&
      this.selectedOid() === commit.oid &&
      this.detailState().kind === 'ready'
    ) {
      return;
    }

    const generation = ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.historySelection.set('commit');
    this.selectedOid.set(commit.oid);
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.detailState.set({ kind: 'loading' });
    try {
      const detail = await this.ipc.invoke('repository_commit_detail', {
        repositoryId: this.repositoryId,
        oid: commit.oid,
      });
      if (generation === this.detailRequestGeneration && this.selectedOid() === commit.oid) {
        this.detailState.set({ kind: 'ready', detail });
      }
    } catch (error) {
      if (generation === this.detailRequestGeneration && this.selectedOid() === commit.oid) {
        this.detailState.set({
          kind: 'error',
          message: this.errorMessage(
            error,
            'Commit details could not be loaded. Select the commit to retry.',
          ),
        });
      }
    }
  }

  protected selectWorkingTree(): void {
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.historySelection.set('working-tree');
    this.selectedOid.set(null);
    this.detailState.set({ kind: 'idle' });
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.fileDiffDisplayMode.set('contextual');
  }

  protected workingTreeFileIdentity(file: WorkingTreeFile): string {
    return workingTreeFileKey(file);
  }

  protected workingTreeFileSelected(file: WorkingTreeFile): boolean {
    return this.reconciledWorkingTreeSelection().has(this.workingTreeFileIdentity(file));
  }

  protected toggleWorkingTreeFile(file: WorkingTreeFile, selected: boolean): void {
    if (file.primaryStatus === 'conflicted' || this.workspaceActionBusy()) {
      return;
    }
    this.selectedWorkingTreeFiles.update((current) => {
      const next = new Set(
        reconcileWorkingTreeSelection(current, this.workingTreeSummary().files),
      );
      const identity = this.workingTreeFileIdentity(file);
      if (selected) {
        next.add(identity);
      } else {
        next.delete(identity);
      }
      return next;
    });
  }

  protected updateCommitMessage(message: string): void {
    this.commitMessage.set(message);
  }

  protected startAmend(trigger: HTMLElement): void {
    if (this.amendUnavailable()) {
      return;
    }
    this.amendReturnFocus = trigger;
    this.amendMode.set(true);
    this.workingTreeMutationError.set('');
    this.selectWorkingTree();
    this.focusAfterRender('commit-message');
  }

  protected cancelAmend(): void {
    if (this.workspaceActionBusy()) {
      return;
    }
    this.amendMode.set(false);
    this.commitMessage.set('');
    this.workingTreeMutationError.set('');
    this.restoreFocusAfterRender(this.amendReturnFocus);
    this.amendReturnFocus = null;
  }

  protected async applyIndexChange(action: IndexAction, all: boolean): Promise<void> {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }

    const plan = planWorkingTreeMutation(
      action,
      this.workingTreeSummary().files,
      all ? 'all' : this.reconciledWorkingTreeSelection(),
    );
    if (plan.actionableCount === 0 || plan.ambiguous) {
      return;
    }

    const selection: ChangeSelection =
      plan.selection.kind === 'all'
        ? { scope: 'all' }
        : {
            scope: 'selected',
            entries: plan.selection.entries,
          };

    this.workingTreeMutationError.set('');
    const generation = ++this.mutationRequestGeneration;
    this.workingTreeMutation.set({ action, scope: all ? 'all' : 'selected' });
    try {
      const result = await this.ipc.invoke('repository_apply_index_change', {
        repositoryId: this.repositoryId,
        operation: {
          action,
          selection,
          expectedHead: state.status.branch.oid,
          expectedHeadName: state.status.branch.head,
          expectedDetached: state.status.branch.detached,
          expectedUnborn: state.status.branch.unborn,
          expectedIndexFingerprint: state.status.indexFingerprint,
          expectedWorktreeFingerprint: state.status.worktreeFingerprint,
        },
      });
      if (!this.isCurrentMutation(generation)) {
        return;
      }
      if (result.status !== null) {
        this.statusStore.acceptMutationResult(result.status);
      }
      this.selectedWorkingTreeFiles.set(new Set());
      this.invalidateWorkingTreeDiff();
    } catch (error) {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutationError.set(
          this.errorMessage(
            error,
            action === 'stage'
              ? 'The selected changes could not be staged.'
              : 'The selected changes could not be unstaged.',
          ),
        );
        await this.statusStore.refresh();
      }
    } finally {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutation.set(null);
      }
    }
  }

  protected async createCommit(): Promise<void> {
    const state = this.statusStore.state();
    const message = this.commitMessage();
    if (state.kind !== 'ready' || this.commitDisabled()) {
      return;
    }

    this.workingTreeMutationError.set('');
    const generation = ++this.mutationRequestGeneration;
    this.workingTreeMutation.set({ action: 'commit', scope: null });
    try {
      const result = await this.ipc.invoke('repository_create_commit', {
        repositoryId: this.repositoryId,
        operation: {
          message,
          expectedHead: state.status.branch.oid,
          expectedHeadName: state.status.branch.head,
          expectedDetached: state.status.branch.detached,
          expectedUnborn: state.status.branch.unborn,
          expectedIndexFingerprint: state.status.indexFingerprint,
          expectedWorktreeFingerprint: state.status.worktreeFingerprint,
        },
      });
      if (!this.isCurrentMutation(generation)) {
        return;
      }
      this.statusStore.acceptMutationResult(result.status);
      this.commitMessage.set('');
      this.selectedWorkingTreeFiles.set(new Set());
      this.invalidateWorkingTreeDiff();
      await Promise.all([this.reloadHistory(), this.loadNavigation()]);
    } catch (error) {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutationError.set(
          this.errorMessage(error, 'The commit could not be created.'),
        );
        await this.statusStore.refresh();
      }
    } finally {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutation.set(null);
      }
    }
  }

  protected async amendCommit(message: string | null): Promise<void> {
    const state = this.statusStore.state();
    if (
      state.kind !== 'ready' ||
      !this.amendMode() ||
      this.amendUnavailable() ||
      (message !== null && message.trim().length === 0)
    ) {
      return;
    }

    const rewritesUpstreamCommit = this.amendRewritesUpstream();
    if (
      rewritesUpstreamCommit &&
      !globalThis.confirm(
        'HEAD is already part of the upstream history. Amending it rewrites published history and the next push may require force. Continue?',
      )
    ) {
      return;
    }

    this.workingTreeMutationError.set('');
    const generation = ++this.mutationRequestGeneration;
    this.workingTreeMutation.set({ action: 'amend', scope: null });
    try {
      const result = await this.ipc.invoke('repository_amend_commit', {
        repositoryId: this.repositoryId,
        operation: {
          message,
          confirmUpstreamRewrite: rewritesUpstreamCommit,
          expectedHead: state.status.branch.oid,
          expectedHeadName: state.status.branch.head,
          expectedDetached: state.status.branch.detached,
          expectedUnborn: state.status.branch.unborn,
          expectedIndexFingerprint: state.status.indexFingerprint,
          expectedWorktreeFingerprint: state.status.worktreeFingerprint,
        },
      });
      if (!this.isCurrentMutation(generation)) {
        return;
      }
      if (result.state === 'succeeded' && result.status !== null) {
        this.statusStore.acceptMutationResult(result.status);
        this.amendMode.set(false);
        this.commitMessage.set('');
        this.selectedWorkingTreeFiles.set(new Set());
        this.invalidateWorkingTreeDiff();
        await Promise.all([this.reloadHistory(), this.loadNavigation()]);
        this.restoreFocusAfterRender(this.amendReturnFocus);
        this.amendReturnFocus = null;
      } else {
        this.workingTreeMutationError.set(
          `The amend outcome is unknown. ${result.errorMessage ?? 'Inspect HEAD and the working tree before deciding whether to retry.'}`,
        );
        await Promise.all([
          this.statusStore.refresh(),
          this.reloadHistory(),
          this.loadNavigation(),
        ]);
        this.selectWorkingTree();
        this.focusAfterRender('commit-message');
      }
    } catch (error) {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutationError.set(
          this.errorMessage(error, 'HEAD could not be amended.'),
        );
        await this.statusStore.refresh();
      }
    } finally {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutation.set(null);
      }
    }
  }

  protected mutationInProgress(action: WorkingTreeMutation['action'], scope?: 'selected' | 'all'): boolean {
    const mutation = this.workingTreeMutation();
    return mutation?.action === action && (scope === undefined || mutation.scope === scope);
  }

  protected indexActionTitle(ambiguous: boolean): string {
    if (this.workingTreeCapabilities().conflictedCount > 0) {
      return 'Resolve all conflicts before changing the index.';
    }
    return ambiguous ? 'Select every entry sharing the same path before changing the index.' : '';
  }

  private async restoreStash(action: 'apply' | 'pop', stash: RepositoryStash): Promise<void> {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.stashMutation.set(`${action}:${stash.oid}`);
    const operation = {
      stash: { oid: stash.oid, selector: stash.selector },
      restoreIndex: true,
      precondition: this.repositoryStatePrecondition(state.status),
    };
    try {
      const result = action === 'apply'
        ? await this.ipc.invoke('repository_apply_stash', {
            repositoryId: this.repositoryId,
            operation,
          })
        : await this.ipc.invoke('repository_pop_stash', {
            repositoryId: this.repositoryId,
            operation,
          });
      if (this.destroyed) {
        return;
      }
      if (result.status !== null) {
        this.statusStore.acceptMutationResult(result.status);
      }
      const restoreError = 'restoreError' in result ? result.restoreError : result.errorMessage;
      const cleanupError = 'cleanupError' in result ? result.cleanupError : null;
      const restoreStage = result.restore === 'applied'
        ? 'Restore: changes applied.'
        : result.restore === 'conflicted'
          ? `Restore: changes applied with conflicts${restoreError ? ` — ${restoreError}` : ''}.`
          : `Restore: ${result.restore}${restoreError ? ` — ${restoreError}` : ''}.`;
      const cleanupStage = result.cleanup === 'dropped'
        ? 'Cleanup: stash dropped.'
        : result.cleanup === 'retained'
          ? `Cleanup: changes applied but stash retained${cleanupError ? ` — ${cleanupError}` : ''}.`
          : result.cleanup === 'notRequired'
            ? action === 'apply'
              ? 'Cleanup: not requested; stash retained.'
              : 'Cleanup: not required.'
            : `Cleanup: failed; changes may already be applied and the stash may still be present${cleanupError ? ` — ${cleanupError}` : ''}.`;
      const applyVerified =
        action === 'apply' &&
        result.restore === 'applied' &&
        result.status !== null &&
        restoreError === null &&
        result.cleanup !== 'failed';
      const popVerified =
        action === 'pop' &&
        result.restore === 'applied' &&
        result.cleanup === 'dropped' &&
        result.status !== null &&
        restoreError === null &&
        cleanupError === null;
      const verified = applyVerified || popVerified;
      const recoveryStage = !verified && result.mutationMayHaveOccurred
        ? ' Recovery: a mutating command was attempted; inspect the refreshed working tree and stash list before retrying.'
        : '';
      const message = `${stash.selector}. ${restoreStage} ${cleanupStage}${recoveryStage}`;
      if (!verified) {
        this.navigationActionError.set(message);
      } else {
        this.navigationActionNotice.set(message);
      }
      this.clearWorkingTreeMutationView();
      await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(
          this.errorMessage(error, `${stash.selector} could not be ${action === 'pop' ? 'popped' : 'applied'}.`),
        );
        await Promise.all([this.statusStore.refresh(), this.loadNavigation()]);
      }
    } finally {
      if (!this.destroyed) {
        this.stashMutation.set(null);
      }
    }
  }

  private repositoryStatePrecondition(status: RepositoryStatusResponse) {
    return {
      expectedHead: status.branch.oid,
      expectedHeadName: status.branch.head,
      expectedDetached: status.branch.detached,
      expectedUnborn: status.branch.unborn,
      expectedIndexFingerprint: status.indexFingerprint,
      expectedWorktreeFingerprint: status.worktreeFingerprint,
    };
  }

  private clearWorkingTreeMutationView(): void {
    this.selectedWorkingTreeFiles.set(new Set());
    this.invalidateWorkingTreeDiff();
  }

  private focusAfterRender(id: string): void {
    globalThis.setTimeout(() => {
      if (!this.destroyed) {
        globalThis.document?.getElementById(id)?.focus();
      }
    }, 0);
  }

  private restoreFocusAfterRender(target: HTMLElement | null): void {
    globalThis.setTimeout(() => {
      if (!this.destroyed && target?.isConnected) {
        target.focus();
      }
    }, 0);
  }

  private isCurrentMutation(generation: number): boolean {
    return !this.destroyed && generation === this.mutationRequestGeneration;
  }

  private invalidateWorkingTreeDiff(): void {
    ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.fileDiffDisplayMode.set('contextual');
    this.fileDiffReturnFocus = null;
  }

  protected async openFileDiff(file: CommitChangedFile): Promise<void> {
    this.fileDiffReturnFocus =
      globalThis.document?.activeElement instanceof HTMLElement
        ? globalThis.document.activeElement
        : null;
    await this.loadFileDiff(file.path, file.oldPath);
  }

  protected async openWorkingTreeFileDiff(file: WorkingTreeFile): Promise<void> {
    this.fileDiffReturnFocus =
      globalThis.document?.activeElement instanceof HTMLElement
        ? globalThis.document.activeElement
        : null;
    await this.loadWorkingTreeFileDiff(file.path, file.oldPath, file.entryKind);
  }

  protected async retryFileDiff(path: string, oldPath: string | null): Promise<void> {
    if (this.historySelection() === 'working-tree') {
      const entryKind = this.selectedWorkingTreeEntryKind();
      if (entryKind !== null) {
        await this.loadWorkingTreeFileDiff(path, oldPath, entryKind);
      }
    } else {
      await this.loadFileDiff(path, oldPath);
    }
  }

  private async loadWorkingTreeFileDiff(
    path: string,
    oldPath: string | null,
    entryKind: WorkingTreeFile['entryKind'],
  ): Promise<void> {
    if (this.historySelection() !== 'working-tree') {
      return;
    }

    const generation = ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(path);
    this.selectedWorkingTreeEntryKind.set(entryKind);
    this.fileDiffDisplayMode.set('contextual');
    this.fileDiffState.set({ kind: 'loading', path, oldPath });
    try {
      const response = await this.ipc.invoke('repository_working_tree_file_diff', {
        repositoryId: this.repositoryId,
        path,
        oldPath,
        entryKind,
      });
      if (
        generation === this.fileDiffRequestGeneration &&
        this.historySelection() === 'working-tree' &&
        this.selectedFilePath() === path
      ) {
        this.fileDiffState.set({ kind: 'ready', response });
      }
    } catch (error) {
      if (
        generation === this.fileDiffRequestGeneration &&
        this.historySelection() === 'working-tree' &&
        this.selectedFilePath() === path
      ) {
        this.fileDiffState.set({
          kind: 'error',
          path,
          oldPath,
          message: this.errorMessage(error, 'The working tree diff could not be loaded.'),
        });
      }
    }
  }

  private async loadFileDiff(path: string, oldPath: string | null): Promise<void> {
    const oid = this.selectedOid();
    if (oid === null) {
      return;
    }

    const generation = ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(path);
    this.selectedWorkingTreeEntryKind.set(null);
    this.fileDiffDisplayMode.set('contextual');
    this.fileDiffState.set({ kind: 'loading', path, oldPath });
    try {
      const response = await this.ipc.invoke('repository_file_diff', {
        repositoryId: this.repositoryId,
        oid,
        path,
        oldPath,
      });
      if (
        generation === this.fileDiffRequestGeneration &&
        this.selectedOid() === oid &&
        this.selectedFilePath() === path
      ) {
        this.fileDiffState.set({ kind: 'ready', response });
      }
    } catch (error) {
      if (
        generation === this.fileDiffRequestGeneration &&
        this.selectedOid() === oid &&
        this.selectedFilePath() === path
      ) {
        this.fileDiffState.set({
          kind: 'error',
          path,
          oldPath,
          message: this.errorMessage(error, 'The file diff could not be loaded.'),
        });
      }
    }
  }

  protected closeFileDiff(): void {
    ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.fileDiffDisplayMode.set('contextual');
    const focusTarget = this.fileDiffReturnFocus;
    this.fileDiffReturnFocus = null;
    globalThis.queueMicrotask(() => focusTarget?.focus());
  }

  protected setFileDiffDisplayMode(mode: FileDiffDisplayMode): void {
    this.fileDiffDisplayMode.set(mode);
  }

  protected formatTimestamp(timestamp: string): string {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(timestamp));
  }

  protected shortOid(oid: string): string {
    return oid.slice(0, 7);
  }

  protected fileStatusLabel(file: CommitChangedFile): string {
    return file.binary ? `${file.status}, binary` : file.status;
  }

  protected fileStatusMark(file: CommitChangedFile): string {
    const marks: Record<CommitChangedFile['status'], string> = {
      added: '+',
      modified: '●',
      deleted: '−',
      renamed: '→',
      copied: '⧉',
      typeChanged: 'T',
      unmerged: '!',
      unknown: '?',
    };
    return marks[file.status];
  }

  protected workingTreeStatusMark(status: WorkingTreePrimaryStatus): string {
    const marks: Record<WorkingTreePrimaryStatus, string> = {
      added: '+',
      modified: '●',
      renamed: '→',
      deleted: '−',
      conflicted: '!',
    };
    return marks[status];
  }

  private branchesOfKind(kind: RepositoryBranch['kind']): readonly RepositoryBranch[] {
    const state = this.navigationState();
    return state.kind === 'ready'
      ? state.navigation.branches.filter((branch) => branch.kind === kind)
      : [];
  }

  private collapsedFoldersFor(kind: RepositoryBranch['kind']): ReadonlySet<string> {
    const expanded = this.expandedFoldersFor(kind)();
    return new Set([...this.availableFolderPathsFor(kind)].filter((path) => !expanded.has(path)));
  }

  private expandedFoldersFor(kind: BranchExpansionScope) {
    return kind === 'local'
      ? this.expandedLocalBranchFolders
      : this.expandedRemoteBranchFolders;
  }

  private availableFolderPathsFor(kind: BranchExpansionScope): ReadonlySet<string> {
    return kind === 'local'
      ? this.localBranchFolderPaths()
      : this.remoteBranchFolderPaths();
  }

  private readonly resizeSidebar = (event: PointerEvent): void => {
    if (event.pointerId === this.activeResizePointer) {
      this.setSidebarWidth(event.clientX, false);
    }
  };

  private readonly finishSidebarResize = (event: PointerEvent): void => {
    if (event.pointerId !== this.activeResizePointer) {
      return;
    }
    this.persistSidebarWidth();
    this.stopSidebarResize();
  };

  private readonly cancelSidebarResize = (): void => {
    if (this.activeResizePointer !== null) {
      this.persistSidebarWidth();
      this.stopSidebarResize();
    }
  };

  private readonly clampSidebarToViewport = (): void => {
    this.setSidebarWidth(this.sidebarWidth(), false);
  };

  private stopSidebarResize(): void {
    this.activeResizePointer = null;
    this.resizingSidebar.set(false);
    globalThis.removeEventListener('pointermove', this.resizeSidebar);
    globalThis.removeEventListener('pointerup', this.finishSidebarResize);
    globalThis.removeEventListener('pointercancel', this.finishSidebarResize);
    globalThis.removeEventListener('blur', this.cancelSidebarResize);
  }

  private setSidebarWidth(width: number, persist: boolean): void {
    this.sidebarWidth.set(Math.min(this.maximumSidebarWidth(), Math.max(SIDEBAR_MIN_WIDTH, width)));
    if (persist) {
      this.persistSidebarWidth();
    }
  }

  private maximumSidebarWidth(): number {
    const viewportWidth = globalThis.innerWidth;
    let reservedWidth = 0;
    if (viewportWidth > INSPECTOR_LAYOUT_BREAKPOINT) {
      reservedWidth = HISTORY_MIN_WIDTH + INSPECTOR_MIN_WIDTH + SIDEBAR_RESIZER_WIDTH;
    } else if (viewportWidth > COMPACT_LAYOUT_BREAKPOINT) {
      reservedWidth = HISTORY_MIN_WIDTH + SIDEBAR_RESIZER_WIDTH;
    }
    const availableWidth = viewportWidth - reservedWidth;
    return Math.max(
      SIDEBAR_MIN_WIDTH,
      Math.floor(Math.min(viewportWidth * 0.5, availableWidth)),
    );
  }

  private isCurrentWorktree(worktree: RepositoryWorktree): boolean {
    return this.repository()?.path === worktree.path;
  }

  private worktreeBranchFullName(worktree: RepositoryWorktree): string | null {
    if (worktree.branch === null) {
      return null;
    }
    return worktree.branch.startsWith('refs/heads/')
      ? worktree.branch
      : `refs/heads/${worktree.branch}`;
  }

  private configureAutoFetchTimer(): void {
    this.clearAutoFetchTimer();
    if (!this.autoFetch() || this.destroyed) {
      return;
    }
    this.autoFetchTimer = globalThis.setInterval(() => {
      if (!this.workspaceActionBusy()) {
        void this.fetchRepository();
      }
    }, AUTO_FETCH_INTERVAL_MS);
  }

  private configureLiveChangesTimer(): void {
    this.clearLiveChangesTimer();
    if (!this.liveChanges() || this.destroyed) {
      return;
    }
    this.liveChangesTimer = globalThis.setInterval(() => {
      if (!this.workspaceActionBusy()) {
        void this.refreshLiveChanges();
      }
    }, LIVE_STATUS_INTERVAL_MS);
  }

  private async refreshLiveChanges(): Promise<void> {
    if (this.liveRefreshInFlight || this.destroyed) {
      return;
    }
    this.liveRefreshInFlight = true;
    const before = this.statusStore.state();
    const beforeFingerprint = before.kind === 'ready' ? before.status.worktreeFingerprint : null;
    try {
      await this.statusStore.refresh({ silent: true });
      if (this.destroyed) {
        return;
      }
      const after = this.statusStore.state();
      const afterFingerprint = after.kind === 'ready' ? after.status.worktreeFingerprint : null;
      if (
        this.historySelection() === 'working-tree' &&
        beforeFingerprint !== null &&
        afterFingerprint !== beforeFingerprint
      ) {
        this.invalidateWorkingTreeDiff();
        if (after.kind === 'ready' && after.status.entries.length === 0) {
          this.historySelection.set('none');
        }
      }
    } finally {
      this.liveRefreshInFlight = false;
    }
  }

  private clearAutoFetchTimer(): void {
    if (this.autoFetchTimer !== null) {
      globalThis.clearInterval(this.autoFetchTimer);
      this.autoFetchTimer = null;
    }
  }

  private clearLiveChangesTimer(): void {
    if (this.liveChangesTimer !== null) {
      globalThis.clearInterval(this.liveChangesTimer);
      this.liveChangesTimer = null;
    }
  }

  private readSidebarWidth(): number {
    try {
      const stored = Number(globalThis.localStorage?.getItem(SIDEBAR_WIDTH_KEY));
      return Number.isFinite(stored) && stored > 0
        ? Math.min(this.maximumSidebarWidth(), Math.max(SIDEBAR_MIN_WIDTH, stored))
        : SIDEBAR_DEFAULT_WIDTH;
    } catch {
      return SIDEBAR_DEFAULT_WIDTH;
    }
  }

  private persistSidebarWidth(): void {
    try {
      globalThis.localStorage?.setItem(SIDEBAR_WIDTH_KEY, String(this.sidebarWidth()));
    } catch {
      // Storage may be unavailable in a hardened webview. Resizing remains usable for the session.
    }
  }

  private errorMessage(error: unknown, fallback: string): string {
    if (typeof error === 'string' && error.trim() !== '') {
      return error;
    }
    if (
      typeof error === 'object' &&
      error !== null &&
      'message' in error &&
      typeof error.message === 'string' &&
      error.message.trim() !== ''
    ) {
      return error.message;
    }
    return fallback;
  }

  private async loadRepository(): Promise<void> {
    await this.catalog.load();
    if (this.destroyed) {
      return;
    }
    const repository = this.catalog.find(this.repositoryId);
    if (repository === undefined) {
      this.historyError.set('This remembered repository is no longer available.');
      this.historyPhase.set('error');
      return;
    }

    this.statusStore.setRepositoryPath(repository.path);
    await Promise.all([
      this.statusStore.refresh(),
      this.reloadHistory(),
      this.loadNavigation(),
    ]);
  }
}
