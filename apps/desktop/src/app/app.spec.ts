import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { vi } from 'vitest';

import { App } from './app';
import { DESKTOP_IPC, DesktopIpc } from './core/ipc/desktop-ipc';
import { DesktopGitHubBridge, GITHUB_BRIDGE, GitHubAccountStore } from './core/github';

describe('App', () => {
  beforeEach(async () => {
    globalThis.localStorage.clear();
    globalThis.document.documentElement.style.removeProperty('zoom');
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [
        provideRouter([]),
        { provide: DESKTOP_IPC, useExisting: DesktopIpc },
        { provide: GITHUB_BRIDGE, useExisting: DesktopGitHubBridge },
        GitHubAccountStore,
      ],
    }).compileComponents();
  });

  afterEach(() => {
    globalThis.localStorage.clear();
    globalThis.document.documentElement.style.removeProperty('zoom');
  });

  it('creates the application shell', () => {
    const fixture = TestBed.createComponent(App);
    const app = fixture.componentInstance;

    expect(app).toBeTruthy();
  });

  it('renders the product title', async () => {
    const fixture = TestBed.createComponent(App);

    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.textContent).toContain('Skibidibi Git');
  });

  it('exposes compact font controls next to the account and increases the UI scale', async () => {
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const compiled = fixture.nativeElement as HTMLElement;
    await vi.waitFor(() => {
      fixture.detectChanges();
      expect(compiled.querySelector('.update-trigger')?.textContent).toContain('Up to date');
    });
    const increase = compiled.querySelector('[aria-label="Increase text size"]') as HTMLButtonElement;
    const controls = compiled.querySelector('.app-controls');
    expect(controls?.querySelector('.update-trigger')?.textContent).toContain('Up to date');
    expect(controls?.querySelector('.font-scale')).not.toBeNull();
    expect(controls?.querySelector('.account-chip')).not.toBeNull();

    increase.click();
    fixture.detectChanges();
    expect(compiled.textContent).toContain('110%');
  });
});
