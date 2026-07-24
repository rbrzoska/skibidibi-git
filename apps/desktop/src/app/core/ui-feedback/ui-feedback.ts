import { Injectable, computed, signal } from '@angular/core';

export type UiFeedbackKind = 'success' | 'warning' | 'error' | 'info';

export interface UiToast {
  readonly id: number;
  readonly key: string | null;
  readonly kind: UiFeedbackKind;
  readonly title: string;
  readonly message: string;
}

interface ToastOptions {
  readonly key?: string;
  readonly timeoutMs?: number;
  readonly onDismiss?: () => void;
}

@Injectable({ providedIn: 'root' })
export class UiFeedback {
  private readonly loadingScopes = signal<ReadonlyMap<string, string>>(new Map());
  private readonly toastState = signal<readonly UiToast[]>([]);
  private readonly toastTimers = new Map<number, ReturnType<typeof globalThis.setTimeout>>();
  private readonly dismissCallbacks = new Map<number, () => void>();
  private nextToastId = 1;

  readonly loading = computed(() => this.loadingScopes().size > 0);
  readonly loadingLabel = computed(() => [...this.loadingScopes().values()].at(-1) ?? 'Working…');
  readonly toasts = this.toastState.asReadonly();

  setLoading(scope: string, active: boolean, label = 'Working…'): void {
    const next = new Map(this.loadingScopes());
    if (active) {
      next.set(scope, label);
    } else {
      next.delete(scope);
    }
    this.loadingScopes.set(next);
  }

  show(kind: UiFeedbackKind, title: string, message: string, options: ToastOptions = {}): number {
    const existing = options.key === undefined
      ? undefined
      : this.toastState().find((toast) => toast.key === options.key);
    const id = existing?.id ?? this.nextToastId++;
    const toast: UiToast = {
      id,
      key: options.key ?? null,
      kind,
      title,
      message,
    };
    const previous = this.toastState();
    const next = [
      ...previous.filter((item) => item.id !== id),
      toast,
    ].slice(-4);
    this.toastState.set(next);
    const retainedIds = new Set(next.map((item) => item.id));
    for (const removed of previous) {
      if (!retainedIds.has(removed.id)) {
        this.clearTimer(removed.id);
        this.dismissCallbacks.delete(removed.id);
      }
    }
    this.clearTimer(id);
    if (options.onDismiss !== undefined) {
      this.dismissCallbacks.set(id, options.onDismiss);
    }
    const timeoutMs = options.timeoutMs ?? (kind === 'error' ? 7000 : 4500);
    if (timeoutMs > 0) {
      this.toastTimers.set(id, globalThis.setTimeout(() => this.dismiss(id), timeoutMs));
    }
    return id;
  }

  sync(
    key: string,
    kind: UiFeedbackKind,
    title: string,
    message: string,
    onDismiss: () => void,
  ): void {
    const existing = this.toastState().find((toast) => toast.key === key);
    if (message.trim() === '') {
      if (existing !== undefined) {
        this.remove(existing.id, false);
      }
      return;
    }
    if (existing?.message === message && existing.title === title && existing.kind === kind) {
      return;
    }
    this.show(kind, title, message, { key, onDismiss });
  }

  dismiss(id: number): void {
    this.remove(id, true);
  }

  private remove(id: number, notifySource: boolean): void {
    this.toastState.update((items) => items.filter((toast) => toast.id !== id));
    this.clearTimer(id);
    const callback = this.dismissCallbacks.get(id);
    this.dismissCallbacks.delete(id);
    if (notifySource) {
      callback?.();
    }
  }

  private clearTimer(id: number): void {
    const timer = this.toastTimers.get(id);
    if (timer !== undefined) {
      globalThis.clearTimeout(timer);
      this.toastTimers.delete(id);
    }
  }
}
