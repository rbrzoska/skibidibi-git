import { Service, computed, inject, signal } from '@angular/core';

import {
  DESKTOP_IPC,
  type ApplicationUpdateInfo,
} from '../ipc/desktop-ipc';
import { UiFeedback } from '../ui-feedback/ui-feedback';

export type ApplicationUpdateState =
  | 'idle'
  | 'checking'
  | 'upToDate'
  | 'available'
  | 'installing'
  | 'restartReady'
  | 'restarting'
  | 'error';

const AUTO_CHECK_STORAGE_KEY = 'skibidibi-git.application-update.auto-check.v1';
const LAST_SEEN_VERSION_STORAGE_KEY = 'skibidibi-git.application-update.last-seen-version.v1';

@Service()
export class ApplicationUpdate {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly feedback = inject(UiFeedback);
  private initialized = false;

  readonly state = signal<ApplicationUpdateState>('idle');
  readonly currentVersion = signal<string | null>(null);
  readonly availableUpdate = signal<ApplicationUpdateInfo | null>(null);
  readonly releaseNotesMarkdown = signal('');
  readonly managedByStore = signal(false);
  readonly lastCheckedAt = signal<Date | null>(null);
  readonly autoCheck = signal(readAutoCheckPreference());
  readonly error = signal<string | null>(null);
  readonly promptVisible = signal(false);
  readonly displayVersion = computed(() => (
    this.availableUpdate()?.version ?? this.currentVersion()
  ));
  readonly displayReleaseNotes = computed(() => {
    const availableNotes = this.availableUpdate()?.body?.trim();
    if (availableNotes) {
      return availableNotes;
    }
    return releaseNotesForVersion(
      this.releaseNotesMarkdown(),
      this.displayVersion(),
    );
  });
  readonly triggerLabel = computed(() => {
    if (this.managedByStore()) {
      return 'Microsoft Store';
    }
    switch (this.state()) {
      case 'checking': return 'Checking…';
      case 'upToDate': return 'Up to date';
      case 'available': return `Update ${this.availableUpdate()?.version ?? ''}`.trim();
      case 'installing': return 'Installing…';
      case 'restartReady': return 'Restart to update';
      case 'restarting': return 'Restarting…';
      default: return 'Check updates';
    }
  });
  readonly triggerDisabled = computed(() => (
    this.state() === 'checking'
    || this.state() === 'installing'
    || this.state() === 'restarting'
  ));

  initialize(): void {
    if (this.initialized) {
      return;
    }
    this.initialized = true;
    void this.initializeUpdateState();
  }

  setAutoCheck(enabled: boolean): void {
    this.autoCheck.set(enabled);
    try {
      globalThis.localStorage?.setItem(AUTO_CHECK_STORAGE_KEY, enabled ? 'true' : 'false');
    } catch {
      // The preference remains active for this session when storage is unavailable.
    }
  }

  async check(announceResult = true): Promise<void> {
    if (this.state() === 'checking' || this.state() === 'installing') {
      return;
    }
    this.state.set('checking');
    this.error.set(null);
    try {
      const result = await this.ipc.invoke('application_update_check', {});
      this.currentVersion.set(result.currentVersion);
      this.availableUpdate.set(result.update);
      this.managedByStore.set(result.managedByStore);
      this.lastCheckedAt.set(new Date());
      this.state.set(result.update === null ? 'upToDate' : 'available');
      if (result.update !== null) {
        this.promptVisible.set(true);
      }
      if (announceResult) {
        this.feedback.show(
          result.update === null ? 'success' : 'info',
          result.update === null ? 'Skibidibi Git is up to date' : 'Update available',
          result.update === null
            ? `Version ${result.currentVersion} is the newest available version.`
            : `Version ${result.update.version} is ready to install.`,
        );
      }
    } catch (error) {
      const message = messageFrom(error, 'The update service could not be reached.');
      this.error.set(message);
      this.state.set('error');
      if (announceResult) {
        this.feedback.show('error', 'Update check failed', message);
      }
    }
  }

