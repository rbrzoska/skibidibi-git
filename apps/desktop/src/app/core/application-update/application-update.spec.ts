import { TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import { DESKTOP_IPC, type DesktopIpcClient } from '../ipc/desktop-ipc';
import { ApplicationUpdate, releaseNotesForVersion } from './application-update';

describe('ApplicationUpdate', () => {
  let service: ApplicationUpdate;
  const invoke = vi.fn<DesktopIpcClient['invoke']>();

  beforeEach(() => {
    globalThis.localStorage.clear();
    invoke.mockReset();
    TestBed.configureTestingModule({
      providers: [{ provide: DESKTOP_IPC, useValue: { invoke } }],
    });
    service = TestBed.inject(ApplicationUpdate);
  });

  afterEach(() => {
    globalThis.localStorage.clear();
  });

  it('checks once on startup and exposes a signed update for explicit installation', async () => {
    invoke.mockResolvedValueOnce({
      currentVersion: '0.1.0',
      markdown: '## 0.1.0\n\n- Installed',
      managedByStore: false,
    }).mockResolvedValueOnce({
      currentVersion: '0.1.0',
      update: { version: '0.2.0', body: 'Faster fetches', date: '2026-07-28T10:00:00Z' },
      managedByStore: false,
    });

    service.initialize();
    service.initialize();
    await vi.waitFor(() => expect(service.state()).toBe('available'));

    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenNthCalledWith(1, 'application_release_notes', {});
    expect(invoke).toHaveBeenNthCalledWith(2, 'application_update_check', {});
    expect(service.currentVersion()).toBe('0.1.0');
    expect(service.availableUpdate()?.version).toBe('0.2.0');
    expect(service.displayReleaseNotes()).toBe('Faster fetches');
  });

  it('persists disabling automatic startup checks while loading bundled release notes', async () => {
    invoke.mockResolvedValueOnce({
      currentVersion: '0.1.0',
      markdown: '## 0.1.0\n\n- Installed',
      managedByStore: false,
    });
    service.setAutoCheck(false);
    service.initialize();
    await vi.waitFor(() => expect(service.currentVersion()).toBe('0.1.0'));

    expect(service.autoCheck()).toBe(false);
    expect(globalThis.localStorage.getItem('skibidibi-git.application-update.auto-check.v1')).toBe('false');
    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith('application_release_notes', {});
    expect(service.displayReleaseNotes()).toContain('- Installed');
    expect(service.promptVisible()).toBe(true);

    service.dismissAvailableUpdate();

    expect(globalThis.localStorage.getItem('skibidibi-git.application-update.last-seen-version.v1')).toBe('0.1.0');
  });

  it('installs the exact selected version and waits for an explicit restart', async () => {
    invoke
      .mockResolvedValueOnce({
        currentVersion: '0.1.0',
        update: { version: '0.2.0', body: null, date: null },
        managedByStore: false,
      })
      .mockResolvedValueOnce(undefined);
    await service.check(false);

    await service.install();

    expect(invoke).toHaveBeenLastCalledWith('application_update_install', {
      expectedVersion: '0.2.0',
    });
    expect(service.state()).toBe('restartReady');
    expect(service.promptVisible()).toBe(true);
  });

  it('reopens a dismissed update from the persistent trigger', async () => {
    invoke.mockResolvedValueOnce({
      currentVersion: '0.1.0',
      update: { version: '0.2.0', body: null, date: null },
      managedByStore: false,
    });
    await service.check(false);
    service.dismissAvailableUpdate();

    service.trigger();

    expect(service.promptVisible()).toBe(true);
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('restarts only after an update has been installed', async () => {
    invoke
      .mockResolvedValueOnce({
        currentVersion: '0.1.0',
        update: { version: '0.2.0', body: null, date: null },
        managedByStore: false,
      })
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce(undefined);
    await service.check(false);
    await service.install();

    await service.restart();

    expect(invoke).toHaveBeenLastCalledWith('application_update_restart', {});
  });

  it('delegates updates to Microsoft Store without contacting GitHub Releases', async () => {
    invoke.mockResolvedValueOnce({
      currentVersion: '0.1.0',
      markdown: '## 0.1.0\n\n- Installed from Store',
      managedByStore: true,
    });

    service.initialize();
    await vi.waitFor(() => expect(service.managedByStore()).toBe(true));

    expect(invoke).toHaveBeenCalledOnce();
    expect(invoke).toHaveBeenCalledWith('application_release_notes', {});
    expect(service.triggerLabel()).toBe('Microsoft Store');

    service.trigger();
    expect(service.promptVisible()).toBe(true);
  });

  it('extracts only the selected installed release including prerelease versions', () => {
    const markdown = [
      '# Releases',
      '## 2.0.0-beta.1 — today',
      '- Preview',
      '## 1.9.0 — yesterday',
      '- Stable',
    ].join('\n');

    expect(releaseNotesForVersion(markdown, '2.0.0-beta.1')).toBe(
      '## 2.0.0-beta.1 — today\n- Preview',
    );
    expect(releaseNotesForVersion(markdown, 'missing')).toBe('');
  });
});
