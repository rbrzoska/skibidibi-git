import {
  provideBrowserGlobalErrorListeners,
  provideZonelessChangeDetection,
  type ApplicationConfig,
} from '@angular/core';
import { provideRouter } from '@angular/router';

import { DESKTOP_IPC, DesktopIpc } from './core/ipc/desktop-ipc';
import {
  DesktopGitHubBridge,
  GITHUB_BRIDGE,
  GitHubAccountStore,
} from './core/github';
import { routes } from './app.routes';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZonelessChangeDetection(),
    provideRouter(routes),
    { provide: DESKTOP_IPC, useExisting: DesktopIpc },
    { provide: GITHUB_BRIDGE, useExisting: DesktopGitHubBridge },
    GitHubAccountStore,
  ],
};
