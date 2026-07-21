import { TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import { DESKTOP_IPC, type DesktopIpcClient } from '../ipc/desktop-ipc';
import { UiScale } from './ui-scale';

describe('UiScale', () => {
  const invoke = vi.fn<DesktopIpcClient['invoke']>();

  beforeEach(() => {
    globalThis.localStorage.clear();
    globalThis.document.documentElement.style.removeProperty('zoom');
    invoke.mockReset();
    invoke.mockResolvedValue({ scale: 1 });
    TestBed.configureTestingModule({
      providers: [{ provide: DESKTOP_IPC, useValue: { invoke } }],
    });
  });

  afterEach(() => {
    globalThis.localStorage.clear();
    globalThis.document.documentElement.style.removeProperty('zoom');
  });

  it('starts at the current 100% size and never decreases below it', () => {
    const service = TestBed.inject(UiScale);

    expect(service.percent()).toBe(100);
    expect(service.canDecrease()).toBe(false);
    service.decrease();

    expect(service.percent()).toBe(100);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('set_application_zoom', { scale: 1 });
  });

  it('increases in bounded steps, persists the choice, and supports keyboard shortcuts', () => {
    const service = TestBed.inject(UiScale);
    const increase = new KeyboardEvent('keydown', { key: '+', metaKey: true, cancelable: true });

    expect(service.handleKeyboardShortcut(increase)).toBe(true);
    expect(increase.defaultPrevented).toBe(true);
    expect(service.percent()).toBe(110);
    expect(globalThis.localStorage.getItem('skibidibi-git.ui-scale.v1')).toBe('1.1');
    expect(globalThis.document.documentElement.style.getPropertyValue('zoom')).toBe('1.1');

    const reset = new KeyboardEvent('keydown', { key: '0', ctrlKey: true, cancelable: true });
    service.handleKeyboardShortcut(reset);
    expect(service.percent()).toBe(100);
  });

  it('caps repeated increases at 150%', () => {
    const service = TestBed.inject(UiScale);

    for (let index = 0; index < 10; index += 1) {
      service.increase();
    }

    expect(service.percent()).toBe(150);
    expect(service.canIncrease()).toBe(false);
  });
});
