import { ChangeDetectionStrategy, Component, computed, HostListener, inject } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { filter, map, scan, startWith } from 'rxjs';

import { GitHubAccountStore } from './core/github';
import { UiScale } from './core/accessibility/ui-scale';

@Component({
  selector: 'app-root',
  imports: [RouterLink, RouterLinkActive, RouterOutlet],
  templateUrl: './app.html',
  styleUrl: './app.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {
  private readonly router = inject(Router);
  protected readonly accounts = inject(GitHubAccountStore);
  protected readonly uiScale = inject(UiScale);
  protected readonly workspaceUrl = toSignal(
    this.router.events.pipe(
      filter((event): event is NavigationEnd => event instanceof NavigationEnd),
      map((event) => event.urlAfterRedirects),
      startWith(this.router.url),
      scan(
        (lastWorkspace, url) => url.startsWith('/workspace/') ? url : lastWorkspace,
        this.router.url.startsWith('/workspace/') ? this.router.url : null as string | null,
      ),
    ),
    { initialValue: this.router.url.startsWith('/workspace/') ? this.router.url : null },
  );

  protected readonly accountLabel = computed(() => {
    const state = this.accounts.state();
    if (state.kind === 'ready' && state.accounts.length > 0) {
      return state.accounts[0]?.login ?? 'GitHub';
    }
    if (state.kind === 'loading') {
      return 'GitHub…';
    }
    return 'Local';
  });
  protected readonly accountConnected = computed(() => {
    const state = this.accounts.state();
    return state.kind === 'ready' && state.accounts.some(({ state: status }) => status === 'connected');
  });

  constructor() {
    if (this.accounts.state().kind === 'idle') {
      void this.accounts.load();
    }
  }

  @HostListener('document:keydown', ['$event'])
  protected handleZoomShortcut(event: KeyboardEvent): void {
    this.uiScale.handleKeyboardShortcut(event);
  }
}
