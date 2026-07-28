import { computed, Injectable, signal } from '@angular/core';

import type { AiCommanderContext } from '../ipc/desktop-ipc';

@Injectable({ providedIn: 'root' })
export class CommanderContextStore {
  private readonly route = signal('/');
  private readonly selectedEntity = signal<string | null>(null);

  readonly context = computed<AiCommanderContext>(() => {
    const route = this.route();
    const workspace = /^\/workspace\/([^/]+)\/([^/?#]+)/.exec(route);
    return {
      route,
      screen: workspace?.[2] ?? route.split(/[/?#]/).filter(Boolean)[0] ?? 'repositories',
      repositoryId: workspace?.[1] ?? null,
      selectedEntity: this.selectedEntity(),
    };
  });

  updateRoute(route: string): void {
    this.route.set(route);
    this.selectedEntity.set(null);
  }

  select(entity: string | null): void {
    this.selectedEntity.set(entity?.slice(0, 512) ?? null);
  }
}
