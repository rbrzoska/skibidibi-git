import { KeyValuePipe } from '@angular/common';
import { ChangeDetectionStrategy, Component, ElementRef, signal, viewChild } from '@angular/core';

import type { DiagnosticEntry, DiagnosticLogResponse } from '../../../core/ipc/desktop-ipc';

@Component({
  selector: 'app-diagnostic-log-dialog',
  imports: [KeyValuePipe],
  templateUrl: './diagnostic-log-dialog.html',
  styleUrl: './diagnostic-log-dialog.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class DiagnosticLogDialog {
  private readonly dialog = viewChild.required<ElementRef<HTMLDialogElement>>('dialog');
  protected readonly entries = signal<readonly DiagnosticEntry[]>([]);
  protected readonly totalBytes = signal(0);
  protected readonly truncated = signal(false);

  open(log: DiagnosticLogResponse): void {
    this.entries.set(log.entries);
    this.totalBytes.set(log.totalBytes);
    this.truncated.set(log.truncated);
    const dialog = this.dialog().nativeElement;
    if (!dialog.open) {
      if (typeof dialog.showModal === 'function') {
        dialog.showModal();
      } else {
        dialog.setAttribute('open', '');
      }
    }
  }

  protected close(): void {
    const dialog = this.dialog().nativeElement;
    if (typeof dialog.close === 'function') {
      dialog.close();
    } else {
      dialog.removeAttribute('open');
    }
  }

  protected formattedTimestamp(timestampMs: number): string {
    if (!Number.isFinite(timestampMs) || timestampMs < 0) {
      return 'Unknown time';
    }
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'medium',
    }).format(new Date(timestampMs));
  }

  protected timestampIso(timestampMs: number): string | null {
    return Number.isFinite(timestampMs) && timestampMs >= 0
      ? new Date(timestampMs).toISOString()
      : null;
  }

  protected formatBytes(bytes: number): string {
    if (bytes < 1024) {
      return `${bytes} B`;
    }
    return `${(bytes / 1024).toFixed(bytes >= 10 * 1024 ? 0 : 1)} KiB`;
  }
}
