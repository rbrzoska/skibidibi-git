import { Injectable, InjectionToken, isDevMode } from '@angular/core';

export interface RepositoryStatusRequest {
  readonly repositoryPath: string;
}

export interface RepositoryBranchStatus {
  readonly oid: string | null;
  readonly head: string | null;
  readonly upstream: string | null;
  readonly ahead: number;
  readonly behind: number;
  readonly detached: boolean;
  readonly unborn: boolean;
}

export type StatusCode =
  | 'unmodified'
  | 'modified'
  | 'typeChanged'
  | 'added'
  | 'deleted'
  | 'renamed'
  | 'copied'
  | 'unmerged'
  | 'untracked'
  | 'ignored';

export interface StatusEntry {
  readonly kind: 'ordinary' | 'renamedOrCopied' | 'unmerged' | 'untracked' | 'ignored';
  readonly path: string;
  readonly originalPath: string | null;
  readonly indexStatus: StatusCode;
  readonly worktreeStatus: StatusCode;
  readonly submodule: string | null;
}

export interface RepositoryStatusResponse {
  readonly branch: RepositoryBranchStatus;
  readonly entries: readonly StatusEntry[];
}

export interface DesktopIpcContract {
  readonly repository_status: {
    readonly request: RepositoryStatusRequest;
    readonly response: RepositoryStatusResponse;
  };
}

type DesktopCommand = keyof DesktopIpcContract;

export interface DesktopIpcClient {
  invoke<C extends DesktopCommand>(
    command: C,
    request: DesktopIpcContract[C]['request'],
  ): Promise<DesktopIpcContract[C]['response']>;
}

interface TauriCore {
  invoke<TResponse>(command: string, arguments_?: object): Promise<TResponse>;
}

interface TauriGlobal {
  readonly __TAURI__?: {
    readonly core?: TauriCore;
  };
}

export const DESKTOP_IPC = new InjectionToken<DesktopIpcClient>('DESKTOP_IPC');

@Injectable({ providedIn: 'root' })
export class DesktopIpc implements DesktopIpcClient {
  private readonly core = (globalThis as TauriGlobal).__TAURI__?.core;

  invoke<C extends DesktopCommand>(
    command: C,
    request: DesktopIpcContract[C]['request'],
  ): Promise<DesktopIpcContract[C]['response']> {
    if (this.core !== undefined) {
      return this.core.invoke<DesktopIpcContract[C]['response']>(command, request);
    }

    if (isDevMode()) {
      return this.invokeMock(command);
    }

    return Promise.reject(
      new Error('The Tauri IPC bridge is unavailable in the production application.'),
    );
  }

  private invokeMock<C extends DesktopCommand>(
    command: C,
  ): Promise<DesktopIpcContract[C]['response']> {
    if (command === 'repository_status') {
      const response: RepositoryStatusResponse = {
        branch: {
          oid: 'a1b2c3d4e5f6',
          head: 'main',
          upstream: 'origin/main',
          ahead: 0,
          behind: 0,
          detached: false,
          unborn: false,
        },
        entries: [
          {
            kind: 'ordinary',
            path: 'src/app/app.ts',
            originalPath: null,
            indexStatus: 'unmodified',
            worktreeStatus: 'modified',
            submodule: null,
          },
        ],
      };

      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    return Promise.reject(new Error(`Unsupported desktop command: ${command}`));
  }
}
