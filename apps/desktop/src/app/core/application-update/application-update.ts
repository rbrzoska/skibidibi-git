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

@Service()
export class ApplicationUpdate {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly feedback = inject(UiFeedback);
  private initialized = false;

  readonly state = signal<ApplicationUpdateState>('idle');
  readonly currentVersion = signal<string | null>(null);
  readonly availableUpdate = signal<ApplicationUpdateInfo | null>(null);
  readonly lastCheckedAt = signal<Date | null>(null);
  readonly autoCheck = signal(readAutoCheckPreference());
  readonly error = signal<string | null>(null);
  readonly promptVisible = signal(false);
  readonly triggerLabel = computed(() => {
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
    if (this.autoCheck()) {
      void this.check(false);
    }
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
      this.lastCheckedAt.set(new Date());
      this.state.set(result.update === null ? 'upToDate' : 'available');
      this.promptVisible.set(result.update !== null);
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
    this.promptVisible.set(false);
  }

  trigger(): void {
    if (this.state() === 'available' || this.state() === 'restartReady') {
      this.promptVisible.set(true);
      return;
    }
    void this.check();
  }

  async install(): Promise<void> {
    const update = this.availableUpdate();
    if (update === null || this.state() === 'installing') {
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
}

function readAutoCheckPreference(): boolean {
  try {
    return globalThis.localStorage?.getItem(AUTO_CHECK_STORAGE_KEY) !== 'false';
  } catch {
    return true;
  }
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
