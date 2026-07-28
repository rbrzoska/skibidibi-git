import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  type OnDestroy,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';

import {
  DESKTOP_IPC,
  type CommitChangedFile,
  type BranchCreationSource,
  type BranchWorkspaceOpenTarget,
  type ChangeSelection,
  type ConflictFileDetailResponse,
  type ConflictFileSummary,
  type ConflictResolution,
  type IndexAction,
  type MergeRepositoryBranchResponse,
  type PullInactiveBranchResponse,
  type PullRepositoryResponse,
  type PullStrategy,
  type PushAnalysisResponse,
  type ResetMode,
  type RepositoryCommitOperationResponse,
  type RepositoryCommitDetailResponse,
  type RepositoryFileDiffResponse,
  type RepositoryHistoryResponse,
  type RepositoryCommitSummary,
  type RepositoryBranch,
  type RepositoryNavigationResponse,
  type RepositoryStashDetailResponse,
  type RepositoryStatusResponse,
  type RepositoryStash,
  type RepositorySubmodule,
  type RepositoryWorktree,
  type StashChangedFile,
  type StashFileSource,
  type SwitchRepositoryBranchResponse,
  type AiCliProvider,
  type WorktreeDirtyStateResponse,
  type WorktreeDirtyStatesResponse,
  type WorktreeRemovalMode,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { AiSupportStore } from '../../core/ai-support/ai-support.store';
import { CommanderContextStore } from '../../core/commander/commander-context';
import { UiFeedback } from '../../core/ui-feedback/ui-feedback';
import {
  GitHubAccountStore,
  GitHubRepositoryPullRequestStore,
  type GitHubAccount,
} from '../../core/github';
import { AppIcon } from '../../shared/app-icon/app-icon';
import { SkibiBot } from '../../shared/skibi-bot/skibi-bot';
import { GitHubPullRequestInspector } from '../github/pull-request-inspector/github-pull-request-inspector';
import { GitHubPullRequestList } from '../github/pull-request-list/github-pull-request-list';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import {
  BranchExpansionState,
  branchFolderPaths,
  type BranchExpansionScope,
} from './branch-expansion-state';
import {
  buildBranchTree,
  visibleBranchTree,
  type BranchTreeNode,
  type VisibleBranchTreeNode,
} from './branch-tree';
import { createContextualDiffRows } from './contextual-diff';
import { splitFilePath } from './file-path-parts';
import { parseUnifiedDiff } from './unified-diff';
import {
  planWorkingTreeMutation,
  reconcileWorkingTreeSelection,
  workingTreeActionCapabilities,
  workingTreeEntrySelector,
  workingTreeFileKey,
} from './working-tree-actions';
import {
  createWorkingTreeSummary,
  type WorkingTreeFile,
  type WorkingTreePrimaryStatus,
} from './working-tree-summary';
import {
  readReleaseBranch,
  writeReleaseBranch,
  type ReleaseBranchStorage,
} from './release-branch-state';
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
type HistorySelection = 'none' | 'working-tree' | 'commit' | 'stash' | 'pull-request';
type WorkingTreeMutation =
  | { readonly action: IndexAction; readonly scope: 'selected' | 'all' }
  | { readonly action: 'discard'; readonly scope: 'selected' }
  | { readonly action: 'discardHunk' | 'commit' | 'amend'; readonly scope: null };
interface DiscardableDiffHunk {
  readonly key: string;
  readonly displayHunkKey: string;
  readonly label: string;
  readonly patch: string;
}
type BranchCreationTarget =
  | { readonly kind: 'current'; readonly label: string }
  | { readonly kind: 'commit'; readonly oid: string; readonly label: string };
type DeletionConfirmation =
  | { readonly kind: 'branch'; readonly branch: RepositoryBranch; readonly returnFocus: HTMLElement }
  | {
      readonly kind: 'worktree';
      readonly worktree: RepositoryWorktree;
      readonly branchFullName: string | null;
      readonly branchLabel: string | null;
      readonly returnFocus: HTMLElement;
    };
type DetailState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly detail: RepositoryCommitDetailResponse }
  | { readonly kind: 'error'; readonly message: string };
type StashDetailState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly detail: RepositoryStashDetailResponse }
  | { readonly kind: 'error'; readonly message: string };
type PushAnalysisState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly analysis: PushAnalysisResponse }
  | { readonly kind: 'error'; readonly message: string };
type ConflictListState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly files: readonly ConflictFileSummary[] }
  | { readonly kind: 'error'; readonly message: string };
type ConflictDetailState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly path: string }
  | { readonly kind: 'ready'; readonly detail: ConflictFileDetailResponse }
  | { readonly kind: 'error'; readonly path: string; readonly message: string };
type NavigationState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly navigation: RepositoryNavigationResponse }
  | { readonly kind: 'error'; readonly message: string };
type SubmoduleState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly submodules: readonly RepositorySubmodule[] }
  | { readonly kind: 'error'; readonly message: string };
type FileDiffState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly path: string; readonly oldPath: string | null }
  | {
      readonly kind: 'ready';
      readonly response: Pick<RepositoryFileDiffResponse, 'path' | 'patch' | 'binary' | 'truncated'> & {
        readonly unstagedPatch?: string;
      };
    }
  | { readonly kind: 'error'; readonly path: string; readonly oldPath: string | null; readonly message: string };
interface BranchContextMenu {
  readonly branch: RepositoryBranch;
  readonly x: number;
  readonly y: number;
  readonly returnFocus: HTMLElement;
}
interface BranchPreview {
  /** Immutable ref/OID pair captured when the user starts a read-only preview. */
  readonly branch: RepositoryBranch;
  /** The release branch when marked, otherwise main/master (or the preview itself). */
  readonly target: RepositoryBranch;
}
interface WorktreeContextMenu {
  readonly worktree: RepositoryWorktree;
  readonly x: number;
  readonly y: number;
  readonly returnFocus: HTMLElement;
}
interface MergeConfirmation {
  readonly source: RepositoryBranch;
  readonly target: RepositoryBranch;
  readonly switchRequired: boolean;
  readonly dirty: boolean;
  readonly returnFocus: HTMLElement;
}
interface SwitchConfirmation {
  readonly branch: RepositoryBranch;
  readonly dirty: boolean;
  readonly targetWorktree: RepositoryWorktree | null;
  readonly returnFocus: HTMLElement;
}
interface DestructiveActionConfirmation {
  readonly title: string;
  readonly description: string;
  readonly confirmLabel: string;
  readonly destructive: boolean;
  readonly returnFocus: HTMLElement | null;
  readonly confirm: () => Promise<void>;
  readonly cancel: (() => void) | null;
}
interface RemoteBranchGroup {
  readonly remote: string;
  readonly count: number;
  readonly collapsed: boolean;
  readonly tree: readonly VisibleBranchTreeNode[];
}

const HISTORY_PAGE_SIZE = 50;
const MAX_RENDERED_DIFF_ROWS = 20_000;
const SIDEBAR_WIDTH_KEY = 'skibidibi-git.workspace.sidebar-width.v2';
const INSPECTOR_WIDTH_KEY = 'skibidibi-git.workspace.inspector-width.v2';
const SIDEBAR_MIN_WIDTH = 220;
const SIDEBAR_DEFAULT_WIDTH = 250;
const SIDEBAR_KEYBOARD_STEP = 16;
const INSPECTOR_MIN_WIDTH = 272;
const INSPECTOR_DEFAULT_WIDTH = 296;
const INSPECTOR_KEYBOARD_STEP = 16;
const ROOT_FONT_SIZE = 16;
const COMPACT_LAYOUT_BREAKPOINT = 48 * ROOT_FONT_SIZE;
const INSPECTOR_LAYOUT_BREAKPOINT = 58 * ROOT_FONT_SIZE;
const HISTORY_MIN_WIDTH = 18 * ROOT_FONT_SIZE;
const SIDEBAR_RESIZER_WIDTH = 0.4 * ROOT_FONT_SIZE;

