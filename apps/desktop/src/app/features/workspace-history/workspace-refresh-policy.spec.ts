import { describe, expect, it } from 'vitest';

import {
  AUTO_FETCH_INTERVAL_MS,
  buildWipStashMessage,
  LIVE_STATUS_INTERVAL_MS,
  MAX_WIP_STASH_MESSAGE_LENGTH,
  parsePersistedBoolean,
  readWorkspaceRefreshPreferences,
  workspaceRefreshStorageKey,
  writeWorkspaceRefreshPreferences,
  writeWorkspaceRefreshSetting,
  type WorkspaceRefreshStorage,
} from './workspace-refresh-policy';

class MemoryStorage implements WorkspaceRefreshStorage {
  readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

describe('workspace refresh policy', () => {
  it('defines the bounded refresh intervals', () => {
    expect(AUTO_FETCH_INTERVAL_MS).toBe(60_000);
    expect(LIVE_STATUS_INTERVAL_MS).toBe(2_000);
  });

  it('keeps every preference repository-scoped', () => {
    const storage = new MemoryStorage();
    writeWorkspaceRefreshPreferences(storage, 'repo/a', {
      currentOnly: true,
      autoFetch: true,
      liveChanges: false,
    });
    writeWorkspaceRefreshSetting(storage, 'repo/b', 'liveChanges', true);

    expect(readWorkspaceRefreshPreferences(storage, 'repo/a')).toEqual({
      currentOnly: true,
      autoFetch: true,
      liveChanges: false,
    });
    expect(readWorkspaceRefreshPreferences(storage, 'repo/b')).toEqual({
      currentOnly: false,
      autoFetch: false,
      liveChanges: true,
    });
    expect(workspaceRefreshStorageKey('repo/a', 'autoFetch')).not.toBe(
      workspaceRefreshStorageKey('repo/b', 'autoFetch'),
    );
  });

  it('only accepts exact persisted booleans and otherwise uses the fallback', () => {
    expect(parsePersistedBoolean('true')).toBe(true);
    expect(parsePersistedBoolean('false', true)).toBe(false);
    expect(parsePersistedBoolean('TRUE')).toBe(false);
    expect(parsePersistedBoolean('1', true)).toBe(true);
    expect(parsePersistedBoolean(null, true)).toBe(true);
  });

  it('recovers independently from malformed values', () => {
    const storage = new MemoryStorage();
    storage.setItem(workspaceRefreshStorageKey('repo', 'currentOnly'), 'true');
    storage.setItem(workspaceRefreshStorageKey('repo', 'autoFetch'), '{bad json');
    storage.setItem(workspaceRefreshStorageKey('repo', 'liveChanges'), 'false');

    expect(readWorkspaceRefreshPreferences(storage, 'repo', {
      currentOnly: false,
      autoFetch: true,
      liveChanges: true,
    })).toEqual({
      currentOnly: true,
      autoFetch: true,
      liveChanges: false,
    });
  });

  it('never propagates storage read or write failures', () => {
    const storage: WorkspaceRefreshStorage = {
      getItem: () => {
        throw new Error('blocked read');
      },
      setItem: () => {
        throw new Error('blocked write');
      },
      removeItem: () => {
        throw new Error('blocked cleanup');
      },
    };

    expect(readWorkspaceRefreshPreferences(storage, 'repo', {
      currentOnly: true,
      autoFetch: false,
      liveChanges: true,
    })).toEqual({ currentOnly: true, autoFetch: false, liveChanges: true });
    expect(writeWorkspaceRefreshSetting(storage, 'repo', 'autoFetch', true)).toBe(true);
    expect(writeWorkspaceRefreshPreferences(storage, 'repo', {
      currentOnly: false,
      autoFetch: true,
      liveChanges: false,
    })).toEqual({ currentOnly: false, autoFetch: true, liveChanges: false });
  });

  it('formats the WIP stash message with a second-precision ISO timestamp', () => {
    expect(buildWipStashMessage(
      '  rb/DEV-123  ',
      new Date('2026-07-15T09:08:07.654Z'),
    )).toBe('WIP 2026-07-15T09:08:07 rb/DEV-123');
  });

  it.each(['', '   ', 'main\nmalicious', 'main\u007fmalicious'])(
    'rejects an invalid branch in a WIP stash message: %j',
    (branch) => {
      expect(() => buildWipStashMessage(branch)).toThrow();
    },
  );

  it('rejects invalid timestamps and messages beyond the fixed bound', () => {
    expect(() => buildWipStashMessage('main', new Date(Number.NaN))).toThrow();
    const fixedPrefixLength = 'WIP 2026-07-15T09:08:07 '.length;
    const tooLongBranch = 'x'.repeat(MAX_WIP_STASH_MESSAGE_LENGTH - fixedPrefixLength + 1);

    expect(() => buildWipStashMessage(
      tooLongBranch,
      new Date('2026-07-15T09:08:07Z'),
    )).toThrow(RangeError);
  });
});