  dismissAvailableUpdate(): void {
    if (this.availableUpdate() === null) {
      this.markInstalledVersionSeen();
    }
    this.promptVisible.set(false);
  }

  trigger(): void {
    if (this.managedByStore()) {
      this.showReleaseNotes();
      return;
    }
    if (
      this.state() === 'available'
      || this.state() === 'restartReady'
      || this.state() === 'upToDate'
    ) {
      this.promptVisible.set(true);
      return;
    }
    void this.check();
  }

  showReleaseNotes(): void {
    this.promptVisible.set(true);
  }

  async install(): Promise<void> {
    const update = this.availableUpdate();
    if (this.managedByStore() || update === null || this.state() === 'installing') {
      return;
    }
    this.state.set('installing');
    this.error.set(null);
    this.feedback.setLoading('application-update', true, 'Installing update…');
    try {
      await this.ipc.invoke('application_update_install', {
        expectedVersion: update.version,
      });
      this.state.set('restartReady');
      this.promptVisible.set(true);
      this.feedback.show(
        'success',
        'Update installed',
        `Version ${update.version} is ready. Restart Skibidibi Git when convenient.`,
      );
    } catch (error) {
      const message = messageFrom(error, 'The update could not be installed.');
      this.error.set(message);
      this.state.set('available');
      this.feedback.show('error', 'Update not installed', message);
    } finally {
      this.feedback.setLoading('application-update', false);
    }
  }

  async restart(): Promise<void> {
    if (this.state() !== 'restartReady') {
      return;
    }
    this.state.set('restarting');
    this.feedback.setLoading('application-update', true, 'Restarting Skibidibi Git…');
    try {
      await this.ipc.invoke('application_update_restart', {});
    } catch (error) {
      const message = messageFrom(error, 'The application could not be restarted.');
      this.error.set(message);
      this.state.set('restartReady');
      this.feedback.show('error', 'Restart failed', message);
      this.feedback.setLoading('application-update', false);
    }
  }

  private async initializeUpdateState(): Promise<void> {
    await this.loadReleaseNotes();
    if (this.autoCheck() && !this.managedByStore()) {
      await this.check(false);
    }
  }

  private async loadReleaseNotes(): Promise<void> {
    try {
      const result = await this.ipc.invoke('application_release_notes', {});
      this.currentVersion.set(result.currentVersion);
      this.releaseNotesMarkdown.set(result.markdown);
      this.managedByStore.set(result.managedByStore);
      if (readLastSeenVersion() !== result.currentVersion) {
        this.promptVisible.set(true);
      }
    } catch {
      // Update checks remain available if bundled release notes cannot be loaded.
    }
  }

  private markInstalledVersionSeen(): void {
    const version = this.currentVersion();
    if (version === null) {
      return;
    }
    try {
      globalThis.localStorage?.setItem(LAST_SEEN_VERSION_STORAGE_KEY, version);
    } catch {
      // The dialog can reappear next session when storage is unavailable.
    }
  }
}

function readAutoCheckPreference(): boolean {
  try {
    return globalThis.localStorage?.getItem(AUTO_CHECK_STORAGE_KEY) !== 'false';
  } catch {
    return true;
  }
}

function readLastSeenVersion(): string | null {
  try {
    return globalThis.localStorage?.getItem(LAST_SEEN_VERSION_STORAGE_KEY) ?? null;
  } catch {
    return null;
  }
}

export function releaseNotesForVersion(markdown: string, version: string | null): string {
  if (markdown.trim() === '' || version === null) {
    return '';
  }
  const headings = [...markdown.matchAll(/^##\s+v?(\S+)(?:\s|$).*$/gm)];
  const headingIndex = headings.findIndex((heading) => heading[1] === version);
  if (headingIndex < 0) {
    return '';
  }
  const start = headings[headingIndex]?.index;
  if (start === undefined) {
    return '';
  }
  const end = headings[headingIndex + 1]?.index ?? markdown.length;
  return markdown.slice(start, end).trim();
}

function messageFrom(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim() !== '') {
    return error.message;
  }
  if (typeof error === 'string' && error.trim() !== '') {
    return error;
  }
  return fallback;
}
