import type { Routes } from '@angular/router';

import { RepositoryLauncher } from './features/repository-launcher/repository-launcher';
import { WorkspaceHistory } from './features/workspace-history/workspace-history';

export const routes: Routes = [
  { path: 'repositories', component: RepositoryLauncher, title: 'Repositories · Skibidibi Git' },
  { path: 'workspace/:repositoryId/history', component: WorkspaceHistory, title: 'History · Skibidibi Git' },
  {
    path: 'settings',
    loadComponent: () => import('./features/settings/settings-page/settings-page')
      .then(({ SettingsPage }) => SettingsPage),
    title: 'Settings · Skibidibi Git',
  },
  { path: '', pathMatch: 'full', redirectTo: 'repositories' },
  { path: '**', redirectTo: 'repositories' },
];
