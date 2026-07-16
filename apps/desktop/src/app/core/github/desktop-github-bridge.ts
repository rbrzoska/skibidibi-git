import { Injectable, inject } from '@angular/core';

import { DESKTOP_IPC } from '../ipc/desktop-ipc';
import type {
  GitHubAccount,
  GitHubBridge,
  GitHubDeviceFlowPoll,
  GitHubDeviceFlowStart,
  GitHubListPullRequestsRequest,
  GitHubListPullRequestsResponse,
  GitHubPullRequestDetail,
  GitHubPullRequestDetailRequest,
  GitHubRepositoryPage,
} from './github-bridge';

@Injectable({ providedIn: 'root' })
export class DesktopGitHubBridge implements GitHubBridge {
  private readonly ipc = inject(DESKTOP_IPC);

  async githubListAccounts(): Promise<readonly GitHubAccount[]> {
    const accounts = await this.ipc.invoke('github_list_accounts', {});
    return accounts.map(({ id, login, host, avatarUrl, state }) => ({ id, login, host, avatarUrl, state }));
  }

  githubStartDeviceFlow(): Promise<GitHubDeviceFlowStart> {
    return this.ipc.invoke('github_start_device_flow', {});
  }

  githubPollDeviceFlow(request: { readonly flowId: string }): Promise<GitHubDeviceFlowPoll> {
    return this.ipc.invoke('github_poll_device_flow', request);
  }

  githubCancelDeviceFlow(
    request: { readonly flowId: string },
  ): Promise<{ readonly cancelled: boolean }> {
    return this.ipc.invoke('github_cancel_device_flow', request);
  }

  githubOpenDeviceVerification(request: { readonly flowId: string }): Promise<void> {
    return this.ipc.invoke('github_open_device_verification', request);
  }

  async githubConnectPat(request: { readonly token: string }): Promise<GitHubAccount> {
    const { id, login, host, avatarUrl, state } = await this.ipc.invoke('github_connect_pat', request);
    return { id, login, host, avatarUrl, state };
  }

  githubDisconnectAccount(
    request: { readonly accountId: string },
  ): Promise<{ readonly disconnected: boolean }> {
    return this.ipc.invoke('github_disconnect_account', request);
  }

  githubListRepositories(
    request: { readonly accountId: string; readonly cursor: string | null; readonly pageSize: number },
  ): Promise<GitHubRepositoryPage> {
    return this.ipc.invoke('github_list_repositories', request);
  }

  githubListPullRequests(
    request: GitHubListPullRequestsRequest,
  ): Promise<GitHubListPullRequestsResponse> {
    return this.ipc.invoke('github_list_pull_requests', request);
  }

  githubPullRequestDetail(
    request: GitHubPullRequestDetailRequest,
  ): Promise<GitHubPullRequestDetail> {
    return this.ipc.invoke('github_pull_request_detail', request);
  }
}
