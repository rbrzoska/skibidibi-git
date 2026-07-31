import { ChangeDetectionStrategy, Component, computed, inject, signal, viewChild } from '@angular/core';
import { RouterLink } from '@angular/router';

import { AiSupportStore } from '../../../core/ai-support/ai-support.store';
import { ApplicationUpdate } from '../../../core/application-update/application-update';
import {
  DESKTOP_IPC,
  type DiagnosticsSettingsResponse,
  type RepositoryMaintenanceStatisticsResponse,
} from '../../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../../core/repositories/repository-catalog';
import { GitHubAccountControl } from '../../github';
import { SafeMarkdown } from '../../github/pull-request-inspector/safe-markdown/safe-markdown';
import { DiagnosticLogDialog } from '../diagnostic-log-dialog/diagnostic-log-dialog';
import {
  browserWorkspaceRefreshStorage,
  readGlobalWorkspaceRefreshPreferences,
  writeGlobalWorkspaceRefreshSetting,
  type WorkspaceRefreshSetting,
} from '../../workspace-history/workspace-refresh-policy';

type RepositoryMaintenanceStats = RepositoryMaintenanceStatisticsResponse;
type RepositoryMaintenanceWorktreeStats = RepositoryMaintenanceStatisticsResponse['worktrees'][number];

type MaintenanceScanState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'error'; readonly message: string }
  | { readonly kind: 'ready'; readonly stats: RepositoryMaintenanceStats };

