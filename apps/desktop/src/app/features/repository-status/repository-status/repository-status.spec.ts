import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { DESKTOP_IPC, type DesktopIpcClient } from '../../../core/ipc/desktop-ipc';
import { RepositoryStatusStore } from '../repository-status';
import { RepositoryStatus } from './repository-status';

describe('RepositoryStatus', () => {
  let component: RepositoryStatus;
  let fixture: ComponentFixture<RepositoryStatus>;

  beforeEach(async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue({
        branch: {
          oid: 'a1b2c3d4e5f6',
          head: 'main',
          upstream: 'origin/main',
          ahead: 0,
          behind: 0,
          detached: false,
          unborn: false,
        },
        entries: [],
      }),
    };
    await TestBed.configureTestingModule({
      imports: [RepositoryStatus],
      providers: [{ provide: DESKTOP_IPC, useValue: ipc }],
    }).compileComponents();

    fixture = TestBed.createComponent(RepositoryStatus);
    component = fixture.componentInstance;
    const store = TestBed.inject(RepositoryStatusStore);
    store.setRepositoryPath('/work/skibidibi-git');
    await store.refresh();
    await fixture.whenStable();
  });

  it('renders the repository status received from IPC', () => {
    fixture.detectChanges();

    expect(component).toBeTruthy();
    expect(fixture.nativeElement.textContent).toContain('main');
    expect(fixture.nativeElement.textContent).toContain('Changes');
  });
});
