import { InjectionToken } from '@angular/core';

export interface GitHubAccount {
  readonly id: string;
  readonly login: string;
  readonly host: string;
  readonly avatarUrl: string | null;
  readonly state: 'unknown' | 'connected' | 'authenticationRequired' | 'unavailable';
  readonly authKind: 'personalAccessToken' | 'oAuthDevice' | 'gitHubCli';
}

export interface GitHubRepository {
  readonly id: string;
  readonly owner: string;
  readonly name: string;
  readonly fullName: string;
  readonly private: boolean;
  readonly updatedAt: string;
  readonly httpsCloneUrl: string;
  readonly sshCloneUrl: string;
}

export interface GitHubRepositoryPage {
  readonly repositories: readonly GitHubRepository[];
  readonly nextCursor: string | null;
}

export interface GitHubPullRequestSummary {
  readonly number: number;
  readonly title: string;
  readonly url: string;
  readonly state: 'open' | 'closed' | 'merged';
  readonly draft: boolean;
  readonly authorLogin: string;
  readonly headRefName: string;
  readonly baseRefName: string;
  readonly updatedAt: string;
  readonly authoredByViewer: boolean;
  readonly commentCount: number;
  readonly reviewRequestedFromViewer: boolean | null;
  readonly unresolvedThreadCount: number | null;
}

export interface GitHubPullRequestComment {
  readonly id: string;
  readonly authorLogin: string;
  readonly body: string;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly url: string | null;
  readonly path: string | null;
  readonly line: number | null;
  readonly side: 'left' | 'right' | null;
}

export interface GitHubReviewThread {
  readonly id: string;
  readonly path: string;
  readonly line: number | null;
  readonly resolved: boolean;
  readonly outdated: boolean;
  readonly comments: readonly GitHubPullRequestComment[];
}

export interface GitHubPullRequestDetail extends GitHubPullRequestSummary {
  readonly body: string;
  readonly additions: number;
  readonly deletions: number;
  readonly changedFiles: number;
  readonly mergeability: 'unknown' | 'mergeable' | 'conflicting';
  readonly comments: readonly GitHubPullRequestComment[];
  readonly reviewThreads: readonly GitHubReviewThread[];
  readonly conversationTruncated: boolean;
  readonly reviewThreadsTruncated: boolean;
}

export interface GitHubListPullRequestsRequest {
  readonly accountId: string;
  readonly repositoryId: string;
  readonly scope: GitHubPullRequestScope;
  readonly cursor: string | null;
  readonly pageSize: number;
}

export type GitHubPullRequestScope = 'assignedToViewer' | 'authoredByViewer';

export interface GitHubListPullRequestsResponse {
  readonly pullRequests: readonly GitHubPullRequestSummary[];
  readonly nextCursor: string | null;
}

export interface GitHubPullRequestDetailRequest {
  readonly accountId: string;
  readonly repositoryId: string;
  readonly number: number;
}

export interface GitHubDeviceFlowStart {
  readonly flowId: string;
  readonly userCode: string;
  readonly verificationUri: string;
  readonly expiresAt: number;
  readonly intervalSeconds: number;
}

export type GitHubDeviceFlowPollState = 'pending' | 'authorized' | 'expired' | 'denied';

export interface GitHubDeviceFlowPoll {
  readonly state: GitHubDeviceFlowPollState;
  readonly nextPollAt: number | null;
  readonly account: GitHubAccount | null;
}

/**
 * Narrow bridge used by Angular GitHub features. The desktop adapter owns token
 * storage and transport; consumers must never persist or expose a PAT.
 */
export interface GitHubBridge {
  githubListAccounts(): Promise<readonly GitHubAccount[]>;
  githubStartDeviceFlow(): Promise<GitHubDeviceFlowStart>;
  githubPollDeviceFlow(request: { readonly flowId: string }): Promise<GitHubDeviceFlowPoll>;
  githubCancelDeviceFlow(request: { readonly flowId: string }): Promise<{ readonly cancelled: boolean }>;
  githubOpenDeviceVerification(request: { readonly flowId: string }): Promise<void>;
  githubConnectPat(request: { readonly token: string }): Promise<GitHubAccount>;
  githubConnectCli(): Promise<GitHubAccount>;
  githubDisconnectAccount(request: { readonly accountId: string }): Promise<{ readonly disconnected: boolean }>;
  githubListRepositories(request: { readonly accountId: string; readonly cursor: string | null; readonly pageSize: number }): Promise<GitHubRepositoryPage>;
  githubListPullRequests(request: GitHubListPullRequestsRequest): Promise<GitHubListPullRequestsResponse>;
  githubPullRequestDetail(request: GitHubPullRequestDetailRequest): Promise<GitHubPullRequestDetail>;
}

export const GITHUB_BRIDGE = new InjectionToken<GitHubBridge>('GITHUB_BRIDGE');
