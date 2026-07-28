import { TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import { DESKTOP_IPC, type DesktopIpcClient } from '../ipc/desktop-ipc';
import { ApplicationUpdate } from './application-update';

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
      update: { version: '0.2.0', body: 'Faster fetches', date: '2026-07-28T10:00:00Z' },
    });

    service.initialize();
    service.initialize();
    await vi.waitFor(() => expect(service.state()).toBe('available'));

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(service.currentVersion()).toBe('0.1.0');
    expect(service.availableUpdate()?.version).toBe('0.2.0');
  });

  it('persists disabling automatic startup checks', () => {
    service.setAutoCheck(false);
    service.initialize();

    expect(service.autoCheck()).toBe(false);
    expect(globalThis.localStorage.getItem('skibidibi-git.application-update.auto-check.v1')).toBe('false');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('rechecks the exact selected version before the native installer restarts', async () => {
    invoke
      .mockResolvedValueOnce({
        currentVersion: '0.1.0',
        update: { version: '0.2.0', body: null, date: null },
      })
      .mockResolvedValueOnce(undefined);
    await service.check(false);

    await service.install();

    expect(invoke).toHaveBeenLastCalledWith('application_update_install', {
      expectedVersion: '0.2.0',
    });
  });
});