@Component({
  selector: 'app-settings-page',
  imports: [DiagnosticLogDialog, GitHubAccountControl, RouterLink, SafeMarkdown],
  templateUrl: './settings-page.html',
  styleUrl: './settings-page.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SettingsPage {
  protected readonly catalog = inject(RepositoryCatalog);
  protected readonly aiSupport = inject(AiSupportStore);
  protected readonly applicationUpdate = inject(ApplicationUpdate);
  private readonly maintenanceIpc = inject(DESKTOP_IPC);
  private readonly diagnosticLogDialog = viewChild.required(DiagnosticLogDialog);
  private readonly storage = browserWorkspaceRefreshStorage();
  private readonly initialRefreshDefaults = readGlobalWorkspaceRefreshPreferences(this.storage);

  protected readonly currentOnly = signal(this.initialRefreshDefaults.currentOnly);
  protected readonly autoFetch = signal(this.initialRefreshDefaults.autoFetch);
  protected readonly liveChanges = signal(this.initialRefreshDefaults.liveChanges);
  protected readonly maintenanceScans = signal<ReadonlyMap<string, MaintenanceScanState>>(new Map());
  protected readonly scanAllRunning = signal(false);
  protected readonly diagnosticsSettings = signal<DiagnosticsSettingsResponse | null>(null);
  protected readonly diagnosticsLoading = signal(false);
  protected readonly diagnosticsSaving = signal(false);
  protected readonly diagnosticsError = signal<string | null>(null);
  protected readonly diagnosticLogLimits = [64, 128, 256, 512, 1024, 2048, 4096, 8192] as const;
  protected readonly maintenanceRepositories = computed(() => {
    const states = this.maintenanceScans();
    return this.catalog.repositories()
      .map((repository, index) => ({ repository, index }))
      .sort((left, right) => compareRepositoryMaintenanceAge(
        states.get(left.repository.id),
        states.get(right.repository.id),
      ) || left.index - right.index)
      .map(({ repository }) => repository);
  });

  constructor() {
    void this.aiSupport.loadAvailability();
    void this.loadDiagnosticsSettings();
    if (this.catalog.state().kind === 'idle') {
      void this.catalog.load();
    }
  }

  protected async chooseDiagnosticsDirectory(): Promise<void> {
    const current = this.diagnosticsSettings();
    if (current === null || this.diagnosticsSaving()) {
      return;
    }
    this.diagnosticsError.set(null);
    try {
      const selection = await this.maintenanceIpc.invoke('select_diagnostics_directory', {
        initialPath: current.dataDirectory,
      });
      if (selection.path !== null) {
        await this.updateDiagnosticsSettings(selection.path, current.maxLogKilobytes);
      }
    } catch (error) {
      this.diagnosticsError.set(messageFrom(error, 'The data directory could not be selected.'));
    }
  }

  protected async setDiagnosticLogLimit(value: string): Promise<void> {
    const current = this.diagnosticsSettings();
    const maxLogKilobytes = Number(value);
    if (current === null || !this.diagnosticLogLimits.includes(maxLogKilobytes as typeof this.diagnosticLogLimits[number])) {
      return;
    }
    await this.updateDiagnosticsSettings(current.dataDirectory, maxLogKilobytes);
  }

  protected async viewDiagnosticLog(): Promise<void> {
    if (this.diagnosticsLoading()) {
      return;
    }
    this.diagnosticsLoading.set(true);
    this.diagnosticsError.set(null);
    try {
      this.diagnosticLogDialog().open(await this.maintenanceIpc.invoke('diagnostics_read', {}));
    } catch (error) {
      this.diagnosticsError.set(messageFrom(error, 'The diagnostic log could not be loaded.'));
    } finally {
      this.diagnosticsLoading.set(false);
    }
  }

  protected async clearDiagnosticLog(): Promise<void> {
    if (this.diagnosticsLoading()) {
      return;
    }
    this.diagnosticsLoading.set(true);
    this.diagnosticsError.set(null);
    try {
      await this.maintenanceIpc.invoke('diagnostics_clear', {});
    } catch (error) {
      this.diagnosticsError.set(messageFrom(error, 'The diagnostic log could not be cleared.'));
    } finally {
      this.diagnosticsLoading.set(false);
    }
  }

  private async loadDiagnosticsSettings(): Promise<void> {
    this.diagnosticsLoading.set(true);
    try {
      this.diagnosticsSettings.set(await this.maintenanceIpc.invoke('diagnostics_settings', {}));
    } catch (error) {
      this.diagnosticsError.set(messageFrom(error, 'Diagnostics settings could not be loaded.'));
    } finally {
      this.diagnosticsLoading.set(false);
    }
  }

  private async updateDiagnosticsSettings(dataDirectory: string, maxLogKilobytes: number): Promise<void> {
    this.diagnosticsSaving.set(true);
    this.diagnosticsError.set(null);
    try {
      this.diagnosticsSettings.set(await this.maintenanceIpc.invoke('diagnostics_update_settings', {
        dataDirectory,
        maxLogKilobytes,
      }));
    } catch (error) {
      this.diagnosticsError.set(messageFrom(error, 'Diagnostics settings could not be saved.'));
    } finally {
      this.diagnosticsSaving.set(false);
    }
  }

  protected setRefreshDefault(setting: WorkspaceRefreshSetting, enabled: boolean): void {
    this[setting].set(writeGlobalWorkspaceRefreshSetting(this.storage, setting, enabled));
  }

  protected availabilityLabel(availability: string): string {
    switch (availability) {
      case 'available': return 'Available';
      case 'missing': return 'Missing';
      case 'inaccessible': return 'Inaccessible';
      default: return 'Not checked';
    }
  }

  protected maintenanceState(repositoryId: string): MaintenanceScanState {
    return this.maintenanceScans().get(repositoryId) ?? { kind: 'idle' };
  }

  protected async scanRepository(repositoryId: string): Promise<void> {
    if (this.maintenanceState(repositoryId).kind === 'loading') {
      return;
    }
    this.setMaintenanceState(repositoryId, { kind: 'loading' });
    try {
      const stats = await this.maintenanceIpc.invoke('repository_maintenance_stats', {
        repositoryId,
      });
      this.setMaintenanceState(repositoryId, {
        kind: 'ready',
        stats: {
          ...stats,
          worktrees: [...stats.worktrees].sort(compareWorktreeCommitAge),
        },
      });
    } catch (error) {
      this.setMaintenanceState(repositoryId, {
        kind: 'error',
        message: error instanceof Error && error.message.trim().length > 0
          ? error.message
          : 'Repository statistics could not be scanned.',
      });
    }
  }

  protected async scanAllRepositories(): Promise<void> {
    if (this.scanAllRunning()) {
      return;
    }
    this.scanAllRunning.set(true);
    try {
      for (const repository of this.catalog.repositories()) {
        if (repository.availability === 'available') {
          await this.scanRepository(repository.id);
        }
      }
    } finally {
      this.scanAllRunning.set(false);
    }
  }

  protected formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) {
      return 'Unavailable';
    }
    const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'] as const;
    let value = bytes;
    let unitIndex = 0;
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex += 1;
    }
    const digits = unitIndex === 0 || value >= 10 ? 0 : 1;
    return `${value.toFixed(digits)} ${units[unitIndex]}`;
  }

  protected formatTimestamp(timestamp: number | null): string {
    if (timestamp === null || !Number.isFinite(timestamp) || timestamp < 0) {
      return 'Not recorded';
    }
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(timestamp * 1000));
  }

  protected timestampIso(timestamp: number | null): string | null {
    if (timestamp === null || !Number.isFinite(timestamp) || timestamp < 0) {
      return null;
    }
    return new Date(timestamp * 1000).toISOString();
  }

  protected formatCommitTimestamp(timestamp: string | null): string {
    const parsed = parseCommitTimestamp(timestamp);
    if (parsed === null) {
      return 'Not recorded';
    }
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(parsed));
  }

  protected commitTimestampIso(timestamp: string | null): string | null {
    const parsed = parseCommitTimestamp(timestamp);
    return parsed === null ? null : new Date(parsed).toISOString();
  }

  protected staleLabel(lastCommitAt: string | null, scannedAt: number): string {
    const lastCommitMillis = parseCommitTimestamp(lastCommitAt);
    if (lastCommitMillis === null) {
      return 'Commit age unknown';
    }
    const ageDays = Math.max(0, Math.floor((scannedAt * 1000 - lastCommitMillis) / 86_400_000));
    if (ageDays > 365) {
      return `Very old · ${ageDays}d`;
    }
    if (ageDays > 90) {
      return `Stale · ${ageDays}d`;
    }
    return `Active · ${ageDays}d`;
  }

  private setMaintenanceState(repositoryId: string, state: MaintenanceScanState): void {
    this.maintenanceScans.update((states) => {
      const next = new Map(states);
      next.set(repositoryId, state);
      return next;
    });
  }
}

