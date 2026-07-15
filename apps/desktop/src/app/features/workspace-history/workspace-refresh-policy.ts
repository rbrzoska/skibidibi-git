export const AUTO_FETCH_INTERVAL_MS = 60_000;
export const LIVE_STATUS_INTERVAL_MS = 2_000;

export const MAX_WIP_STASH_MESSAGE_LENGTH = 256;

const STORAGE_PREFIX: Record<WorkspaceRefreshSetting, string> = {
  currentOnly: 'skibidibi-git.workspace.current-only',
  autoFetch: 'skibidibi-git.workspace.auto-fetch',
  liveChanges: 'skibidibi-git.workspace.live-changes',
};

export type WorkspaceRefreshSetting = 'currentOnly' | 'autoFetch' | 'liveChanges';

export interface WorkspaceRefreshPreferences {
  readonly currentOnly: boolean;
  readonly autoFetch: boolean;
  readonly liveChanges: boolean;
}

export interface WorkspaceRefreshStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export const DEFAULT_WORKSPACE_REFRESH_PREFERENCES: WorkspaceRefreshPreferences = {
  currentOnly: false,
  autoFetch: false,
  liveChanges: false,
};

const SETTINGS: readonly WorkspaceRefreshSetting[] = [
  'currentOnly',
  'autoFetch',
  'liveChanges',
];

export function browserWorkspaceRefreshStorage(): WorkspaceRefreshStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function workspaceRefreshStorageKey(
  repositoryId: string,
  setting: WorkspaceRefreshSetting,
): string {
  return `${STORAGE_PREFIX[setting]}.${repositoryId}`;
}

export function parsePersistedBoolean(value: string | null, fallback = false): boolean {
  if (value === 'true') {
    return true;
  }
  if (value === 'false') {
    return false;
  }
  return fallback;
}

export function readWorkspaceRefreshPreferences(
  storage: WorkspaceRefreshStorage | null,
  repositoryId: string,
  fallback: WorkspaceRefreshPreferences = DEFAULT_WORKSPACE_REFRESH_PREFERENCES,
): WorkspaceRefreshPreferences {
  return {
    currentOnly: readSetting(storage, repositoryId, 'currentOnly', fallback.currentOnly),
    autoFetch: readSetting(storage, repositoryId, 'autoFetch', fallback.autoFetch),
    liveChanges: readSetting(storage, repositoryId, 'liveChanges', fallback.liveChanges),
  };
}

/**
 * Persists each setting independently. Failed browser storage is best-effort and
 * never changes the in-memory preferences returned to the caller.
 */
export function writeWorkspaceRefreshPreferences(
  storage: WorkspaceRefreshStorage | null,
  repositoryId: string,
  preferences: WorkspaceRefreshPreferences,
): WorkspaceRefreshPreferences {
  for (const setting of SETTINGS) {
    writeSetting(storage, repositoryId, setting, preferences[setting]);
  }

  return preferences;
}

export function writeWorkspaceRefreshSetting(
  storage: WorkspaceRefreshStorage | null,
  repositoryId: string,
  setting: WorkspaceRefreshSetting,
  enabled: boolean,
): boolean {
  writeSetting(storage, repositoryId, setting, enabled);
  return enabled;
}

export function buildWipStashMessage(branch: string, now = new Date()): string {
  const normalizedBranch = branch.trim();
  if (
    normalizedBranch.length === 0 ||
    containsControlCharacters(normalizedBranch) ||
    Number.isNaN(now.getTime())
  ) {
    throw new Error('A valid branch and timestamp are required for a WIP stash message.');
  }

  const timestamp = now.toISOString().slice(0, 19);
  const message = `WIP ${timestamp} ${normalizedBranch}`;
  if (message.length > MAX_WIP_STASH_MESSAGE_LENGTH) {
    throw new RangeError(`WIP stash messages cannot exceed ${MAX_WIP_STASH_MESSAGE_LENGTH} characters.`);
  }

  return message;
}

function containsControlCharacters(value: string): boolean {
  return [...value].some((character) => {
    const codePoint = character.codePointAt(0);
    return codePoint !== undefined && (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f));
  });
}

function readSetting(
  storage: WorkspaceRefreshStorage | null,
  repositoryId: string,
  setting: WorkspaceRefreshSetting,
  fallback: boolean,
): boolean {
  try {
    return parsePersistedBoolean(
      storage?.getItem(workspaceRefreshStorageKey(repositoryId, setting)) ?? null,
      fallback,
    );
  } catch {
    return fallback;
  }
}

function writeSetting(
  storage: WorkspaceRefreshStorage | null,
  repositoryId: string,
  setting: WorkspaceRefreshSetting,
  enabled: boolean,
): void {
  try {
    storage?.setItem(workspaceRefreshStorageKey(repositoryId, setting), String(enabled));
  } catch {
    // Persistence is best-effort; active repository controls remain usable.
  }
}
