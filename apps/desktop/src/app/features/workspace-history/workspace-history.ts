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
  type RepositoryWorktree,
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
  | { readonly action: 'commit'; readonly scope: null };
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
  protected readonly currentOnly = signal(this.initialRefreshPreferences.currentOnly);
  protected readonly autoFetch = signal(this.initialRefreshPreferences.autoFetch);
  protected readonly liveChanges = signal(this.initialRefreshPreferences.liveChanges);
  protected readonly fetchingRepository = signal(false);
  protected readonly deletingBranch = signal<string | null>(null);
  protected readonly removingWorktree = signal<string | null>(null);
  protected readonly selectedFilePath = signal<string | null>(null);
  protected readonly selectedWorkingTreeEntryKind = signal<WorkingTreeFile['entryKind'] | null>(null);
  protected readonly fileDiffState = signal<FileDiffState>({ kind: 'idle' });
  protected readonly fileDiffDisplayMode = signal<FileDiffDisplayMode>('contextual');
  protected readonly selectedWorkingTreeFiles = signal<ReadonlySet<string>>(new Set());
  protected readonly commitMessage = signal('');
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
      this.removingWorktree() !== null,
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
    this.switchingBranch.set(branch.fullName);
    try {
      await this.ipc.invoke('switch_repository_branch', {
        repositoryId: this.repositoryId,
        operation: {
          fullName: branch.fullName,
          stashOnDirty: false,
          stashMessage: null,
        },
      });
      if (this.destroyed) {
        return;
      }
      await Promise.all([
        this.statusStore.refresh(),
        this.reloadHistory(),
        this.loadNavigation(),
      ]);
    } catch (error) {
      const message = this.errorMessage(error, 'The branch could not be switched.');
      if (!this.destroyed && message.includes('dirtyWorkingTree')) {
        const currentBranch = this.currentLocalBranch()?.name ?? this.branchName();
        if (globalThis.confirm(`The working tree has uncommitted changes. Stash them and switch to “${branch.name}”?`)) {
          try {
            await this.ipc.invoke('switch_repository_branch', {
              repositoryId: this.repositoryId,
              operation: {
                fullName: branch.fullName,
                stashOnDirty: true,
                stashMessage: buildWipStashMessage(
                  currentBranch,
                  new Date(Date.now() - new Date().getTimezoneOffset() * 60_000),
                ),
              },
            });
            if (!this.destroyed) {
              await Promise.all([this.statusStore.refresh(), this.reloadHistory(), this.loadNavigation()]);
            }
          } catch (retryError) {
            if (!this.destroyed) {
              this.navigationActionError.set(this.errorMessage(retryError, 'The branch could not be switched after stashing.'));
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
    } catch {
      if (generation === this.detailRequestGeneration && this.selectedOid() === commit.oid) {
        this.detailState.set({
          kind: 'error',
          message: 'Commit details could not be loaded. Select the commit to retry.',
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
      this.statusStore.acceptMutationResult(result.status);
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
