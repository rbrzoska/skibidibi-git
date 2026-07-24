import type { Routes } from '@angular/router';

import { RepositoryLauncher } from './features/repository-launcher/repository-launcher';

export const routes: Routes = [
  { path: 'repositories', component: RepositoryLauncher, title: 'Repositories · Skibidibi Git' },
  {
    path: 'workspace/:repositoryId/history',
    loadComponent: () => import('./features/workspace-history/workspace-history')
      .then(({ WorkspaceHistory }) => WorkspaceHistory),
    title: 'History · Skibidibi Git',
  },
  {
    path: 'workspace/:repositoryId/compare',
    loadComponent: () => import('./features/ref-comparison/ref-comparison')
      .then(({ RefComparison }) => RefComparison),
    title: 'Compare refs · Skibidibi Git',
  },
  {
    path: 'workspace/:repositoryId/file-history',
    loadComponent: () => import('./features/file-history/file-history/file-history')
      .then(({ FileHistory }) => FileHistory),
    title: 'File History · Skibidibi Git',
  },
  {
    path: 'pull-requests',
    loadComponent: () => import('./features/pull-request-dashboard/pull-request-dashboard')
      .then(({ PullRequestDashboard }) => PullRequestDashboard),
    title: 'Pull Requests · Skibidibi Git',
  },
  {
    path: 'code-reviews',
    loadComponent: () => import('./features/code-review-dashboard/code-review-dashboard')
      .then(({ CodeReviewDashboard }) => CodeReviewDashboard),
    title: 'Code Reviews · Skibidibi Git',
  },
  {
    path: 'settings',
    loadComponent: () => import('./features/settings/settings-page/settings-page')
      .then(({ SettingsPage }) => SettingsPage),
    title: 'Settings · Skibidibi Git',
  },
  { path: '', pathMatch: 'full', redirectTo: 'repositories' },
  { path: '**', redirectTo: 'repositories' },
];