function remoteBranchParts(branch: RepositoryBranch): {
  readonly remote: string;
  readonly branch: RepositoryBranch;
} {
  const refName = branch.fullName.replace(/^refs\/remotes\//, '');
  const separator = refName.indexOf('/');
  const remote = separator > 0 ? refName.slice(0, separator) : 'remote';
  const name = separator > 0 ? refName.slice(separator + 1) : branch.name;
  return { remote, branch: { ...branch, name } };
}

function remoteBranchTrees(
  branches: readonly RepositoryBranch[],
): ReadonlyMap<string, readonly BranchTreeNode[]> {
  const grouped = new Map<string, RepositoryBranch[]>();
  for (const branch of branches) {
    const parts = remoteBranchParts(branch);
    grouped.set(parts.remote, [...(grouped.get(parts.remote) ?? []), parts.branch]);
  }
  return new Map(
    [...grouped.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([remote, remoteBranches]) => [remote, buildBranchTree(remoteBranches)]),
  );
}

function remoteFolderKey(remote: string, path: string): string {
  return `${remote}/${path}`;
}

function remoteFolderPaths(branches: readonly RepositoryBranch[]): ReadonlySet<string> {
  const paths = new Set<string>();
  for (const [remote, tree] of remoteBranchTrees(branches)) {
    paths.add(remote);
    for (const path of branchFolderPaths(tree)) {
      paths.add(remoteFolderKey(remote, path));
    }
  }
  return paths;
}

export function selectPreferredGitHubAccountId(
  accounts: readonly GitHubAccount[],
  host: string,
): string | null {
  const matching = accounts.filter(
    (account) => account.state === 'connected' && account.host.toLowerCase() === host.toLowerCase(),
  );
  return matching.find((account) => account.authKind === 'gitHubCli')?.id
    ?? matching.find((account) => account.authKind === 'oAuthDevice')?.id
    ?? matching.find((account) => account.authKind === 'personalAccessToken')?.id
    ?? null;
}

@Component({
  selector: 'app-workspace-history',
  imports: [AppIcon, GitHubPullRequestInspector, GitHubPullRequestList, RouterLink, SkibiBot],
  providers: [GitHubRepositoryPullRequestStore, RepositoryStatusStore],
  templateUrl: './workspace-history.html',
  styleUrls: ['./workspace-history.css', './workspace-history-discard.css'],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class WorkspaceHistory implements OnDestroy {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly catalog = inject(RepositoryCatalog);
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly aiSupport = inject(AiSupportStore);
  private readonly commanderContext = inject(CommanderContextStore);
  private readonly feedback = inject(UiFeedback);
  protected readonly githubAccounts = inject(GitHubAccountStore);
  protected readonly filePathParts = splitFilePath;
  private readonly branchExpansionState = new BranchExpansionState();
  private historyRequestGeneration = 0;
  private detailRequestGeneration = 0;
  private navigationRequestGeneration = 0;
  private fileDiffRequestGeneration = 0;
  private mutationRequestGeneration = 0;
  private pushAnalysisRequestGeneration = 0;
  private conflictListRequestGeneration = 0;
  private conflictDetailRequestGeneration = 0;
  private aiGenerationRequestGeneration = 0;
  private dirtyStatesRequestGeneration = 0;
  private submodulesRequestGeneration = 0;
  private destroyed = false;
  private fileDiffReturnFocus: HTMLElement | null = null;
  private activeResizePointer: number | null = null;
  private activeInspectorResizePointer: number | null = null;
  private autoFetchTimer: ReturnType<typeof globalThis.setInterval> | null = null;
  private liveChangesTimer: ReturnType<typeof globalThis.setInterval> | null = null;
  private liveRefreshInFlight = false;
  private branchCreationReturnFocus: HTMLElement | null = null;
  private amendReturnFocus: HTMLElement | null = null;
  private readonly releaseBranchStorage: ReleaseBranchStorage = globalThis.localStorage;

  protected readonly statusStore = inject(RepositoryStatusStore);
  protected readonly repositoryId = this.route.snapshot.paramMap.get('repositoryId') ?? '';
  protected readonly repository = computed(() => this.catalog.find(this.repositoryId));
  protected readonly githubAccountId = computed(() => {
    const state = this.githubAccounts.state();
    const host = this.repository()?.hostedIdentity?.host;
    if (state.kind !== 'ready' || host === undefined) {
      return null;
    }
    return selectPreferredGitHubAccountId(state.accounts, host);
  });
  private readonly refreshStorage = browserWorkspaceRefreshStorage();
  private readonly initialRefreshPreferences = readWorkspaceRefreshPreferences(
    this.refreshStorage,
    this.repositoryId,
  );
  protected readonly historyPhase = signal<HistoryPhase>('idle');
  protected readonly historyError = signal('');
  protected readonly commits = signal<readonly RepositoryCommitSummary[]>([]);
  protected readonly branchPreview = signal<BranchPreview | null>(null);
  protected readonly nextCursor = signal<string | null>(null);
  protected readonly isLoadingMore = signal(false);
  protected readonly selectedOid = signal<string | null>(null);
  protected readonly historySelection = signal<HistorySelection>('none');
  protected readonly detailState = signal<DetailState>({ kind: 'idle' });
  protected readonly stashDetailState = signal<StashDetailState>({ kind: 'idle' });
  protected readonly selectedStash = signal<RepositoryStash | null>(null);
  protected readonly navigationState = signal<NavigationState>({ kind: 'loading' });
  protected readonly submoduleState = signal<SubmoduleState>({ kind: 'loading' });
  protected readonly navigationFilter = signal('');
  protected readonly expandedLocalBranchFolders = signal<ReadonlySet<string>>(new Set());
  protected readonly expandedRemoteBranchFolders = signal<ReadonlySet<string>>(new Set());
  protected readonly sidebarWidth = signal(this.readSidebarWidth());
  protected readonly resizingSidebar = signal(false);
  protected readonly inspectorWidth = signal(this.readInspectorWidth());
  protected readonly resizingInspector = signal(false);
  protected readonly inspectorMaximized = signal(false);
  protected readonly switchingBranch = signal<string | null>(null);
  protected readonly openingWorktree = signal<string | null>(null);
  protected readonly openingBranchWorkspace = signal<string | null>(null);
  protected readonly openingSubmodule = signal<string | null>(null);
  protected readonly refreshingWorkspace = signal(false);
  protected readonly navigationActionError = signal('');
  protected readonly navigationActionNotice = signal('');
  protected readonly branchContextMenu = signal<BranchContextMenu | null>(null);
  protected readonly worktreeContextMenu = signal<WorktreeContextMenu | null>(null);
  protected readonly branchContextMutation = signal<string | null>(null);
  protected readonly mergeConfirmation = signal<MergeConfirmation | null>(null);
  protected readonly mergeAutoStash = signal(true);
  protected readonly switchConfirmation = signal<SwitchConfirmation | null>(null);
  protected readonly switchAutoStash = signal(true);
  protected readonly branchCreationTarget = signal<BranchCreationTarget | null>(null);
  protected readonly newBranchName = signal('');
  protected readonly creatingBranch = signal(false);
  protected readonly releaseBranchFullName = signal<string | null>(null);
  protected readonly worktreeDirtyStates = signal<ReadonlyMap<string, WorktreeDirtyStateResponse>>(new Map());
  protected readonly currentOnly = signal(this.initialRefreshPreferences.currentOnly);
  protected readonly autoFetch = signal(this.initialRefreshPreferences.autoFetch);
  protected readonly liveChanges = signal(this.initialRefreshPreferences.liveChanges);
  protected readonly fetchingRepository = signal(false);
  protected readonly deletingBranch = signal<string | null>(null);
  protected readonly removingWorktree = signal<string | null>(null);
  protected readonly deletionConfirmation = signal<DeletionConfirmation | null>(null);
  protected readonly destructiveActionConfirmation = signal<DestructiveActionConfirmation | null>(null);
  protected readonly worktreeRemovalMode = signal<WorktreeRemovalMode>('safe');
  protected readonly forceRemovalAcknowledged = signal(false);
  protected readonly worktreeRemovalStashMessage = signal<string | null>(null);
  protected readonly worktreeRemovalDialogError = signal('');
  private readonly deletionDialogElement = viewChild<ElementRef<HTMLDialogElement>>('referenceDeletionDialog');
  private readonly destructiveActionDialogElement = viewChild<ElementRef<HTMLDialogElement>>('destructiveActionDialog');
  private readonly mergeDialogElement = viewChild<ElementRef<HTMLDialogElement>>('referenceMergeDialog');
  private readonly switchDialogElement = viewChild<ElementRef<HTMLDialogElement>>('referenceSwitchDialog');
  private readonly branchCreationDialogElement = viewChild<ElementRef<HTMLDialogElement>>('referenceBranchCreationDialog');
  private readonly openDeletionDialog = effect(() => {
    if (this.deletionConfirmation() === null) {
      return;
    }
    const dialog = this.deletionDialogElement()?.nativeElement;
    if (dialog === undefined || dialog.open) {
      return;
    }
    globalThis.queueMicrotask(() => {
      if (!dialog.isConnected || dialog.open || this.deletionConfirmation() === null) {
        return;
      }
      try {
        dialog.showModal();
      } catch {
        // jsdom and older webviews may not implement the top-layer API.
        dialog.setAttribute('open', '');
      }
    });
  });
  private readonly openDestructiveActionDialog = effect(() => {
    if (this.destructiveActionConfirmation() === null) {
      return;
    }
    this.showDialogAfterRender(
      this.destructiveActionDialogElement()?.nativeElement,
      () => this.destructiveActionConfirmation() !== null,
    );
  });
  private readonly openMergeDialog = effect(() => {
    if (this.mergeConfirmation() === null) {
      return;
    }
    const dialog = this.mergeDialogElement()?.nativeElement;
    if (dialog === undefined || dialog.open) {
      return;
    }
    globalThis.queueMicrotask(() => {
      if (!dialog.isConnected || dialog.open || this.mergeConfirmation() === null) {
        return;
      }
      try {
        dialog.showModal();
      } catch {
        dialog.setAttribute('open', '');
      }
    });
  });
  private readonly openSwitchDialog = effect(() => {
    if (this.switchConfirmation() === null) {
      return;
    }
    this.showDialogAfterRender(this.switchDialogElement()?.nativeElement, () => this.switchConfirmation() !== null);
  });
  private readonly openBranchCreationDialog = effect(() => {
    if (this.branchCreationTarget() === null) {
      return;
    }
    this.showDialogAfterRender(
      this.branchCreationDialogElement()?.nativeElement,
      () => this.branchCreationTarget() !== null,
    );
  });
  protected readonly stashMessage = signal('');
  protected readonly stashIncludeUntracked = signal(true);
  protected readonly stashMutation = signal<string | null>(null);
  protected readonly stashCreationOpen = signal(false);
  private readonly stashCreationDialogElement = viewChild<ElementRef<HTMLDialogElement>>('stashCreationDialog');
  private readonly openStashCreationDialog = effect(() => {
    if (!this.stashCreationOpen()) {
      return;
    }
    this.showDialogAfterRender(
      this.stashCreationDialogElement()?.nativeElement,
      () => this.stashCreationOpen(),
    );
  });
  protected readonly pullStrategy = signal<PullStrategy>('ffIfPossible');
  protected readonly networkMutation = signal<'pull' | 'push' | 'setUpstream' | null>(null);
  protected readonly networkNotice = signal('');
  protected readonly networkError = signal('');
  protected readonly pushAnalysisState = signal<PushAnalysisState>({ kind: 'loading' });
  protected readonly conflictListState = signal<ConflictListState>({ kind: 'loading' });
  protected readonly selectedConflict = signal<ConflictFileSummary | null>(null);
  protected readonly conflictDetailState = signal<ConflictDetailState>({ kind: 'idle' });
  protected readonly conflictResult = signal('');
  protected readonly conflictMutation = signal(false);
  protected readonly conflictNotice = signal('');
  protected readonly conflictError = signal('');
  protected readonly selectedFilePath = signal<string | null>(null);
  protected readonly selectedWorkingTreeEntryKind = signal<WorkingTreeFile['entryKind'] | null>(null);
  protected readonly selectedStashFileSource = signal<StashFileSource | null>(null);
  protected readonly fileDiffState = signal<FileDiffState>({ kind: 'idle' });
  protected readonly fileDiffDisplayMode = signal<FileDiffDisplayMode>('contextual');
  protected readonly selectedWorkingTreeFiles = signal<ReadonlySet<string>>(new Set());
  protected readonly commitMessage = signal('');
  protected readonly amendMode = signal(false);
  protected readonly generatingCommitMessageWith = signal<string | null>(null);
  protected readonly commitMessageGenerationError = signal('');
  protected readonly workingTreeMutation = signal<WorkingTreeMutation | null>(null);
  protected readonly commitOperation = signal<'cherryPick' | 'revert' | 'reset' | null>(null);
  protected readonly resetMode = signal<ResetMode>('mixed');
  protected readonly workingTreeMutationError = signal('');
  protected readonly workspaceActionBusy = computed(
    () =>
      this.workingTreeMutation() !== null ||
      this.commitOperation() !== null ||
      this.switchingBranch() !== null ||
      this.openingWorktree() !== null ||
      this.openingBranchWorkspace() !== null ||
      this.openingSubmodule() !== null ||
      this.refreshingWorkspace() ||
      this.fetchingRepository() ||
      this.deletingBranch() !== null ||
      this.removingWorktree() !== null ||
      this.deletionConfirmation() !== null ||
      this.destructiveActionConfirmation() !== null ||
      this.mergeConfirmation() !== null ||
      this.switchConfirmation() !== null ||
      this.creatingBranch() ||
      this.stashMutation() !== null ||
      this.networkMutation() !== null ||
      this.conflictMutation() ||
      this.branchContextMutation() !== null ||
      this.generatingCommitMessageWith() !== null,
  );
  private readonly workspaceOperationLoading = computed(
    () =>
      this.navigationState().kind === 'loading' ||
      this.historyPhase() === 'loading' ||
      this.workingTreeMutation() !== null ||
      this.commitOperation() !== null ||
      this.switchingBranch() !== null ||
      this.openingWorktree() !== null ||
      this.openingBranchWorkspace() !== null ||
      this.openingSubmodule() !== null ||
      this.refreshingWorkspace() ||
      this.fetchingRepository() ||
      this.deletingBranch() !== null ||
      this.removingWorktree() !== null ||
      this.creatingBranch() ||
      this.stashMutation() !== null ||
      this.networkMutation() !== null ||
      this.conflictMutation() ||
      this.branchContextMutation() !== null ||
      this.generatingCommitMessageWith() !== null,
  );
  private readonly syncGlobalFeedback = effect(() => {
    const navigationError = this.navigationActionError();
    const navigationNotice = this.navigationActionNotice();
    const remoteError = this.networkError();
    const remoteNotice = this.networkNotice();
    const backgroundError = this.liveChanges() ? this.statusStore.backgroundError() : '';
    untracked(() => {
      this.feedback.sync('workspace-navigation-error', 'error', 'Git needs attention', navigationError, () => this.dismissNavigationActionError());
      this.feedback.sync('workspace-navigation-notice', 'success', 'Done', navigationNotice, () => this.dismissNavigationActionNotice());
      this.feedback.sync('workspace-network-error', 'error', 'Remote operation failed', remoteError, () => this.dismissNetworkError());
      this.feedback.sync('workspace-network-notice', 'success', 'Remote updated', remoteNotice, () => this.dismissNetworkNotice());
      this.feedback.sync('workspace-background-error', 'warning', 'Live refresh paused', backgroundError, () => this.dismissBackgroundRefreshError());
    });
  });
  private readonly syncGlobalLoader = effect(() => {
    const active = this.workspaceOperationLoading();
    const label = this.generatingCommitMessageWith() !== null
      ? 'Skibi-Bot is writing…'
      : this.networkMutation() !== null || this.fetchingRepository()
        ? 'Talking to the remote…'
        : this.workingTreeMutation() !== null || this.commitOperation() !== null
          ? 'Updating the repository…'
          : 'Reading repository…';
    untracked(() => this.feedback.setLoading(`workspace:${this.repositoryId}`, active, label));
  });
  protected readonly pushDisabledReason = computed(() => {
    const state = this.pushAnalysisState();
    if (state.kind !== 'ready') {
      return state.kind === 'loading' ? 'Push analysis is loading.' : state.message;
    }
    switch (state.analysis.readiness) {
      case 'ready':
        return null;
      case 'noUpstream':
        return null;
      case 'upToDate':
        return 'The configured upstream is already up to date.';
      case 'behind':
        return 'Pull before pushing because the local branch is behind.';
      case 'diverged':
        return 'Pull and reconcile the diverged branch before pushing.';
    }
  });
  protected readonly selectedConflictIsBinary = computed(() => {
    const state = this.conflictDetailState();
    return state.kind === 'ready' &&
      (state.detail.workingBinary || state.detail.base.binary || state.detail.ours.binary || state.detail.theirs.binary);
  });
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
  protected readonly discardableDiffHunks = computed<readonly DiscardableDiffHunk[]>(() => {
    const state = this.fileDiffState();
    if (state.kind !== 'ready' || this.historySelection() !== 'working-tree' || !state.response.unstagedPatch) {
      return [];
    }
    const displayedHunks = parseUnifiedDiff(state.response.patch).files.flatMap((file) => file.hunks);
    const model = parseUnifiedDiff(state.response.unstagedPatch);
    return model.files.flatMap((file) => {
      const firstHunkIndex = file.rows.findIndex((row) => row.kind === 'hunk-header');
      if (firstHunkIndex < 0 || file.binary) {
        return [];
      }
      const header = file.rows.slice(0, firstHunkIndex).map((row) => row.raw);
      return file.hunks.map((hunk, index) => {
        const displayHunk = displayedHunks.reduce<(typeof displayedHunks)[number] | null>((closest, candidate) => {
          if (closest === null) return candidate;
          return Math.abs(candidate.newStart - hunk.newStart) < Math.abs(closest.newStart - hunk.newStart)
            ? candidate
            : closest;
        }, null);
        if (displayHunk === null) return null;
        const additions = hunk.rows.filter((row) => row.kind === 'addition').length;
        const deletions = hunk.rows.filter((row) => row.kind === 'deletion').length;
        return {
          key: hunk.key,
          displayHunkKey: displayHunk.key,
          label: `Discard chunk ${index + 1} (+${additions} −${deletions})`,
          patch: [...header, ...hunk.rows.map((row) => row.raw)].join('\n') + '\n',
        };
      }).filter((hunk): hunk is DiscardableDiffHunk => hunk !== null);
    });
  });
  protected readonly discardableDiffHunksByDisplayHunk = computed<ReadonlyMap<string, readonly DiscardableDiffHunk[]>>(() => {
    const grouped = new Map<string, DiscardableDiffHunk[]>();
    for (const hunk of this.discardableDiffHunks()) {
      const items = grouped.get(hunk.displayHunkKey) ?? [];
      items.push(hunk);
      grouped.set(hunk.displayHunkKey, items);
    }
    return grouped;
  });
  protected readonly isFileDiffVisuallyTruncated = computed(
    () => this.displayedFileDiffRows().length > MAX_RENDERED_DIFF_ROWS,
  );

  protected readonly localBranches = computed(() => this.branchesOfKind('local'));
  protected readonly remoteBranches = computed(() => this.branchesOfKind('remote'));
  protected readonly currentLocalBranch = computed(
    () => this.localBranches().find((branch) => branch.current) ?? null,
  );
  protected readonly releaseBranch = computed(() => {
    const fullName = this.releaseBranchFullName();
    return fullName === null
      ? null
      : (this.localBranches().find((branch) => branch.fullName === fullName) ?? null);
  });
  protected readonly primaryBranch = computed(
    () => this.localBranches().find((branch) => branch.name === 'main')
      ?? this.localBranches().find((branch) => branch.name === 'master')
      ?? null,
  );
  /**
   * The toolbar intentionally opens comparison with a deterministic base. The
   * full comparison view may offer broader ref selection, but entering it
   * without two exact, distinct refs would produce an ambiguous empty state.
   */
  protected readonly compareRefs = computed<Readonly<{ source: string; target: string }> | null>(() => {
    if (this.navigationState().kind !== 'ready') {
      return null;
    }
    const source = this.currentLocalBranch();
    if (source === null) {
      return null;
    }
    const release = this.releaseBranch();
    const primary = this.primaryBranch();
    const remotePrimary = this.remoteBranches().find((branch) => {
      const name = branch.name.split('/').at(-1);
      return name === 'main' || name === 'master';
    }) ?? null;
    const target = [release, primary, remotePrimary].find(
      (candidate): candidate is RepositoryBranch => candidate !== null && candidate.fullName !== source.fullName,
    ) ?? null;
    if (target === null) {
      return null;
    }
    return { source: source.fullName, target: target.fullName };
  });
  protected readonly compareDisabledReason = computed(() => {
    if (this.navigationState().kind !== 'ready') {
      return 'Load repository references before comparing branches.';
    }
    if (this.currentLocalBranch() === null) {
      return 'Compare is available only when a local branch is active.';
    }
    return 'Mark a different release branch, or add a local main/master branch, to compare.';
  });
  protected readonly isPreviewingBranch = computed(() => this.branchPreview() !== null);
  protected readonly otherLocalBranches = computed(() =>
    this.localBranches().filter((branch) => !branch.current),
  );
  protected readonly localBranchHierarchy = computed(() =>
    buildBranchTree(this.otherLocalBranches()),
  );
  protected readonly localBranchFolderPaths = computed(() =>
    branchFolderPaths(this.localBranchHierarchy()),
  );
  protected readonly remoteBranchFolderPaths = computed(() =>
    remoteFolderPaths(this.remoteBranches()),
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

  /**
   * Select a ref to compare a preview with. A user-marked release branch wins;
   * main/master is the safe fallback. When previewing either of those refs we
   * use the other one, and finally the preview itself for a valid zero-diff
   * snapshot in unusual repositories with only one local branch.
   */
  private previewTargetFor(branch: RepositoryBranch): RepositoryBranch {
    const release = this.releaseBranch();
    if (release !== null && release.fullName !== branch.fullName) {
      return release;
    }
    const primary = this.primaryBranch();
    if (primary !== null && primary.fullName !== branch.fullName) {
      return primary;
    }
    const fallback = this.localBranches().find((candidate) => candidate.fullName !== branch.fullName);
    return fallback ?? branch;
  }

  protected previewBranch(branch: RepositoryBranch): void {
    if (branch.kind !== 'local' || this.workspaceActionBusy()) {
      return;
    }
    if (branch.current) {
      this.returnToActiveHistory();
      return;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.branchPreview.set({ branch, target: this.previewTargetFor(branch) });
    this.commanderContext.select(`branch:${branch.name}|oid:${branch.oid}|mode:preview`);
    this.closeBranchContextMenu();
    void this.reloadHistory();
  }

  protected openComparison(): void {
    const refs = this.compareRefs();
    if (refs === null) {
      return;
    }
    void this.router.navigate(['/workspace', this.repositoryId, 'compare'], {
      queryParams: refs,
    });
  }

  protected returnToActiveHistory(): void {
    if (this.branchPreview() === null) {
      return;
    }
    this.branchPreview.set(null);
    this.commanderContext.select(null);
    void this.reloadHistory();
  }

  protected isPreviewedBranch(branch: RepositoryBranch): boolean {
    const preview = this.branchPreview();
    return preview !== null && preview.branch.fullName === branch.fullName && preview.branch.oid === branch.oid;
  }

  protected checkoutBranchFromContextMenu(): void {
    const menu = this.branchContextMenu();
    if (menu === null || menu.branch.current || this.workspaceActionBusy()) {
      return;
    }
    this.closeBranchContextMenu();
    this.switchBranch(menu.branch, menu.returnFocus);
  }

  protected openBranchWorkspaceFromContextMenu(target: BranchWorkspaceOpenTarget): void {
    const menu = this.branchContextMenu();
    if (menu === null || this.workspaceActionBusy()) {
      return;
    }
    this.closeBranchContextMenu();
    void this.openBranchWorkspace(menu.branch, target);
  }

  protected openWorktreeInApplication(target: BranchWorkspaceOpenTarget): void {
    const menu = this.worktreeContextMenu();
    if (menu === null || this.workspaceActionBusy()) {
      return;
    }
    const branchFullName = this.worktreeBranchFullName(menu.worktree);
    const branch = branchFullName === null
      ? null
      : this.localBranches().find((candidate) => candidate.fullName === branchFullName) ?? null;
    if (branch === null) {
      this.navigationActionError.set('A detached worktree cannot be opened as a branch workspace.');
      return;
    }
    this.closeWorktreeContextMenu();
    void this.openBranchWorkspace(branch, target);
  }

  private async openBranchWorkspace(
    branch: RepositoryBranch,
    target: BranchWorkspaceOpenTarget,
  ): Promise<void> {
    if (branch.kind !== 'local' || this.openingBranchWorkspace() !== null) {
      return;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.openingBranchWorkspace.set(branch.fullName);
    try {
      const result = await this.ipc.invoke('open_branch_workspace', {
        request: {
          repositoryId: this.repositoryId,
          branchFullName: branch.fullName,
          expectedOid: branch.oid,
          target,
        },
      });
      if (!this.destroyed) {
        const destination = target === 'system'
          ? 'Finder / Explorer'
          : target === 'vsCode'
            ? 'VS Code'
            : 'Cursor';
        this.navigationActionNotice.set(
          `${result.worktreeCreated ? 'Created a managed worktree and opened' : 'Opened'} “${branch.name}” in ${destination}.`,
        );
        await this.loadNavigation();
      }
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(
          this.errorMessage(error, `Could not open “${branch.name}” in the selected application.`),
        );
      }
    } finally {
      if (!this.destroyed) {
        this.openingBranchWorkspace.set(null);
      }
    }
  }
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
  protected readonly remoteBranchGroups = computed<readonly RemoteBranchGroup[]>(() => {
    const expanded = this.expandedRemoteBranchFolders();
    const filter = this.navigationFilter().trim().toLowerCase();
    return [...remoteBranchTrees(this.remoteBranches()).entries()].map(([remote, tree]) => {
      const available = branchFolderPaths(tree);
      const collapsedGroup = filter.length === 0 && !expanded.has(remote);
      const collapsed = new Set(
        [...available].filter((path) => !expanded.has(remoteFolderKey(remote, path))),
      );
      const visibleFilter = filter.length > 0 && remote.toLowerCase().includes(filter)
        ? ''
        : this.navigationFilter();
      return {
        remote,
        count: this.remoteBranches().filter((branch) => remoteBranchParts(branch).remote === remote).length,
        collapsed: collapsedGroup,
        tree: collapsedGroup ? [] : visibleBranchTree(tree, collapsed, visibleFilter),
      };
    }).filter((group) => filter.length === 0 || group.remote.toLowerCase().includes(filter) || group.tree.length > 0);
  });

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
  protected readonly stagedWorkingTreeFiles = computed(() =>
    this.workingTreeSummary().files.filter((file) => file.staged),
  );
  protected readonly unstagedWorkingTreeFiles = computed(() =>
    this.workingTreeSummary().files.filter(
      (file) => file.unstaged || file.primaryStatus === 'conflicted',
    ),
  );
  protected readonly selectedDiscardableWorkingTreeFiles = computed(() => {
    const selected = this.reconciledWorkingTreeSelection();
    return this.workingTreeSummary().files.filter(
      (file) => file.unstaged && file.primaryStatus !== 'conflicted' && selected.has(this.workingTreeFileIdentity(file)),
    );
  });
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
  protected readonly enabledAiCommitMessageProviders = computed(() =>
    this.aiSupport.enabledAvailableProviders(),
  );
  protected readonly aiCommitMessageGenerationDisabled = computed(() => {
    const state = this.statusStore.state();
    return (
      this.workspaceActionBusy() ||
      state.kind !== 'ready' ||
      !this.workingTreeCapabilities().canCommit ||
      this.workingTreeCapabilities().conflictedCount > 0
    );
  });
  protected readonly amendRewritesUpstream = computed(() => {
    const state = this.statusStore.state();
    return (
      state.kind === 'ready' &&
      state.status.branch.upstream !== null &&
      state.status.branch.ahead === 0
    );
  });

  protected readonly selectedFileSummary = computed(() => {
    const files = this.selectedDetailFiles();
    if (files === null) {
      return { files: 0, additions: 0, deletions: 0 };
    }
    return files.reduce(
      (summary, file) => ({
        files: summary.files + 1,
        additions: summary.additions + (file.additions ?? 0),
        deletions: summary.deletions + (file.deletions ?? 0),
      }),
      { files: 0, additions: 0, deletions: 0 },
    );
  });

  constructor() {
    globalThis.addEventListener('resize', this.clampPanelsToViewport);
    globalThis.document.addEventListener('pointerdown', this.closeBranchContextMenuFromOutside);
    globalThis.document.addEventListener('keydown', this.handleBranchContextMenuKeydown);
    this.configureAutoFetchTimer();
    this.configureLiveChangesTimer();
    void this.aiSupport.loadAvailability();
    void this.loadRepository();
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    ++this.historyRequestGeneration;
    ++this.detailRequestGeneration;
    ++this.navigationRequestGeneration;
    ++this.fileDiffRequestGeneration;
    ++this.mutationRequestGeneration;
    ++this.pushAnalysisRequestGeneration;
    ++this.conflictListRequestGeneration;
    ++this.conflictDetailRequestGeneration;
    ++this.aiGenerationRequestGeneration;
    ++this.dirtyStatesRequestGeneration;
    ++this.submodulesRequestGeneration;
    this.clearAutoFetchTimer();
    this.clearLiveChangesTimer();
    this.feedback.setLoading(`workspace:${this.repositoryId}`, false);
    this.stopSidebarResize();
    this.stopInspectorResize();
    globalThis.removeEventListener('resize', this.clampPanelsToViewport);
    globalThis.document.removeEventListener('pointerdown', this.closeBranchContextMenuFromOutside);
    globalThis.document.removeEventListener('keydown', this.handleBranchContextMenuKeydown);
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

  protected setPullStrategy(strategy: PullStrategy): void {
    this.pullStrategy.set(strategy);
  }

  protected async pullRepository(): Promise<void> {
    const status = this.statusStore.state();
    if (status.kind !== 'ready' || status.status.branch.oid === null || this.workspaceActionBusy()) {
      return;
    }
    await this.performPull(status.status, false);
  }

  private async performPull(status: RepositoryStatusResponse, useAutoStash: boolean): Promise<void> {
    this.networkMutation.set('pull');
    this.networkNotice.set('');
    this.networkError.set('');
    try {
      let result: PullRepositoryResponse;
      try {
        result = await this.invokePull(status, useAutoStash ? {
          message: buildWipStashMessage(status.branch.head ?? 'detached HEAD'),
        } : null);
      } catch (error) {
        const message = this.errorMessage(error, 'Pull failed.');
        if (
          useAutoStash ||
          status.entries.length === 0 ||
          !this.pullFailureNeedsCleanTree(message) ||
          !this.requestPullAutoStashConfirmation(message)
        ) {
          throw error;
        }
        return;
      }
      if (
        result.state === 'failed' &&
        !useAutoStash &&
        status.entries.length > 0 &&
        this.pullFailureNeedsCleanTree(result.errorMessage)
      ) {
        this.requestPullAutoStashConfirmation(result.errorMessage ?? 'Pull failed.');
        return;
      }
      const autoStash = this.autoStashSummary(result.autoStash);
      if (result.state === 'succeeded') {
        this.networkNotice.set(`Pull succeeded.${autoStash}`);
      } else if (result.state === 'conflicted') {
        this.networkError.set(`Pull stopped with conflicts. ${result.errorMessage ?? 'Resolve the unmerged files before continuing.'}${autoStash}`);
      } else {
        this.networkError.set(`${result.errorMessage ?? 'Pull failed.'}${autoStash}`);
      }
      await this.refreshAfterNetworkMutation();
    } catch (error) {
      this.networkError.set(this.errorMessage(error, 'Pull failed.'));
      await this.refreshAfterNetworkMutation();
    } finally {
      if (!this.destroyed) {
        this.networkMutation.set(null);
      }
    }
  }

  private requestPullAutoStashConfirmation(fallbackError: string): boolean {
    this.requestDestructiveAction(
      'Stash changes and pull?',
      'Pull requires a clean working tree. Stash local changes automatically and retry?',
      'Stash and pull',
      async () => {
        const current = this.statusStore.state();
        if (current.kind !== 'ready' || current.status.branch.oid === null) {
          return;
        }
        await this.performPull(current.status, true);
      },
      {
        destructive: false,
        onCancel: () => this.networkError.set(fallbackError),
      },
    );
    return true;
  }

  protected async pushRepository(upstreamConfirmed = false): Promise<void> {
    const status = this.statusStore.state();
    const analysis = this.pushAnalysisState();
    if (
      status.kind !== 'ready' ||
      analysis.kind !== 'ready' ||
      this.workspaceActionBusy() ||
      this.pushDisabledReason() !== null
    ) {
      return;
    }
    const target = analysis.analysis.readiness === 'noUpstream'
      ? { kind: 'setUpstream' as const, remote: 'origin', remoteBranch: analysis.analysis.branch }
      : analysis.analysis.upstream === null
        ? null
        : { kind: 'configured' as const, expectedUpstream: analysis.analysis.upstream };
    if (target === null) {
      return;
    }
    if (
      target.kind === 'setUpstream' &&
      !upstreamConfirmed
    ) {
      this.requestDestructiveAction(
        'Set upstream and push?',
        `Push “${analysis.analysis.branch}” to origin/${analysis.analysis.branch} and set it as upstream?`,
        'Set upstream and push',
        () => this.pushRepository(true),
        { destructive: false },
      );
      return;
    }
    this.networkMutation.set('push');
    this.networkNotice.set('');
    this.networkError.set('');
    try {
      const result = await this.ipc.invoke('repository_push', {
        repositoryId: this.repositoryId,
        operation: { target, precondition: this.repositoryStatePrecondition(status.status) },
      });
      this.networkNotice.set(result.pushed ? 'Push succeeded.' : 'The upstream was already up to date.');
      await this.refreshAfterNetworkMutation();
    } catch (error) {
      this.networkError.set(this.errorMessage(error, 'Push failed.'));
      await this.loadPushAnalysis();
    } finally {
      if (!this.destroyed) {
        this.networkMutation.set(null);
      }
    }
  }

  protected canSetNaturalUpstream(branch: RepositoryBranch): boolean {
    const status = this.statusStore.state();
    if (
      branch.kind !== 'remote' ||
      branch.symbolicTarget !== null ||
      status.kind !== 'ready' ||
      status.status.branch.upstream !== null ||
      status.status.branch.head === null
    ) {
      return false;
    }
    const separator = branch.name.indexOf('/');
    return separator > 0 && branch.name.slice(separator + 1) === status.status.branch.head;
  }

  protected async setNaturalUpstream(branch: RepositoryBranch, confirmed = false): Promise<void> {
    const status = this.statusStore.state();
    if (status.kind !== 'ready' || !this.canSetNaturalUpstream(branch) || this.workspaceActionBusy()) {
      return;
    }
    if (!confirmed) {
      this.requestDestructiveAction(
        'Set upstream?',
        `Set ${branch.name} as the upstream for ${status.status.branch.head}?`,
        'Set upstream',
        () => this.setNaturalUpstream(branch, true),
        { destructive: false },
      );
      return;
    }
    this.networkMutation.set('setUpstream');
    this.networkNotice.set('');
    this.networkError.set('');
    try {
      const result = await this.ipc.invoke('repository_set_upstream', {
        repositoryId: this.repositoryId,
        operation: {
          remoteFullName: branch.fullName,
          expectedOid: branch.oid,
          precondition: this.repositoryStatePrecondition(status.status),
        },
      });
      this.networkNotice.set(`Upstream set to ${result.upstream}.`);
      await this.refreshAfterNetworkMutation();
    } catch (error) {
      this.networkError.set(this.errorMessage(error, 'The upstream could not be set.'));
      await this.loadPushAnalysis();
    } finally {
      if (!this.destroyed) {
        this.networkMutation.set(null);
      }
    }
  }

  private invokePull(status: RepositoryStatusResponse, autoStash: { readonly message: string } | null) {
    return this.ipc.invoke('repository_pull', {
      repositoryId: this.repositoryId,
      operation: {
        strategy: this.pullStrategy(),
        autoStash,
        precondition: this.repositoryStatePrecondition(status),
      },
    });
  }

  private pullFailureNeedsCleanTree(message: string | null): boolean {
    return /dirtyWorkingTree|clean working tree|local changes|dirty|would be overwritten/i.test(message ?? '');
  }

  private autoStashSummary(outcome: PullRepositoryResponse['autoStash']): string {
    if (outcome.create === 'notRequested' || outcome.create === 'notNeeded') {
      return '';
    }
    const stash = outcome.stash?.selector ? ` ${outcome.stash.selector}` : '';
    return ` Auto-stash:${stash} create=${outcome.create}, restore=${outcome.restore}, cleanup=${outcome.cleanup}.`;
  }

  private async loadPushAnalysis(): Promise<void> {
    const generation = ++this.pushAnalysisRequestGeneration;
    this.pushAnalysisState.set({ kind: 'loading' });
    try {
      const analysis = await this.ipc.invoke('repository_push_analysis', { repositoryId: this.repositoryId });
      if (generation === this.pushAnalysisRequestGeneration && !this.destroyed) {
        this.pushAnalysisState.set({ kind: 'ready', analysis });
      }
    } catch (error) {
      if (generation === this.pushAnalysisRequestGeneration && !this.destroyed) {
        this.pushAnalysisState.set({ kind: 'error', message: this.errorMessage(error, 'Push analysis failed.') });
      }
    }
  }

  private async refreshAfterNetworkMutation(): Promise<void> {
    await this.refreshHistoryContext();
    await Promise.all([
      this.loadPushAnalysis(),
      this.loadConflicts(),
    ]);
    this.selectWorkingTreeIfConflicted();
  }

  protected async loadConflicts(): Promise<void> {
    const generation = ++this.conflictListRequestGeneration;
    this.conflictListState.set({ kind: 'loading' });
    try {
      const response = await this.ipc.invoke('repository_conflicts', { repositoryId: this.repositoryId });
      if (generation !== this.conflictListRequestGeneration || this.destroyed) {
        return;
      }
      this.conflictListState.set({ kind: 'ready', files: response.files });
      const selected = this.selectedConflict();
      if (selected === null) {
        return;
      }
      const current = response.files.find((file) => file.path === selected.path);
      if (current === undefined || this.conflictIdentityKey(current) !== this.conflictIdentityKey(selected)) {
        this.clearConflictSelection();
      } else {
        this.selectedConflict.set(current);
      }
    } catch (error) {
      if (generation === this.conflictListRequestGeneration && !this.destroyed) {
        this.conflictListState.set({ kind: 'error', message: this.errorMessage(error, 'Conflicts could not be loaded.') });
      }
    }
  }

  protected async selectConflict(file: ConflictFileSummary): Promise<void> {
    if (this.conflictMutation()) {
      return;
    }
    const generation = ++this.conflictDetailRequestGeneration;
    this.selectedConflict.set(file);
    this.conflictDetailState.set({ kind: 'loading', path: file.path });
    this.conflictResult.set('');
    this.conflictError.set('');
    try {
      const detail = await this.ipc.invoke('repository_conflict_detail', {
        repositoryId: this.repositoryId,
        operation: {
          path: file.path,
          expectedBase: file.base,
          expectedOurs: file.ours,
          expectedTheirs: file.theirs,
        },
      });
      const selected = this.selectedConflict();
      if (
        generation !== this.conflictDetailRequestGeneration ||
        this.destroyed ||
        selected === null ||
        this.conflictIdentityKey(selected) !== this.conflictIdentityKey(file)
      ) {
        return;
      }
      this.conflictDetailState.set({ kind: 'ready', detail });
      this.conflictResult.set(detail.workingContent ?? detail.ours.content ?? detail.theirs.content ?? '');
    } catch (error) {
      if (generation === this.conflictDetailRequestGeneration && !this.destroyed) {
        this.conflictDetailState.set({
          kind: 'error',
          path: file.path,
          message: this.errorMessage(error, 'Conflict details could not be loaded.'),
        });
      }
    }
  }

  protected updateConflictResult(content: string): void {
    this.conflictResult.set(content);
  }

  protected async resolveConflict(
    resolution: ConflictResolution,
    confirmed = false,
  ): Promise<void> {
    const status = this.statusStore.state();
    const selected = this.selectedConflict();
    const detail = this.conflictDetailState();
    if (
      status.kind !== 'ready' ||
      selected === null ||
      detail.kind !== 'ready' ||
      detail.detail.path !== selected.path ||
      this.workspaceActionBusy() ||
      (resolution.kind === 'content' && this.selectedConflictIsBinary())
    ) {
      return;
    }
    if (
      resolution.kind === 'content' &&
      this.conflictResultNeedsConfirmation(detail.detail) &&
      !confirmed
    ) {
      this.requestDestructiveAction(
        'Stage unresolved content?',
        'The resolved content is unchanged or still contains conflict markers. Stage it as resolved anyway?',
        'Stage as resolved',
        () => this.resolveConflict(resolution, true),
      );
      return;
    }
    this.conflictMutation.set(true);
    this.conflictNotice.set('');
    this.conflictError.set('');
    try {
      const result = await this.ipc.invoke('repository_resolve_conflict', {
        repositoryId: this.repositoryId,
        operation: {
          path: selected.path,
          expectedBase: selected.base,
          expectedOurs: selected.ours,
          expectedTheirs: selected.theirs,
          resolution,
          precondition: this.repositoryStatePrecondition(status.status),
        },
      });
      if (result.resolved) {
        this.conflictNotice.set(`Resolved ${selected.path}.`);
      } else {
        this.conflictError.set(
          `${result.errorMessage ?? `Could not resolve ${selected.path}.`}${result.mutationMayHaveOccurred ? ' The working tree was refreshed because the mutation may have occurred.' : ''}`,
        );
      }
    } catch (error) {
      this.conflictError.set(this.errorMessage(error, `Could not resolve ${selected.path}.`));
    } finally {
      await this.refreshAfterConflictMutation();
      if (!this.destroyed) {
        this.conflictMutation.set(false);
      }
    }
  }

  protected conflictVersionText(content: string | null, binary: boolean): string {
    if (binary) {
      return 'Binary version';
    }
    return content ?? 'Version not present';
  }

  protected conflictResultNeedsConfirmation(detail: ConflictFileDetailResponse): boolean {
    const initial = detail.workingContent ?? detail.ours.content ?? detail.theirs.content ?? '';
    return (
      this.conflictResult() === initial ||
      /^(?:<{7}|={7}|>{7})(?: |$)/m.test(this.conflictResult())
    );
  }

  private conflictIdentityKey(file: ConflictFileSummary): string {
    const stage = (identity: ConflictFileSummary['base']) => identity === null ? '-' : `${identity.oid}:${identity.mode}`;
    return `${file.path}\u0000${stage(file.base)}\u0000${stage(file.ours)}\u0000${stage(file.theirs)}`;
  }

  private clearConflictSelection(): void {
    ++this.conflictDetailRequestGeneration;
    this.selectedConflict.set(null);
    this.conflictDetailState.set({ kind: 'idle' });
    this.conflictResult.set('');
  }

  private async refreshAfterConflictMutation(): Promise<void> {
    await this.refreshHistoryContext();
    await Promise.all([
      this.loadPushAnalysis(),
      this.loadConflicts(),
    ]);
    this.selectWorkingTreeIfConflicted();
  }

  private selectWorkingTreeIfConflicted(): void {
    const state = this.statusStore.state();
    if (
      state.kind === 'ready' &&
      state.status.entries.some((entry) => entry.kind === 'unmerged')
    ) {
      this.selectWorkingTree();
    }
  }

  private selectWorkingTreeWhenChanged(): void {
    const state = this.statusStore.state();
    if (
      this.historySelection() === 'none' &&
      state.kind === 'ready' &&
      state.status.entries.length > 0
    ) {
      this.selectWorkingTree();
    }
  }

  protected async refreshWorkspace(): Promise<void> {
    if (this.workspaceActionBusy()) {
      return;
    }
    this.navigationActionError.set('');
    this.refreshingWorkspace.set(true);
    try {
      await this.refreshHistoryContext();
      await Promise.all([
        this.loadPushAnalysis(),
        this.loadConflicts(),
        this.loadSubmodules(),
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

  protected async createBranchFromRemote(branch: RepositoryBranch): Promise<void> {
    if (branch.kind !== 'remote' || this.workspaceActionBusy()) {
      return;
    }
    await this.submitBranchCreation(
      null,
      {
        kind: 'remoteTracking',
        fullName: branch.fullName,
        expectedOid: branch.oid,
      },
      null,
      false,
    );
  }

  protected startBranchFromCommit(detail: RepositoryCommitDetailResponse, trigger: HTMLElement): void {
    if (this.historySelection() !== 'commit' || this.selectedOid() !== detail.oid || this.workspaceActionBusy()) {
      return;
    }
    this.branchCreationTarget.set({ kind: 'commit', oid: detail.oid, label: this.shortOid(detail.oid) });
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
    const source: BranchCreationSource | null = target.kind === 'commit'
        ? { kind: 'commit' as const, oid: target.oid }
        : state.status.branch.oid === null
          ? null
          : { kind: 'current' as const, expectedOid: state.status.branch.oid };
    if (source === null) {
      return;
    }

    await this.submitBranchCreation(name, source, this.branchCreationReturnFocus, true);
  }

  private async submitBranchCreation(
    name: string | null,
    source: BranchCreationSource,
    returnFocus: HTMLElement | null,
    closeForm: boolean,
  ): Promise<void> {

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
      if (closeForm) {
        this.branchCreationTarget.set(null);
        this.newBranchName.set('');
      }
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
          if (closeForm) {
            this.restoreFocusAfterRender(returnFocus);
            this.branchCreationReturnFocus = null;
          } else {
            this.focusAfterRender('navigation-action-notice');
          }
        }
      }
    }
  }

  protected updateStashMessage(message: string): void {
    this.stashMessage.set(message);
  }

  protected openStashCreation(): void {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }
    this.stashMessage.set(buildWipStashMessage(state.status.branch.head ?? 'detached HEAD'));
    this.stashIncludeUntracked.set(true);
    this.stashCreationOpen.set(true);
  }

  protected closeStashCreation(): void {
    if (this.stashMutation() !== null) {
      return;
    }
    this.stashCreationOpen.set(false);
    this.closeDialog(this.stashCreationDialogElement()?.nativeElement);
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
        this.stashCreationOpen.set(false);
        this.closeDialog(this.stashCreationDialogElement()?.nativeElement);
        this.navigationActionNotice.set(
          `Created ${result.stash.selector}.`,
        );
      } else if (verifiedNoChanges) {
        this.stashCreationOpen.set(false);
        this.closeDialog(this.stashCreationDialogElement()?.nativeElement);
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
    this.requestDestructiveAction(
      'Drop stash?',
      `Drop ${stash.selector} “${stash.message}”? This cannot be undone.`,
      'Drop stash',
      () => this.dropStashConfirmed(stash),
    );
  }

  private async dropStashConfirmed(stash: RepositoryStash): Promise<void> {
    if (this.workspaceActionBusy()) {
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

  protected remoteBranchFolderKey(remote: string, path: string): string {
    return remoteFolderKey(remote, path);
  }

  protected remoteBranchQualifiedName(branch: RepositoryBranch): string {
    return branch.fullName.replace(/^refs\/remotes\//, '');
  }

  protected openBranchContextMenu(
    event: MouseEvent | null,
    branch: RepositoryBranch,
    trigger: HTMLElement,
  ): void {
    if (branch.kind !== 'local') {
      return;
    }
    event?.preventDefault();
    event?.stopPropagation();
    const rect = trigger.getBoundingClientRect();
    const requestedX = event !== null && event.clientX > 0 ? event.clientX : rect.right;
    const requestedY = event !== null && event.clientY > 0 ? event.clientY : rect.bottom;
    this.closeWorktreeContextMenu();
    this.branchContextMenu.set({
      branch,
      x: Math.min(requestedX, Math.max(8, globalThis.innerWidth - 250)),
      y: Math.min(requestedY, Math.max(8, globalThis.innerHeight - 260)),
      returnFocus: trigger,
    });
    this.focusAfterRender('branch-context-menu');
  }

  protected closeBranchContextMenu(restoreFocus = false): void {
    const menu = this.branchContextMenu();
    this.branchContextMenu.set(null);
    if (restoreFocus && menu !== null) {
      this.restoreFocusAfterRender(menu.returnFocus);
    }
  }

  protected openWorktreeContextMenu(
    event: MouseEvent | null,
    worktree: RepositoryWorktree,
    trigger: HTMLElement,
  ): void {
    if (this.worktreeDisabledReason(worktree) !== null) {
      return;
    }
    event?.preventDefault();
    event?.stopPropagation();
    const rect = trigger.getBoundingClientRect();
    const requestedX = event !== null && event.clientX > 0 ? event.clientX : rect.right;
    const requestedY = event !== null && event.clientY > 0 ? event.clientY : rect.bottom;
    this.closeBranchContextMenu();
    this.worktreeContextMenu.set({
      worktree,
      x: Math.min(requestedX, Math.max(8, globalThis.innerWidth - 250)),
      y: Math.min(requestedY, Math.max(8, globalThis.innerHeight - 120)),
      returnFocus: trigger,
    });
    this.focusAfterRender('worktree-context-menu');
  }

  protected closeWorktreeContextMenu(restoreFocus = false): void {
    const menu = this.worktreeContextMenu();
    this.worktreeContextMenu.set(null);
    if (restoreFocus && menu !== null) {
      this.restoreFocusAfterRender(menu.returnFocus);
    }
  }

  protected async openWorktreeAsNewRepository(): Promise<void> {
    const menu = this.worktreeContextMenu();
    if (menu === null || this.workspaceActionBusy()) {
      return;
    }
    this.worktreeContextMenu.set(null);
    await this.openWorktree(menu.worktree);
  }

  protected isReleaseBranch(branch: RepositoryBranch): boolean {
    return this.releaseBranchFullName() === branch.fullName;
  }

  protected toggleReleaseBranch(branch: RepositoryBranch): void {
    if (branch.kind !== 'local' || this.workspaceActionBusy()) {
      return;
    }
    const next = this.isReleaseBranch(branch) ? null : branch.fullName;
    writeReleaseBranch(this.releaseBranchStorage, this.repositoryId, next);
    this.releaseBranchFullName.set(next);
    const preview = this.branchPreview();
    if (preview !== null) {
      this.branchPreview.set({
        branch: preview.branch,
        target: this.previewTargetFor(preview.branch),
      });
      void this.reloadHistory();
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set(
      next === null
        ? `“${branch.name}” is no longer the release branch.`
        : `“${branch.name}” is now the release branch for this repository.`,
    );
    this.closeBranchContextMenu(true);
  }

  protected branchDirtyChangeCount(branch: RepositoryBranch): number {
    if (branch.current) {
      const status = this.statusStore.state();
      return status.kind === 'ready' ? status.status.entries.length : 0;
    }
    const state = this.worktreeDirtyStates().get(branch.fullName);
    return state?.dirty ? state.changeCount : 0;
  }

  protected canMergeInto(branch: RepositoryBranch, source: RepositoryBranch | null): boolean {
    return source !== null && source.fullName !== branch.fullName && !this.workspaceActionBusy();
  }

  protected async mergeSelectedIntoActive(source: RepositoryBranch): Promise<void> {
    const target = this.currentLocalBranch();
    if (target === null || source.fullName === target.fullName || this.workspaceActionBusy()) {
      return;
    }
    this.requestMerge(source, target);
  }

  protected async mergeReleaseIntoBranch(target: RepositoryBranch): Promise<void> {
    const source = this.releaseBranch();
    if (!this.canMergeInto(target, source) || source === null) {
      return;
    }
    this.requestMerge(source, target);
  }

  protected async mergePrimaryIntoBranch(target: RepositoryBranch): Promise<void> {
    const source = this.primaryBranch();
    if (!this.canMergeInto(target, source) || source === null) {
      return;
    }
    this.requestMerge(source, target);
  }

  protected setMergeAutoStash(enabled: boolean): void {
    this.mergeAutoStash.set(enabled);
  }

  protected cancelMerge(): void {
    const confirmation = this.mergeConfirmation();
    if (confirmation === null || this.branchContextMutation() !== null) {
      return;
    }
    this.mergeConfirmation.set(null);
    this.closeDialog(this.mergeDialogElement()?.nativeElement);
    this.restoreFocusAfterRender(confirmation.returnFocus);
  }

  protected async confirmMerge(): Promise<void> {
    const confirmation = this.mergeConfirmation();
    if (confirmation === null || this.branchContextMutation() !== null) {
      return;
    }
    const autoStash = this.mergeAutoStash();
    this.mergeConfirmation.set(null);
    this.closeDialog(this.mergeDialogElement()?.nativeElement);
    await this.mergeIntoTargetWithSwitch(confirmation.source, confirmation.target, autoStash);
  }

  private requestMerge(source: RepositoryBranch, target: RepositoryBranch): void {
    const menu = this.branchContextMenu();
    const status = this.statusStore.state();
    if (menu === null || status.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }
    this.branchContextMenu.set(null);
    this.mergeAutoStash.set(true);
    this.mergeConfirmation.set({
      source,
      target,
      switchRequired: !target.current,
      dirty: status.status.entries.length > 0,
      returnFocus: menu.returnFocus,
    });
    this.focusAfterRender('cancel-branch-merge');
  }

  protected async pullInactiveBranch(branch: RepositoryBranch): Promise<void> {
    this.closeBranchContextMenu();
    if (branch.current) {
      await this.pullRepository();
      return;
    }
    if (
      branch.kind !== 'local' ||
      branch.upstream === null ||
      branch.upstreamGone ||
      this.workspaceActionBusy()
    ) {
      return;
    }
    this.branchContextMutation.set(`pull:${branch.fullName}`);
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    try {
      const result: PullInactiveBranchResponse = await this.ipc.invoke(
        'repository_pull_inactive_branch', {
          repositoryId: this.repositoryId,
          operation: {
            branchFullName: branch.fullName,
            expectedOid: branch.oid,
            expectedUpstream: branch.upstream,
          },
        },
      );
      this.navigationActionNotice.set(
        result.changed
          ? `Fast-forwarded “${branch.name}” from ${result.upstream}.`
          : `“${branch.name}” is already up to date with ${result.upstream}.`,
      );
    } catch (error) {
      this.navigationActionError.set(
        this.errorMessage(error, `“${branch.name}” could not be fast-forwarded in the background.`),
      );
    } finally {
      if (!this.destroyed) {
        this.branchContextMutation.set(null);
        await this.loadNavigation();
      }
    }
  }

  private async mergeIntoTargetWithSwitch(
    source: RepositoryBranch,
    target: RepositoryBranch,
    autoStash: boolean,
  ): Promise<void> {
    if (!target.current && !(await this.switchBranchForMerge(target, autoStash))) {
      return;
    }
    const refreshedTarget = this.currentLocalBranch();
    const refreshedSource = this.localBranches().find((branch) => branch.fullName === source.fullName) ?? null;
    if (refreshedTarget === null || refreshedSource === null) {
      this.navigationActionError.set('The source or target branch changed while preparing the merge. Refresh and retry.');
      return;
    }
    await this.mergeBranch(refreshedSource, refreshedTarget, autoStash);
  }

  private async switchBranchForMerge(branch: RepositoryBranch, autoStash: boolean): Promise<boolean> {
    if (branch.kind !== 'local' || branch.current || this.workspaceActionBusy()) {
      return branch.current;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.switchingBranch.set(branch.fullName);
    try {
      const currentBranch = this.currentLocalBranch()?.name ?? this.branchName();
      const result = await this.ipc.invoke('switch_repository_branch', {
        repositoryId: this.repositoryId,
        operation: {
          fullName: branch.fullName,
          expectedOid: branch.oid,
          stashOnDirty: autoStash,
          stashMessage: autoStash
            ? buildWipStashMessage(
                currentBranch,
                new Date(Date.now() - new Date().getTimezoneOffset() * 60_000),
              )
            : null,
        },
      });
      if (this.destroyed) {
        return false;
      }
      this.recordBranchSwitchOutcome(result);
      await this.refreshAfterBranchSwitch(result.operationSucceeded);
      return result.operationSucceeded;
    } catch (error) {
      if (!this.destroyed) {
        const fallback = autoStash
          ? 'The target branch could not be activated after auto-stashing.'
          : 'The target branch could not be activated. Enable auto-stash if uncommitted changes block the switch.';
        this.navigationActionError.set(this.errorMessage(error, fallback));
        await this.refreshAfterBranchSwitch(false);
      }
      return false;
    } finally {
      if (!this.destroyed) {
        this.switchingBranch.set(null);
      }
    }
  }

  private async mergeBranch(
    source: RepositoryBranch,
    target: RepositoryBranch,
    autoStashRequested: boolean,
  ): Promise<void> {
    const status = this.statusStore.state();
    if (status.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }
    let autoStash: { readonly message: string } | null = null;
    if (status.status.entries.length > 0 && autoStashRequested) {
      autoStash = { message: buildWipStashMessage(target.name) };
    }

    this.branchContextMutation.set(`merge:${source.fullName}:${target.fullName}`);
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    try {
      const result: MergeRepositoryBranchResponse = await this.ipc.invoke('repository_merge_branch', {
        repositoryId: this.repositoryId,
        operation: {
          sourceFullName: source.fullName,
          expectedSourceOid: source.oid,
          targetFullName: target.fullName,
          expectedTargetOid: target.oid,
          autoStash,
        },
      });
      if (result.status !== null) {
        this.statusStore.acceptMutationResult(result.status);
      }
      const stash = this.autoStashSummary(result.autoStash);
      if (result.state === 'succeeded') {
        this.navigationActionNotice.set(`Merged “${source.name}” into “${target.name}”.${stash}`);
      } else if (result.state === 'conflicted') {
        this.navigationActionError.set(
          `Merge stopped with conflicts. ${result.errorMessage ?? 'Resolve the unmerged files before continuing.'}${stash}`,
        );
      } else {
        const uncertainty = result.mutationMayHaveOccurred
          ? ' Git may have changed the target; inspect the refreshed branch before retrying.'
          : '';
        this.navigationActionError.set(`${result.errorMessage ?? 'Merge failed.'}${uncertainty}${stash}`);
      }
      await this.refreshAfterNetworkMutation();
    } catch (error) {
      this.navigationActionError.set(this.errorMessage(error, `Could not merge “${source.name}” into “${target.name}”.`));
      await this.refreshAfterNetworkMutation();
    } finally {
      if (!this.destroyed) {
        this.branchContextMutation.set(null);
      }
    }
  }

  protected switchBranch(branch: RepositoryBranch, trigger?: HTMLElement): boolean {
    if (branch.kind !== 'local' || branch.current || this.workspaceActionBusy()) {
      return false;
    }
    const returnFocus = trigger
      ?? (globalThis.document?.activeElement instanceof HTMLElement
        ? globalThis.document.activeElement
        : globalThis.document?.body);
    if (!(returnFocus instanceof HTMLElement)) {
      return false;
    }
    const state = this.statusStore.state();
    const targetWorktree = this.worktreeForBranch(branch.fullName);
    this.switchAutoStash.set(
      targetWorktree === null && state.kind === 'ready' && state.status.entries.length > 0,
    );
    this.switchConfirmation.set({
      branch,
      dirty: targetWorktree === null && state.kind === 'ready' && state.status.entries.length > 0,
      targetWorktree,
      returnFocus,
    });
    this.focusAfterRender('cancel-branch-switch');
    return true;
  }

  protected setSwitchAutoStash(enabled: boolean): void {
    this.switchAutoStash.set(enabled);
  }

  protected cancelBranchSwitch(): void {
    const confirmation = this.switchConfirmation();
    if (confirmation === null || this.switchingBranch() !== null) {
      return;
    }
    this.switchConfirmation.set(null);
    this.restoreFocusAfterRender(confirmation.returnFocus);
  }

  protected async confirmBranchSwitch(): Promise<void> {
    const confirmation = this.switchConfirmation();
    if (confirmation === null || this.switchingBranch() !== null) {
      return;
    }
    const branch = confirmation.branch;
    if (confirmation.targetWorktree !== null) {
      this.switchConfirmation.set(null);
      await this.openWorktree(confirmation.targetWorktree);
      return;
    }
    const autoStash = confirmation.dirty && this.switchAutoStash();
    this.switchConfirmation.set(null);

    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.switchingBranch.set(branch.fullName);
    try {
      const result = await this.ipc.invoke('switch_repository_branch', {
        repositoryId: this.repositoryId,
        operation: {
          fullName: branch.fullName,
          expectedOid: branch.oid,
          stashOnDirty: autoStash,
          stashMessage: autoStash
            ? buildWipStashMessage(
                this.currentLocalBranch()?.name ?? this.branchName(),
                new Date(Date.now() - new Date().getTimezoneOffset() * 60_000),
              )
            : null,
        },
      });
      if (this.destroyed) {
        return;
      }
      this.recordBranchSwitchOutcome(result);
      await this.refreshAfterBranchSwitch(result.operationSucceeded);
    } catch (error) {
      const message = this.errorMessage(error, 'The branch could not be switched.');
      if (!this.destroyed) {
        this.navigationActionError.set(message);
        if (message.includes('dirtyWorkingTree') && !autoStash) {
          this.navigationActionError.set(
            `${message} Retry the switch and enable auto-stash to preserve local changes.`,
          );
        }
      }
    } finally {
      if (!this.destroyed) {
        this.switchingBranch.set(null);
      }
    }
    if (!this.destroyed) {
      this.restoreFocusAfterRender(confirmation.returnFocus);
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

  private async refreshAfterBranchSwitch(clearPreview: boolean): Promise<void> {
    await this.refreshHistoryContext(clearPreview);
  }

  protected requestBranchDeletion(branch: RepositoryBranch, trigger: HTMLElement): void {
    if (branch.kind !== 'local' || branch.current || this.workspaceActionBusy()) {
      return;
    }
    this.deletionConfirmation.set({ kind: 'branch', branch, returnFocus: trigger });
    this.focusAfterRender('cancel-reference-deletion');
  }

  protected requestWorktreeRemoval(worktree: RepositoryWorktree, trigger: HTMLElement): void {
    if (this.workspaceActionBusy() || this.worktreeRemovalDisabledReason(worktree) !== null) {
      return;
    }
    const branchFullName = this.worktreeBranchFullName(worktree);
    this.deletionConfirmation.set({
      kind: 'worktree',
      worktree,
      branchFullName,
      branchLabel: branchFullName === null ? null : this.worktreeBranchLabel(worktree),
      returnFocus: trigger,
    });
    this.worktreeRemovalMode.set('safe');
    this.forceRemovalAcknowledged.set(false);
    this.worktreeRemovalStashMessage.set(null);
    this.worktreeRemovalDialogError.set('');
    this.focusAfterRender('cancel-reference-deletion');
  }

  protected setWorktreeRemovalMode(mode: WorktreeRemovalMode): void {
    const confirmation = this.deletionConfirmation();
    if (confirmation?.kind !== 'worktree' || this.removingWorktree() !== null) {
      return;
    }
    this.worktreeRemovalMode.set(mode);
    this.forceRemovalAcknowledged.set(false);
    this.worktreeRemovalDialogError.set('');
    this.worktreeRemovalStashMessage.set(
      mode === 'stashAndForce'
        ? buildWipStashMessage(confirmation.branchLabel ?? 'detached HEAD')
        : null,
    );
  }

  protected setForceRemovalAcknowledged(acknowledged: boolean): void {
    this.forceRemovalAcknowledged.set(acknowledged);
  }

  protected cancelReferenceDeletion(): void {
    if (this.deletingBranch() !== null || this.removingWorktree() !== null) {
      return;
    }
    const confirmation = this.deletionConfirmation();
    this.deletionConfirmation.set(null);
    this.restoreFocusAfterRender(confirmation?.returnFocus ?? null);
  }

  private requestDestructiveAction(
    title: string,
    description: string,
    confirmLabel: string,
    confirm: () => Promise<void>,
    options: { readonly destructive?: boolean; readonly onCancel?: () => void } = {},
  ): void {
    const activeElement = globalThis.document?.activeElement;
    this.destructiveActionConfirmation.set({
      title,
      description,
      confirmLabel,
      destructive: options.destructive ?? true,
      returnFocus: activeElement instanceof HTMLElement ? activeElement : null,
      confirm,
      cancel: options.onCancel ?? null,
    });
  }

  protected cancelDestructiveAction(): void {
    const confirmation = this.destructiveActionConfirmation();
    if (confirmation === null) {
      return;
    }
    this.destructiveActionConfirmation.set(null);
    confirmation.cancel?.();
    this.restoreFocusAfterRender(confirmation.returnFocus);
  }

  protected async confirmDestructiveAction(): Promise<void> {
    const confirmation = this.destructiveActionConfirmation();
    if (confirmation === null) {
      return;
    }
    this.destructiveActionConfirmation.set(null);
    await confirmation.confirm();
  }

  protected async confirmReferenceDeletion(): Promise<void> {
    const confirmation = this.deletionConfirmation();
    if (confirmation === null || this.deletingBranch() !== null || this.removingWorktree() !== null) {
      return;
    }
    if (confirmation.kind === 'branch') {
      await this.deleteBranch(confirmation.branch);
    } else {
      const mode = this.worktreeRemovalMode();
      if (mode === 'force' && !this.forceRemovalAcknowledged()) {
        return;
      }
      await this.removeWorktree(
        confirmation.worktree,
        confirmation.branchFullName,
        mode,
        this.worktreeRemovalStashMessage(),
      );
    }
  }

  private async deleteBranch(branch: RepositoryBranch): Promise<void> {
    this.navigationActionError.set('');
    this.deletingBranch.set(branch.fullName);
    try {
      await this.ipc.invoke('delete_repository_branch', {
        repositoryId: this.repositoryId,
        fullName: branch.fullName,
        expectedOid: branch.oid,
      });
      if (!this.destroyed) {
        this.deletionConfirmation.set(null);
        this.navigationActionNotice.set(`Deleted local branch “${branch.name}”.`);
        await this.loadNavigation();
        this.focusAfterRender('navigation-action-notice');
      }
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The local branch could not be deleted.'));
        this.focusAfterRender('navigation-action-error');
      }
    } finally {
      if (!this.destroyed) {
        this.deletingBranch.set(null);
        this.deletionConfirmation.set(null);
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

  private async removeWorktree(
    worktree: RepositoryWorktree,
    branchFullName: string | null,
    mode: WorktreeRemovalMode,
    stashMessage: string | null,
  ): Promise<void> {
    this.navigationActionError.set('');
    this.worktreeRemovalDialogError.set('');
    this.removingWorktree.set(worktree.path);
    let closeDialog = false;
    try {
      const result = await this.ipc.invoke('remove_repository_worktree', {
        repositoryId: this.repositoryId,
        path: worktree.path,
        expectedHead: worktree.head,
        branchFullName,
        mode,
        stashMessage,
      });
      if (!this.destroyed) {
        if (!result.worktreeRemoved) {
          const retainedStash = result.stash === null
            ? ''
            : ` Saved changes remain available as ${result.stash.selector}.`;
          this.worktreeRemovalDialogError.set(
            `${result.worktreeRemovalError ?? 'The worktree was not removed.'}${retainedStash}`,
          );
          this.focusAfterRender('worktree-removal-dialog-error');
          return;
        }
        closeDialog = true;
        if (result.branchDeletionError !== null) {
          this.navigationActionError.set(
            `The worktree was removed, but its branch was kept: ${result.branchDeletionError}`,
          );
        } else {
          const stashNotice = result.stash === null
            ? ''
            : ` Changes were saved as ${result.stash.selector}.`;
          this.navigationActionNotice.set(
            result.branchDeleted
              ? `Removed worktree and deleted local branch “${this.worktreeBranchLabel(worktree)}”.${stashNotice}`
              : `Removed worktree. No local branch was deleted.${stashNotice}`,
          );
        }
        await this.loadNavigation();
        this.focusAfterRender(
          result.branchDeletionError === null ? 'navigation-action-notice' : 'navigation-action-error',
        );
      }
    } catch (error) {
      if (!this.destroyed) {
        this.worktreeRemovalDialogError.set(
          this.errorMessage(error, 'The worktree could not be removed.'),
        );
        await this.loadNavigation();
        this.focusAfterRender('worktree-removal-dialog-error');
      }
    } finally {
      if (!this.destroyed) {
        this.removingWorktree.set(null);
        if (closeDialog) {
          this.deletionConfirmation.set(null);
        }
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

  protected dismissNavigationActionError(): void {
    this.navigationActionError.set('');
  }

  protected dismissNavigationActionNotice(): void {
    this.navigationActionNotice.set('');
  }

  protected dismissNetworkError(): void {
    this.networkError.set('');
  }

  protected dismissNetworkNotice(): void {
    this.networkNotice.set('');
  }

  protected dismissBackgroundRefreshError(): void {
    this.statusStore.backgroundError.set('');
  }

  protected submoduleDisabledReason(submodule: RepositorySubmodule): string | null {
    if (!submodule.present || !submodule.initialized) {
      return 'Not initialized locally';
    }
    if (submodule.commitState === 'unavailable') {
      return 'Commit is unavailable locally';
    }
    if (submodule.worktreeState === 'unavailable') {
      return 'Working tree is unavailable';
    }
    return null;
  }

  protected submoduleCommitToken(submodule: RepositorySubmodule): string {
    switch (submodule.commitState) {
      case 'atExpected':
        return 'at expected commit';
      case 'different':
        return 'commit differs';
      case 'conflicted':
        return 'commit conflict';
      case 'unavailable':
        return 'commit unavailable';
    }
  }

  protected submoduleWorktreeToken(submodule: RepositorySubmodule): string {
    switch (submodule.worktreeState) {
      case 'clean':
        return 'clean';
      case 'modified':
        return `modified${submodule.changeCount > 0 ? ` · ${submodule.changeCount}` : ''}`;
      case 'untracked':
        return `untracked${submodule.changeCount > 0 ? ` · ${submodule.changeCount}` : ''}`;
      case 'modifiedAndUntracked':
        return `changes${submodule.changeCount > 0 ? ` · ${submodule.changeCount}` : ''}`;
      case 'conflicted':
        return 'conflicted';
      case 'unavailable':
        return 'unavailable';
    }
  }

  protected async openSubmodule(submodule: RepositorySubmodule): Promise<void> {
    if (this.workspaceActionBusy() || this.submoduleDisabledReason(submodule) !== null) {
      return;
    }
    this.navigationActionError.set('');
    this.openingSubmodule.set(submodule.path);
    try {
      const repository = await this.catalog.openSubmodule(this.repositoryId, submodule.path);
      if (this.destroyed) {
        return;
      }
      this.statusStore.setRepositoryPath(repository.path);
      if (repository.id !== this.repositoryId) {
        await this.router.navigateByUrl('/repositories', { skipLocationChange: true });
      }
      await this.router.navigate(['/workspace', repository.id, 'history']);
    } catch (error) {
      if (!this.destroyed) {
        this.navigationActionError.set(this.errorMessage(error, 'The submodule could not be opened.'));
      }
    } finally {
      if (!this.destroyed) {
        this.openingSubmodule.set(null);
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

  protected startInspectorResize(event: PointerEvent): void {
    if (event.button !== 0 || this.inspectorMaximized() || globalThis.innerWidth <= INSPECTOR_LAYOUT_BREAKPOINT) {
      return;
    }
    event.preventDefault();
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture can be unavailable in tests and older embedded webviews.
    }
    this.activeInspectorResizePointer = event.pointerId;
    this.resizingInspector.set(true);
    globalThis.addEventListener('pointermove', this.resizeInspector);
    globalThis.addEventListener('pointerup', this.finishInspectorResize);
    globalThis.addEventListener('pointercancel', this.finishInspectorResize);
    globalThis.addEventListener('blur', this.cancelInspectorResize);
  }

  protected resizeInspectorWithKeyboard(event: KeyboardEvent): void {
    if (this.inspectorMaximized() || globalThis.innerWidth <= INSPECTOR_LAYOUT_BREAKPOINT) {
      return;
    }
    let width: number | null = null;
    if (event.key === 'ArrowLeft') {
      width = this.inspectorWidth() + INSPECTOR_KEYBOARD_STEP;
    } else if (event.key === 'ArrowRight') {
      width = this.inspectorWidth() - INSPECTOR_KEYBOARD_STEP;
    } else if (event.key === 'Home') {
      width = INSPECTOR_MIN_WIDTH;
    } else if (event.key === 'End') {
      width = this.maximumInspectorWidth();
    }
    if (width === null) {
      return;
    }
    event.preventDefault();
    this.setInspectorWidth(width, true);
  }

  protected inspectorMaximumWidth(): number {
    return this.maximumInspectorWidth();
  }

  protected toggleInspectorMaximized(): void {
    this.stopInspectorResize();
    this.inspectorMaximized.update((maximized) => !maximized);
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

  protected worktreeDisplayName(worktree: RepositoryWorktree): string {
    const normalized = worktree.path.replace(/[\\/]+$/, '');
    return normalized.split(/[\\/]/).at(-1) || worktree.path;
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
        const availableLocalRefs = new Set(
          navigation.branches
            .filter((branch) => branch.kind === 'local')
            .map((branch) => branch.fullName),
        );
        this.releaseBranchFullName.set(
          readReleaseBranch(this.releaseBranchStorage, this.repositoryId, availableLocalRefs),
        );
        this.reconcileSelectedStash(navigation.stashes);
        const localTree = buildBranchTree(
          navigation.branches.filter((branch) => branch.kind === 'local' && !branch.current),
        );
        const remoteBranches = navigation.branches.filter((branch) => branch.kind === 'remote');
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
            remoteFolderPaths(remoteBranches),
          ),
        );
        void this.loadWorktreeDirtyStates();
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

  /**
   * History requests carry navigation OIDs as optimistic preconditions. Keep
   * the snapshot deterministic after any operation that can move HEAD or a
   * ref: status first, navigation second, then history.
   */
  private async refreshHistoryContext(
    clearPreview = false,
    refreshStatus = true,
  ): Promise<void> {
    if (clearPreview) {
      this.branchPreview.set(null);
    }
    if (refreshStatus) {
      await this.statusStore.refresh();
    }
    await this.loadNavigation();
    this.reconcileBranchPreview();
    await this.reloadHistory();
  }

  private reconcileBranchPreview(): void {
    const preview = this.branchPreview();
    const navigation = this.navigationState();
    if (preview === null || navigation.kind !== 'ready') {
      return;
    }
    const current = navigation.navigation.branches.find(
      (branch) => branch.kind === 'local' && branch.fullName === preview.branch.fullName,
    );
    if (current === undefined || current.current || current.oid !== preview.branch.oid) {
      this.branchPreview.set(null);
    }
  }

  protected async loadSubmodules(): Promise<void> {
    const generation = ++this.submodulesRequestGeneration;
    this.submoduleState.set({ kind: 'loading' });
    try {
      const response = await this.ipc.invoke('repository_submodules', {
        repositoryId: this.repositoryId,
      });
      if (generation === this.submodulesRequestGeneration && !this.destroyed) {
        this.submoduleState.set({ kind: 'ready', submodules: response.submodules });
      }
    } catch (error) {
      if (generation === this.submodulesRequestGeneration && !this.destroyed) {
        this.submoduleState.set({
          kind: 'error',
          message: this.errorMessage(error, 'Submodules could not be loaded.'),
        });
      }
    }
  }

  protected async reloadHistory(): Promise<void> {
    const selectionBeforeReload = this.historySelection();
    const generation = ++this.historyRequestGeneration;
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.commits.set([]);
    this.nextCursor.set(null);
    this.isLoadingMore.set(false);
    this.selectedOid.set(null);
    this.historySelection.set('none');
    this.detailState.set({ kind: 'idle' });
    this.stashDetailState.set({ kind: 'idle' });
    this.selectedStash.set(null);
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.selectedStashFileSource.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.historyError.set('');
    this.historyPhase.set('loading');

    try {
      const response = await this.requestHistoryPage(null);
      if (generation !== this.historyRequestGeneration) {
        return;
      }
      this.commits.set(response.commits);
      this.nextCursor.set(response.nextCursor);
      this.historyPhase.set('ready');
      if (
        this.branchPreview() === null &&
        (selectionBeforeReload === 'none' || selectionBeforeReload === 'working-tree')
      ) {
        this.selectWorkingTreeWhenChanged();
      }
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
      const response = await this.requestHistoryPage(cursor);
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

  private requestHistoryPage(cursor: string | null): Promise<RepositoryHistoryResponse> {
    const preview = this.branchPreview();
    const activeBranch = this.currentLocalBranch();
    const release = this.releaseBranch();
    const primary = this.primaryBranch();
    const activeTarget = activeBranch === null
      ? null
      : release !== null && activeBranch.fullName !== release.fullName
        ? release
        : primary !== null && activeBranch.fullName !== primary.fullName
          ? primary
          : null;
    const historyContext = preview ?? (
      activeBranch !== null && activeTarget !== null
        ? { branch: activeBranch, target: activeTarget }
        : null
    );
    if (historyContext === null) {
      return this.ipc.invoke('repository_history', {
        repositoryId: this.repositoryId,
        cursor,
        limit: HISTORY_PAGE_SIZE,
      });
    }
    return this.ipc.invoke('repository_branch_history', {
      repositoryId: this.repositoryId,
      branchFullName: historyContext.branch.fullName,
      expectedBranchOid: historyContext.branch.oid,
      targetFullName: historyContext.target.fullName,
      expectedTargetOid: historyContext.target.oid,
      cursor,
      limit: HISTORY_PAGE_SIZE,
    });
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
    this.commanderContext.select(`commit:${commit.oid}|summary:${commit.summary}`);
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.detailState.set({ kind: 'loading' });
    this.stashDetailState.set({ kind: 'idle' });
    this.selectedStash.set(null);
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

  protected async selectStash(stash: RepositoryStash): Promise<void> {
    if (
      this.historySelection() === 'stash' &&
      this.selectedOid() === stash.oid &&
      this.stashDetailState().kind === 'ready'
    ) {
      return;
    }

    const generation = ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.historySelection.set('stash');
    this.selectedOid.set(stash.oid);
    this.selectedStash.set(stash);
    this.commanderContext.select(`stash:${stash.selector}|oid:${stash.oid}|message:${stash.message}`);
    this.detailState.set({ kind: 'idle' });
    this.stashDetailState.set({ kind: 'loading' });
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.selectedStashFileSource.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.fileDiffDisplayMode.set('contextual');
    try {
      const detail = await this.ipc.invoke('repository_stash_detail', {
        repositoryId: this.repositoryId,
        oid: stash.oid,
      });
      if (
        generation === this.detailRequestGeneration &&
        this.historySelection() === 'stash' &&
        this.selectedOid() === stash.oid
      ) {
        this.stashDetailState.set({ kind: 'ready', detail });
      }
    } catch (error) {
      if (
        generation === this.detailRequestGeneration &&
        this.historySelection() === 'stash' &&
        this.selectedOid() === stash.oid
      ) {
        this.stashDetailState.set({
          kind: 'error',
          message: this.errorMessage(error, 'Stash details could not be loaded. Select the stash to retry.'),
        });
      }
    }
  }

  protected selectPullRequest(): void {
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.historySelection.set('pull-request');
    this.commanderContext.select('pull-request:list');
    this.selectedOid.set(null);
    this.detailState.set({ kind: 'idle' });
    this.stashDetailState.set({ kind: 'idle' });
    this.selectedStash.set(null);
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.selectedStashFileSource.set(null);
    this.fileDiffState.set({ kind: 'idle' });
  }

  protected selectWorkingTree(): void {
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.historySelection.set('working-tree');
    this.commanderContext.select('working-tree:current');
    this.selectedOid.set(null);
    this.detailState.set({ kind: 'idle' });
    this.stashDetailState.set({ kind: 'idle' });
    this.selectedStash.set(null);
    this.selectedFilePath.set(null);
    this.selectedWorkingTreeEntryKind.set(null);
    this.selectedStashFileSource.set(null);
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

  protected clearWorkingTreeSelection(): void {
    this.selectedWorkingTreeFiles.set(new Set());
  }

  protected updateCommitMessage(message: string): void {
    this.commitMessage.set(message);
  }

  protected async generateCommitMessage(
    provider: AiCliProvider,
  ): Promise<void> {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || this.aiCommitMessageGenerationDisabled()) {
      return;
    }

    const expectedHeadOid = state.status.branch.oid;
    const expectedIndexFingerprint = state.status.indexFingerprint;
    const expectedWorktreeFingerprint = state.status.worktreeFingerprint;
    const generation = ++this.aiGenerationRequestGeneration;
    this.commitMessageGenerationError.set('');
    this.generatingCommitMessageWith.set(provider);

    try {
      const result = await this.aiSupport.generateCommitMessage(provider, {
        repositoryId: this.repositoryId,
        expectedHead: expectedHeadOid,
        indexFingerprint: expectedIndexFingerprint,
        worktreeFingerprint: expectedWorktreeFingerprint,
      });
      if (this.destroyed || generation !== this.aiGenerationRequestGeneration) {
        return;
      }

      const current = this.statusStore.state();
      if (
        current.kind !== 'ready' ||
        current.status.branch.oid !== expectedHeadOid ||
        result.indexFingerprint !== expectedIndexFingerprint ||
        result.worktreeFingerprint !== expectedWorktreeFingerprint ||
        current.status.indexFingerprint !== result.indexFingerprint ||
        current.status.worktreeFingerprint !== result.worktreeFingerprint
      ) {
        this.commitMessageGenerationError.set(
          'The staged changes changed while the message was being generated. Generate a new message for the current changes.',
        );
        return;
      }

      this.commitMessage.set(result.message);
    } catch (error) {
      if (generation === this.aiGenerationRequestGeneration && !this.destroyed) {
        this.commitMessageGenerationError.set(this.aiCommitMessageError(error, provider));
      }
    } finally {
      if (generation === this.aiGenerationRequestGeneration && !this.destroyed) {
        this.generatingCommitMessageWith.set(null);
      }
    }
  }

  protected aiCommitMessageProviderIsGenerating(provider: AiCliProvider): boolean {
    return this.generatingCommitMessageWith() === provider;
  }

  protected aiCommitMessageProviderLabel(provider: AiCliProvider): string {
    switch (provider) {
      case 'claude':
        return 'Claude';
      case 'cursor':
        return 'Cursor';
      default:
        return 'Codex';
    }
  }

  private aiCommitMessageError(error: unknown, provider: AiCliProvider): string {
    const label = this.aiCommitMessageProviderLabel(provider);
    const code = typeof error === 'object' && error !== null && 'code' in error
      ? (error as { readonly code?: unknown }).code
      : null;
    switch (code) {
      case 'notAvailable':
        return `${label} is no longer available on this computer.`;
      case 'notEnabled':
        return `${label} is disabled in AI support settings.`;
      case 'authenticationRequired':
        return `${label} requires sign-in before it can generate a commit message.`;
      case 'cancelled':
        return `${label} cancelled commit-message generation.`;
      default:
        return this.errorMessage(error, `${label} could not generate a commit message.`);
    }
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
    await this.applyIndexChangeWithSelection(
      action,
      all,
      this.reconciledWorkingTreeSelection(),
    );
  }

  protected workingTreeFileActionDisabled(action: IndexAction, file: WorkingTreeFile): boolean {
    if (this.workspaceActionBusy() || file.primaryStatus === 'conflicted') {
      return true;
    }
    const plan = planWorkingTreeMutation(
      action,
      this.workingTreeSummary().files,
      new Set([this.workingTreeFileIdentity(file)]),
    );
    return plan.actionableCount === 0 || plan.ambiguous;
  }

  protected async applyIndexChangeToFile(action: IndexAction, file: WorkingTreeFile): Promise<void> {
    await this.applyIndexChangeWithSelection(
      action,
      false,
      new Set([this.workingTreeFileIdentity(file)]),
    );
  }

  protected async discardSelectedWorkingTreeFiles(): Promise<void> {
    await this.discardWorkingTreeFiles(this.selectedDiscardableWorkingTreeFiles());
  }

  protected async discardWorkingTreeFile(file: WorkingTreeFile): Promise<void> {
    await this.discardWorkingTreeFiles([file]);
  }

  private async discardWorkingTreeFiles(files: readonly WorkingTreeFile[]): Promise<void> {
    const state = this.statusStore.state();
    const candidates = files.filter((file) => file.unstaged && file.primaryStatus !== 'conflicted');
    if (state.kind !== 'ready' || this.workspaceActionBusy() || candidates.length === 0) {
      return;
    }
    const untracked = candidates.filter((file) => file.entryKind === 'untracked');
    const preview = candidates.slice(0, 5).map((file) => `• ${file.path}`).join('\n');
    const remainder = candidates.length > 5 ? `\n…and ${candidates.length - 5} more.` : '';
    const deletionWarning = untracked.length > 0
      ? `\n\nWARNING: ${untracked.length} new untracked ${untracked.length === 1 ? 'file' : 'files'} will be permanently deleted, not restored.`
      : '';
    this.requestDestructiveAction(
      `Discard ${candidates.length} ${candidates.length === 1 ? 'file' : 'files'}?`,
      `Discard local changes in ${candidates.length} ${candidates.length === 1 ? 'file' : 'files'}?\n\n${preview}${remainder}${deletionWarning}\n\nThis cannot be undone.`,
      untracked.length > 0 ? 'Delete and discard' : 'Discard changes',
      () => this.discardWorkingTreeFilesConfirmed(candidates),
    );
  }

  private async discardWorkingTreeFilesConfirmed(files: readonly WorkingTreeFile[]): Promise<void> {
    const state = this.statusStore.state();
    const selected = new Set(files.map((file) => this.workingTreeFileIdentity(file)));
    const candidates = this.workingTreeSummary().files.filter(
      (file) => selected.has(this.workingTreeFileIdentity(file)) && file.unstaged && file.primaryStatus !== 'conflicted',
    );
    if (state.kind !== 'ready' || this.workspaceActionBusy() || candidates.length === 0) {
      return;
    }

    this.workingTreeMutationError.set('');
    const generation = ++this.mutationRequestGeneration;
    this.workingTreeMutation.set({ action: 'discard', scope: 'selected' });
    try {
      const result = await this.ipc.invoke('repository_discard_worktree_changes', {
        repositoryId: this.repositoryId,
        operation: {
          entries: candidates.map(workingTreeEntrySelector),
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
          this.errorMessage(error, 'The selected local changes could not be discarded.'),
        );
        await this.statusStore.refresh();
      }
    } finally {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutation.set(null);
      }
    }
  }

  protected async discardWorkingTreeHunk(hunk: DiscardableDiffHunk): Promise<void> {
    const state = this.statusStore.state();
    const path = this.selectedFilePath();
    const entryKind = this.selectedWorkingTreeEntryKind();
    const selected = this.workingTreeSummary().files.find(
      (file) => file.path === path && file.entryKind === entryKind,
    );
    if (
      state.kind !== 'ready' || selected === undefined || !selected.unstaged ||
      selected.entryKind === 'untracked' || this.workspaceActionBusy()
    ) {
      return;
    }
    const selectedIdentity = this.workingTreeFileIdentity(selected);
    this.requestDestructiveAction(
      'Discard change block?',
      `Discard this change block from ${selected.path}?\n\nThis cannot be undone.`,
      'Discard chunk',
      () => this.discardWorkingTreeHunkConfirmed(hunk, selectedIdentity),
    );
  }

  private async discardWorkingTreeHunkConfirmed(
    hunk: DiscardableDiffHunk,
    selectedIdentity: string,
  ): Promise<void> {
    const state = this.statusStore.state();
    const selected = this.workingTreeSummary().files.find(
      (file) => this.workingTreeFileIdentity(file) === selectedIdentity,
    );
    if (
      state.kind !== 'ready' || selected === undefined || !selected.unstaged ||
      selected.entryKind === 'untracked' || this.workspaceActionBusy()
    ) {
      return;
    }
    this.workingTreeMutationError.set('');
    const generation = ++this.mutationRequestGeneration;
    this.workingTreeMutation.set({ action: 'discardHunk', scope: null });
    try {
      const result = await this.ipc.invoke('repository_discard_worktree_hunk', {
        repositoryId: this.repositoryId,
        operation: {
          entry: workingTreeEntrySelector(selected),
          patch: hunk.patch,
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
      const remaining = this.workingTreeSummary().files.find(
        (file) => file.path === selected.path && file.entryKind === selected.entryKind && file.unstaged,
      );
      if (remaining === undefined) {
        this.closeFileDiff();
      } else {
        await this.loadWorkingTreeFileDiff(remaining.path, remaining.oldPath, remaining.entryKind);
      }
    } catch (error) {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutationError.set(
          this.errorMessage(error, 'The selected change block could not be discarded.'),
        );
        await this.statusStore.refresh();
      }
    } finally {
      if (this.isCurrentMutation(generation)) {
        this.workingTreeMutation.set(null);
      }
    }
  }

  private async applyIndexChangeWithSelection(
    action: IndexAction,
    all: boolean,
    selected: ReadonlySet<string>,
  ): Promise<void> {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || this.workspaceActionBusy()) {
      return;
    }

    const plan = planWorkingTreeMutation(
      action,
      this.workingTreeSummary().files,
      all ? 'all' : selected,
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

  protected setResetMode(mode: ResetMode): void {
    this.resetMode.set(mode);
  }

  protected requestCherryPick(detail: RepositoryCommitDetailResponse): void {
    if (detail.parents.length > 1 || this.workspaceActionBusy()) {
      return;
    }
    this.requestDestructiveAction(
      'Cherry-pick this commit?',
      `Apply ${this.shortOid(detail.oid)} to the current local branch. The index and working tree must be clean.`,
      'Cherry-pick',
      () => this.runCommitOperation('cherryPick', detail.oid),
      { destructive: false },
    );
  }

  protected requestRevert(detail: RepositoryCommitDetailResponse): void {
    if (detail.parents.length > 1 || this.workspaceActionBusy()) {
      return;
    }
    this.requestDestructiveAction(
      'Revert this commit?',
      `Create a new commit that reverses ${this.shortOid(detail.oid)} on the current local branch. The index and working tree must be clean.`,
      'Revert commit',
      () => this.runCommitOperation('revert', detail.oid),
      { destructive: false },
    );
  }

  protected requestReset(detail: RepositoryCommitDetailResponse): void {
    if (this.workspaceActionBusy()) {
      return;
    }
    const mode = this.resetMode();
    const descriptions: Readonly<Record<ResetMode, string>> = {
      soft: `Move the current branch to ${this.shortOid(detail.oid)} and keep the index and working tree unchanged.`,
      mixed: `Move the current branch to ${this.shortOid(detail.oid)}, unstage tracked changes, and keep working-tree files.`,
      hard: `Move the current branch to ${this.shortOid(detail.oid)} and permanently discard tracked staged and unstaged changes. Untracked files are preserved.`,
    };
    this.requestDestructiveAction(
      `Reset ${mode} to this commit?`,
      descriptions[mode],
      `Reset ${mode}`,
      () => this.runCommitOperation('reset', detail.oid, mode),
      { destructive: mode === 'hard' },
    );
  }

  private async runCommitOperation(
    action: 'cherryPick' | 'revert' | 'reset',
    targetOid: string,
    resetMode: ResetMode = 'mixed',
  ): Promise<void> {
    const state = this.statusStore.state();
    if (state.kind !== 'ready' || this.commitOperation() !== null) {
      return;
    }
    this.navigationActionError.set('');
    this.navigationActionNotice.set('');
    this.commitOperation.set(action);
    try {
      const result = action === 'cherryPick'
        ? await this.ipc.invoke('repository_cherry_pick_commit', {
            repositoryId: this.repositoryId,
            operation: {
              targetOid,
              precondition: this.repositoryStatePrecondition(state.status),
            },
          })
        : action === 'revert'
          ? await this.ipc.invoke('repository_revert_commit', {
              repositoryId: this.repositoryId,
              operation: {
                targetOid,
                precondition: this.repositoryStatePrecondition(state.status),
              },
            })
          : await this.ipc.invoke('repository_reset_commit', {
              repositoryId: this.repositoryId,
              operation: {
                targetOid,
                mode: resetMode,
                confirmHardReset: resetMode === 'hard',
                precondition: this.repositoryStatePrecondition(state.status),
              },
            });
      await this.handleCommitOperationResult(action, result);
    } catch (error) {
      this.navigationActionError.set(
        this.errorMessage(
          error,
          action === 'cherryPick'
            ? 'The commit could not be cherry-picked.'
            : action === 'revert'
              ? 'The commit could not be reverted.'
              : 'The branch could not be reset.',
        ),
      );
      await this.refreshHistoryContext(false);
    } finally {
      this.commitOperation.set(null);
    }
  }

  private async handleCommitOperationResult(
    action: 'cherryPick' | 'revert' | 'reset',
    result: RepositoryCommitOperationResponse,
  ): Promise<void> {
    if (result.status !== null) {
      this.statusStore.acceptMutationResult(result.status);
    }
    this.clearWorkingTreeMutationView();

    if (result.state === 'succeeded') {
      await this.refreshHistoryContext(false, result.status === null);
      this.navigationActionNotice.set(
        action === 'cherryPick'
          ? `Cherry-picked ${this.shortOid(result.targetOid)}.`
          : action === 'revert'
            ? `Reverted ${this.shortOid(result.targetOid)} in a new commit.`
            : `Reset the current branch to ${this.shortOid(result.targetOid)}.`,
      );
      return;
    }

    if (result.state === 'conflicted') {
      await Promise.all([
        this.loadNavigation(),
        this.reloadHistory(),
        this.loadConflicts(),
      ]);
      this.selectWorkingTree();
      this.navigationActionError.set(
        `${action === 'cherryPick' ? 'Cherry-pick' : 'Revert'} stopped on conflicts. ${result.errorMessage ?? 'Resolve the conflicted files before continuing.'}`,
      );
      return;
    }

    await this.refreshHistoryContext(false);
    this.selectWorkingTreeWhenChanged();
    this.navigationActionError.set(
      `The ${action === 'cherryPick' ? 'cherry-pick' : action} outcome could not be verified. ${result.errorMessage ?? 'Inspect the refreshed branch and working tree before retrying.'}`,
    );
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
      await this.refreshHistoryContext(false, false);
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
    if (rewritesUpstreamCommit) {
      this.requestDestructiveAction(
        'Amend published commit?',
        'HEAD is already part of the upstream history. Amending it rewrites published history and the next push may require force.',
        'Amend commit',
        () => this.amendCommitConfirmed(message, true),
      );
      return;
    }

    await this.amendCommitConfirmed(message, false);
  }

  private async amendCommitConfirmed(
    message: string | null,
    confirmUpstreamRewrite: boolean,
  ): Promise<void> {
    const state = this.statusStore.state();
    if (
      state.kind !== 'ready' ||
      !this.amendMode() ||
      this.amendUnavailable() ||
      (message !== null && message.trim().length === 0)
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
          confirmUpstreamRewrite,
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
        await this.refreshHistoryContext(false, false);
        this.restoreFocusAfterRender(this.amendReturnFocus);
        this.amendReturnFocus = null;
      } else {
        this.workingTreeMutationError.set(
          `The amend outcome is unknown. ${result.errorMessage ?? 'Inspect HEAD and the working tree before deciding whether to retry.'}`,
        );
        await this.refreshHistoryContext();
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

  private showDialogAfterRender(
    dialog: HTMLDialogElement | undefined,
    shouldOpen: () => boolean,
  ): void {
    if (dialog === undefined || dialog.open) {
      return;
    }
    globalThis.queueMicrotask(() => {
      if (!dialog.isConnected || dialog.open || !shouldOpen()) {
        return;
      }
      try {
        dialog.showModal();
      } catch {
        dialog.setAttribute('open', '');
      }
    });
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

  protected async openStashFileDiff(file: StashChangedFile): Promise<void> {
    this.fileDiffReturnFocus =
      globalThis.document?.activeElement instanceof HTMLElement
        ? globalThis.document.activeElement
        : null;
    await this.loadStashFileDiff(file.path, file.oldPath, file.source);
  }

  protected async retryFileDiff(path: string, oldPath: string | null): Promise<void> {
    if (this.historySelection() === 'working-tree') {
      const entryKind = this.selectedWorkingTreeEntryKind();
      if (entryKind !== null) {
        await this.loadWorkingTreeFileDiff(path, oldPath, entryKind);
      }
    } else if (this.historySelection() === 'stash') {
      const source = this.selectedStashFileSource();
      if (source !== null) {
        await this.loadStashFileDiff(path, oldPath, source);
      }
    } else if (this.historySelection() === 'commit') {
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
    this.commanderContext.select(`working-tree-file:${path}|entry:${entryKind}`);
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
    if (oid === null || this.historySelection() !== 'commit') {
      return;
    }

    const generation = ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(path);
    this.commanderContext.select(`commit-file:${path}|commit:${oid}`);
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

  private async loadStashFileDiff(
    path: string,
    oldPath: string | null,
    source: StashFileSource,
  ): Promise<void> {
    const oid = this.selectedOid();
    if (oid === null || this.historySelection() !== 'stash') {
      return;
    }

    const generation = ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(path);
    this.commanderContext.select(`stash-file:${path}|stash:${oid}|source:${source}`);
    this.selectedWorkingTreeEntryKind.set(null);
    this.selectedStashFileSource.set(source);
    this.fileDiffDisplayMode.set('contextual');
    this.fileDiffState.set({ kind: 'loading', path, oldPath });
    try {
      const response = await this.ipc.invoke('repository_stash_file_diff', {
        repositoryId: this.repositoryId,
        oid,
        source,
        path,
        oldPath,
      });
      if (
        generation === this.fileDiffRequestGeneration &&
        this.historySelection() === 'stash' &&
        this.selectedOid() === oid &&
        this.selectedFilePath() === path &&
        this.selectedStashFileSource() === source
      ) {
        this.fileDiffState.set({ kind: 'ready', response });
      }
    } catch (error) {
      if (
        generation === this.fileDiffRequestGeneration &&
        this.historySelection() === 'stash' &&
        this.selectedOid() === oid &&
        this.selectedFilePath() === path &&
        this.selectedStashFileSource() === source
      ) {
        this.fileDiffState.set({
          kind: 'error',
          path,
          oldPath,
          message: this.errorMessage(error, 'The stash file diff could not be loaded.'),
        });
      }
    }
  }

  protected closeFileDiff(): void {
    ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(null);
    const selection = this.historySelection();
    const oid = this.selectedOid();
    this.commanderContext.select(
      selection === 'commit' && oid !== null
        ? `commit:${oid}`
        : selection === 'stash' && oid !== null
          ? `stash:${oid}`
          : selection === 'working-tree'
            ? 'working-tree:current'
            : null,
    );
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

  private selectedDetailFiles(): readonly CommitChangedFile[] | null {
    if (this.historySelection() === 'stash') {
      const state = this.stashDetailState();
      return state.kind === 'ready' ? state.detail.files : null;
    }
    const state = this.detailState();
    return state.kind === 'ready' ? state.detail.files : null;
  }

  private reconcileSelectedStash(stashes: readonly RepositoryStash[]): void {
    if (this.historySelection() !== 'stash') {
      return;
    }
    const selected = stashes.find((stash) => stash.oid === this.selectedOid());
    if (selected !== undefined) {
      this.selectedStash.set(selected);
      return;
    }
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.historySelection.set('none');
    this.selectedOid.set(null);
    this.selectedStash.set(null);
    this.stashDetailState.set({ kind: 'idle' });
    this.selectedFilePath.set(null);
    this.selectedStashFileSource.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.fileDiffDisplayMode.set('contextual');
    this.fileDiffReturnFocus = null;
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

  private async loadWorktreeDirtyStates(): Promise<void> {
    const generation = ++this.dirtyStatesRequestGeneration;
    try {
      const response: WorktreeDirtyStatesResponse = await this.ipc.invoke(
        'repository_worktree_dirty_states',
        { repositoryId: this.repositoryId },
      );
      if (generation !== this.dirtyStatesRequestGeneration || this.destroyed) {
        return;
      }
      this.worktreeDirtyStates.set(new Map(
        response.states
          .filter((state): state is WorktreeDirtyStateResponse & { readonly branchFullName: string } =>
            state.branchFullName !== null && state.dirty && state.errorMessage == null)
          .map((state) => [state.branchFullName, state]),
      ));
    } catch {
      if (generation === this.dirtyStatesRequestGeneration && !this.destroyed) {
        this.worktreeDirtyStates.set(new Map());
      }
    }
  }

  private readonly closeBranchContextMenuFromOutside = (event: PointerEvent): void => {
    if (this.branchContextMenu() === null && this.worktreeContextMenu() === null) {
      return;
    }
    const target = event.target;
    if (target instanceof Element && target.closest('.branch-context-menu, .branch-menu-trigger') !== null) {
      return;
    }
    this.closeBranchContextMenu();
    this.closeWorktreeContextMenu();
  };

  private readonly handleBranchContextMenuKeydown = (event: KeyboardEvent): void => {
    if (
      event.key !== 'Escape' ||
      (this.branchContextMenu() === null && this.worktreeContextMenu() === null)
    ) {
      return;
    }
    event.preventDefault();
    if (this.worktreeContextMenu() !== null) {
      this.closeWorktreeContextMenu(true);
    } else {
      this.closeBranchContextMenu(true);
    }
  };

  private closeDialog(dialog: HTMLDialogElement | undefined): void {
    if (dialog === undefined) {
      return;
    }
    if (typeof dialog.close === 'function') {
      dialog.close();
    } else {
      dialog.removeAttribute('open');
    }
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

  private readonly resizeInspector = (event: PointerEvent): void => {
    if (event.pointerId === this.activeInspectorResizePointer) {
      this.setInspectorWidth(globalThis.innerWidth - event.clientX, false);
    }
  };

  private readonly finishInspectorResize = (event: PointerEvent): void => {
    if (event.pointerId !== this.activeInspectorResizePointer) {
      return;
    }
    this.persistInspectorWidth();
    this.stopInspectorResize();
  };

  private readonly cancelInspectorResize = (): void => {
    if (this.activeInspectorResizePointer !== null) {
      this.persistInspectorWidth();
      this.stopInspectorResize();
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

  private readonly clampPanelsToViewport = (): void => {
    this.setSidebarWidth(this.sidebarWidth(), false);
    this.setInspectorWidth(this.inspectorWidth(), false);
  };

  private stopSidebarResize(): void {
    this.activeResizePointer = null;
    this.resizingSidebar.set(false);
    globalThis.removeEventListener('pointermove', this.resizeSidebar);
    globalThis.removeEventListener('pointerup', this.finishSidebarResize);
    globalThis.removeEventListener('pointercancel', this.finishSidebarResize);
    globalThis.removeEventListener('blur', this.cancelSidebarResize);
  }

  private stopInspectorResize(): void {
    this.activeInspectorResizePointer = null;
    this.resizingInspector.set(false);
    globalThis.removeEventListener('pointermove', this.resizeInspector);
    globalThis.removeEventListener('pointerup', this.finishInspectorResize);
    globalThis.removeEventListener('pointercancel', this.finishInspectorResize);
    globalThis.removeEventListener('blur', this.cancelInspectorResize);
  }

  private setSidebarWidth(width: number, persist: boolean): void {
    this.sidebarWidth.set(Math.min(this.maximumSidebarWidth(), Math.max(SIDEBAR_MIN_WIDTH, width)));
    if (persist) {
      this.persistSidebarWidth();
    }
  }

  private setInspectorWidth(width: number, persist: boolean): void {
    this.inspectorWidth.set(Math.min(this.maximumInspectorWidth(), Math.max(INSPECTOR_MIN_WIDTH, width)));
    if (persist) {
      this.persistInspectorWidth();
    }
  }

  private maximumInspectorWidth(): number {
    if (globalThis.innerWidth <= INSPECTOR_LAYOUT_BREAKPOINT) {
      return INSPECTOR_DEFAULT_WIDTH;
    }
    const reservedWidth = this.sidebarWidth() + HISTORY_MIN_WIDTH + 2 * SIDEBAR_RESIZER_WIDTH;
    return Math.max(INSPECTOR_MIN_WIDTH, Math.floor(globalThis.innerWidth - reservedWidth));
  }

  private maximumSidebarWidth(): number {
    const viewportWidth = globalThis.innerWidth;
    let reservedWidth = 0;
    if (viewportWidth > INSPECTOR_LAYOUT_BREAKPOINT) {
      reservedWidth = HISTORY_MIN_WIDTH + INSPECTOR_MIN_WIDTH + 2 * SIDEBAR_RESIZER_WIDTH;
    } else if (viewportWidth > COMPACT_LAYOUT_BREAKPOINT) {
      reservedWidth = HISTORY_MIN_WIDTH + SIDEBAR_RESIZER_WIDTH;
    }
    const availableWidth = viewportWidth - reservedWidth;
    return Math.max(
      SIDEBAR_MIN_WIDTH,
      Math.floor(Math.min(viewportWidth * 0.5, availableWidth)),
    );
  }

  protected isCurrentWorktree(worktree: RepositoryWorktree): boolean {
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

  private worktreeForBranch(branchFullName: string): RepositoryWorktree | null {
    const navigation = this.navigationState();
    if (navigation.kind !== 'ready') {
      return null;
    }
    return navigation.navigation.worktrees.find(
      (worktree) =>
        !this.isCurrentWorktree(worktree) &&
        !worktree.bare &&
        !worktree.prunable &&
        this.worktreeBranchFullName(worktree) === branchFullName,
    ) ?? null;
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

  private readInspectorWidth(): number {
    try {
      const stored = Number(globalThis.localStorage?.getItem(INSPECTOR_WIDTH_KEY));
      return Number.isFinite(stored) && stored > 0
        ? Math.min(this.maximumInspectorWidth(), Math.max(INSPECTOR_MIN_WIDTH, stored))
        : INSPECTOR_DEFAULT_WIDTH;
    } catch {
      return INSPECTOR_DEFAULT_WIDTH;
    }
  }

  private persistSidebarWidth(): void {
    try {
      globalThis.localStorage?.setItem(SIDEBAR_WIDTH_KEY, String(this.sidebarWidth()));
    } catch {
      // Storage may be unavailable in a hardened webview. Resizing remains usable for the session.
    }
  }

  private persistInspectorWidth(): void {
    try {
      globalThis.localStorage?.setItem(INSPECTOR_WIDTH_KEY, String(this.inspectorWidth()));
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
    // Submodules are independent repositories.  Their inspection must not
    // delay opening the parent workspace or its history.
    void this.loadSubmodules();
    const statusRefresh = this.statusStore.refresh();
    await this.loadNavigation();
    await Promise.all([
      statusRefresh,
      this.reloadHistory(),
      this.loadPushAnalysis(),
      this.loadConflicts(),
    ]);
    this.selectWorkingTreeWhenChanged();
  }
}
