import { DOCUMENT } from '@angular/common';
import { computed, inject, Injectable, signal } from '@angular/core';

import { DESKTOP_IPC } from '../ipc/desktop-ipc';

const STORAGE_KEY = 'skibidibi-git.ui-scale.v1';
const SCALE_STEPS = [1, 1.1, 1.2, 1.3, 1.4, 1.5] as const;

interface TauriMarker {
  readonly __TAURI__?: { readonly core?: unknown };
}

@Injectable({ providedIn: 'root' })
export class UiScale {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly document = inject(DOCUMENT);
  private readonly scaleIndex = signal(this.restoreScaleIndex());
  private requestGeneration = 0;

  readonly scale = computed(() => SCALE_STEPS[this.scaleIndex()]);
  readonly percent = computed(() => Math.round(this.scale() * 100));
  readonly canDecrease = computed(() => this.scaleIndex() > 0);
  readonly canIncrease = computed(() => this.scaleIndex() < SCALE_STEPS.length - 1);
  readonly error = signal('');

  constructor() {
    this.applyCurrentScale();
  }

  increase(): void {
    this.setScaleIndex(Math.min(this.scaleIndex() + 1, SCALE_STEPS.length - 1));
  }

  decrease(): void {
    this.setScaleIndex(Math.max(this.scaleIndex() - 1, 0));
  }

  reset(): void {
    this.setScaleIndex(0);
  }

  handleKeyboardShortcut(event: KeyboardEvent): boolean {
    if (!(event.ctrlKey || event.metaKey) || event.altKey) {
      return false;
    }
    if (event.key === '+' || event.key === '=' || event.code === 'NumpadAdd') {
      event.preventDefault();
      this.increase();
      return true;
    }
    if (event.key === '-' || event.key === '_' || event.code === 'NumpadSubtract') {
      event.preventDefault();
      this.decrease();
      return true;
    }
    if (event.key === '0' || event.code === 'Numpad0') {
      event.preventDefault();
      this.reset();
      return true;
    }
    return false;
  }

  private setScaleIndex(index: number): void {
    if (index === this.scaleIndex()) {
      return;
    }
    this.scaleIndex.set(index);
    this.persistScaleIndex(index);
    this.applyCurrentScale();
  }

  private applyCurrentScale(): void {
    const scale = this.scale();
    const generation = ++this.requestGeneration;
    this.error.set('');

    if (!this.isTauriDesktop()) {
      this.document.documentElement.style.setProperty('zoom', String(scale));
    }

    void this.ipc.invoke('set_application_zoom', { scale }).catch(() => {
      if (generation === this.requestGeneration && this.isTauriDesktop()) {
        this.error.set('Application zoom could not be changed.');
      }
    });
  }

  private isTauriDesktop(): boolean {
    return (globalThis as TauriMarker).__TAURI__?.core !== undefined;
  }

  private restoreScaleIndex(): number {
    try {
      const persisted = globalThis.localStorage?.getItem(STORAGE_KEY);
      const scale = persisted === null ? Number.NaN : Number(persisted);
      const index = SCALE_STEPS.findIndex((candidate) => candidate === scale);
      return index >= 0 ? index : 0;
    } catch {
      return 0;
    }
  }

  private persistScaleIndex(index: number): void {
    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, String(SCALE_STEPS[index]));
    } catch {
      // Scaling remains available for this session when storage is unavailable.
    }
  }
}
