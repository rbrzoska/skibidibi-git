import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../../core/github';
import { GitHubAccountControl } from './github-account-control';

describe('GitHubAccountControl', () => {
  it('clears the PAT field before the pending bridge call completes', async () => {
    let resolveConnect!: (account: { id: string; login: string; host: string; avatarUrl: null; state: 'connected' }) => void;
    const connect = new Promise<{ id: string; login: string; host: string; avatarUrl: null; state: 'connected' }>((resolve) => { resolveConnect = resolve; });
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
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
