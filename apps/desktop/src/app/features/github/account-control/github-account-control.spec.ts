import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../../core/github';
import { GitHubAccountControl } from './github-account-control';

describe('GitHubAccountControl', () => {
  it('starts the primary GitHub device flow and presents only the user code', async () => {
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
      githubStartDeviceFlow: vi.fn().mockResolvedValue({
        flowId: 'opaque-flow',
        userCode: 'ABCD-EFGH',
        verificationUri: 'https://github.com/login/device',
        expiresAt: Date.now() / 1_000 + 900,
        intervalSeconds: 5,
      }),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn().mockResolvedValue({ cancelled: true }),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    await TestBed.configureTestingModule({
      imports: [GitHubAccountControl],
      providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }],
    }).compileComponents();
    const fixture = TestBed.createComponent(GitHubAccountControl);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const connectButton = [...fixture.nativeElement.querySelectorAll('button')]
      .find((button: HTMLButtonElement) => button.textContent?.includes('Connect with GitHub')) as HTMLButtonElement;
    connectButton.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(bridge.githubStartDeviceFlow).toHaveBeenCalledOnce();
    expect(fixture.nativeElement.textContent).toContain('ABCD-EFGH');
    expect(fixture.nativeElement.textContent).not.toContain('opaque-flow');
    await TestBed.inject(GitHubAccountStore).cancelDeviceFlow();
  });

  it('clears the PAT field before the pending bridge call completes', async () => {
    let resolveConnect!: (account: { id: string; login: string; host: string; avatarUrl: null; state: 'connected' }) => void;
    const connect = new Promise<{ id: string; login: string; host: string; avatarUrl: null; state: 'connected' }>((resolve) => { resolveConnect = resolve; });
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
      githubStartDeviceFlow: vi.fn(),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn().mockReturnValue(connect),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    await TestBed.configureTestingModule({
      imports: [GitHubAccountControl],
      providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }],
    }).compileComponents();
    const fixture: ComponentFixture<GitHubAccountControl> = TestBed.createComponent(GitHubAccountControl);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const input = fixture.nativeElement.querySelector('#github-pat') as HTMLInputElement;
    input.value = 'github_pat_secret';
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (fixture.nativeElement.querySelector('form') as HTMLFormElement).dispatchEvent(new Event('submit'));
    fixture.detectChanges();

    expect(input.value).toBe('');
    expect(bridge.githubConnectPat).toHaveBeenCalledWith({ token: 'github_pat_secret' });
    resolveConnect({ id: '1', login: 'ada', host: 'github.com', avatarUrl: null, state: 'connected' });
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('ada');
  });
});
