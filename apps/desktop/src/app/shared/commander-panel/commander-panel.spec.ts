import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { Commander } from '../../core/commander/commander';
import { DESKTOP_IPC, type DesktopIpcClient } from '../../core/ipc/desktop-ipc';
import { CommanderPanel } from './commander-panel';

describe('CommanderPanel', () => {
  let component: CommanderPanel;
  let fixture: ComponentFixture<CommanderPanel>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [CommanderPanel],
      providers: [
        provideRouter([]),
        {
          provide: DESKTOP_IPC,
          useValue: {
            invoke: vi.fn().mockResolvedValue({
              statuses: [{ provider: 'codex', displayName: 'Codex', available: true, version: '1', detail: null }],
            }),
          } satisfies DesktopIpcClient,
        },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(CommanderPanel);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('renders the conversation when the commander is opened', async () => {
    const open = fixture.nativeElement.ownerDocument.querySelector('.commander-panel');
    expect(open).toBeNull();

    TestBed.inject(Commander).open.set(true);
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.commander-panel')?.textContent).toContain('AI Commander');
  });
});
