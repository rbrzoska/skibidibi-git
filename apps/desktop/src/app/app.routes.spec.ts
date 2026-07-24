import { describe, expect, it } from 'vitest';

import { routes } from './app.routes';

describe('application routes', () => {
  it('loads the workspace history feature lazily', async () => {
    const workspaceRoute = routes.find((route) => route.path === 'workspace/:repositoryId/history');

    expect(workspaceRoute?.component).toBeUndefined();
    expect(workspaceRoute?.loadComponent).toBeTypeOf('function');

    const component = await workspaceRoute?.loadComponent?.();
    expect(component).toBeTypeOf('function');
  });

  it('loads the reference comparison feature lazily', async () => {
    const comparisonRoute = routes.find((route) => route.path === 'workspace/:repositoryId/compare');

    expect(comparisonRoute?.component).toBeUndefined();
    expect(comparisonRoute?.loadComponent).toBeTypeOf('function');

    const component = await comparisonRoute?.loadComponent?.();
    expect(component).toBeTypeOf('function');
  });

  it('loads the file history feature lazily', async () => {
    const fileHistoryRoute = routes.find((route) => route.path === 'workspace/:repositoryId/file-history');

    expect(fileHistoryRoute?.component).toBeUndefined();
    expect(fileHistoryRoute?.loadComponent).toBeTypeOf('function');

    const component = await fileHistoryRoute?.loadComponent?.();
    expect(component).toBeTypeOf('function');
  });
});