function compareWorktreeCommitAge(
  left: RepositoryMaintenanceWorktreeStats,
  right: RepositoryMaintenanceWorktreeStats,
): number {
  const leftTimestamp = parseCommitTimestamp(left.lastCommitAt);
  const rightTimestamp = parseCommitTimestamp(right.lastCommitAt);
  if (leftTimestamp === null) {
    return rightTimestamp === null ? left.path.localeCompare(right.path) : 1;
  }
  if (rightTimestamp === null) {
    return -1;
  }
  return leftTimestamp - rightTimestamp || left.path.localeCompare(right.path);
}

function compareRepositoryMaintenanceAge(
  left: MaintenanceScanState | undefined,
  right: MaintenanceScanState | undefined,
): number {
  const leftTimestamp = left?.kind === 'ready'
    ? parseCommitTimestamp(left.stats.repositoryLastCommitAt)
    : null;
  const rightTimestamp = right?.kind === 'ready'
    ? parseCommitTimestamp(right.stats.repositoryLastCommitAt)
    : null;
  const leftRank = left?.kind === 'ready' ? (leftTimestamp === null ? 1 : 0) : 2;
  const rightRank = right?.kind === 'ready' ? (rightTimestamp === null ? 1 : 0) : 2;
  return leftRank - rightRank || (leftTimestamp ?? 0) - (rightTimestamp ?? 0);
}

function parseCommitTimestamp(timestamp: string | null): number | null {
  if (timestamp === null || timestamp.trim().length === 0) {
    return null;
  }
  const parsed = Date.parse(timestamp);
  return Number.isNaN(parsed) ? null : parsed;
}

function messageFrom(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim().length > 0 ? error.message : fallback;
}
