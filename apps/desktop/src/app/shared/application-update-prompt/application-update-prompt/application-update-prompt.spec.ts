import { ComponentFixture, TestBed } from '@angular/core/testing';

import { ApplicationUpdate } from '../../../core/application-update/application-update';
import { ApplicationUpdatePrompt } from './application-update-prompt';

describe('ApplicationUpdatePrompt', () => {
  let component: ApplicationUpdatePrompt;
  let fixture: ComponentFixture<ApplicationUpdatePrompt>;

  beforeEach(async () => {
    const updater = {
      state: () => 'available',
      promptVisible: () => true,
      availableUpdate: () => ({
        version: '0.2.0',
        body: 'Release notes',
        date: null,
      }),
      displayVersion: () => '0.2.0',
      displayReleaseNotes: () => '## 0.2.0\n\n- Release notes',
      dismissAvailableUpdate: () => undefined,
      check: () => Promise.resolve(),
      install: () => Promise.resolve(),
      restart: () => Promise.resolve(),
    };
    await TestBed.configureTestingModule({
      imports: [ApplicationUpdatePrompt],
      providers: [{ provide: ApplicationUpdate, useValue: updater }],
    }).compileComponents();

    fixture = TestBed.createComponent(ApplicationUpdatePrompt);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('presents an explicit update installation decision', () => {
    expect(component).toBeTruthy();
    expect(fixture.nativeElement.textContent).toContain('Skibidibi Git 0.2.0 is available');
    expect(fixture.nativeElement.textContent).toContain('Install update');
  });
});
