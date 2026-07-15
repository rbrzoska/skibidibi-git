import {
  provideBrowserGlobalErrorListeners,
  provideZonelessChangeDetection,
  type ApplicationConfig,
} from '@angular/core';

import { DESKTOP_IPC, DesktopIpc } from './core/ipc/desktop-ipc';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZonelessChangeDetection(),
    { provide: DESKTOP_IPC, useExisting: DesktopIpc },
  ],
};
