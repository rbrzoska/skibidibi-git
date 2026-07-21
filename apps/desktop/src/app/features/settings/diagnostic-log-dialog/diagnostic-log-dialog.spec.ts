import { ComponentFixture, TestBed } from '@angular/core/testing';

import { DiagnosticLogDialog } from './diagnostic-log-dialog';

describe('DiagnosticLogDialog', () => {
  let component: DiagnosticLogDialog;
  let fixture: ComponentFixture<DiagnosticLogDialog>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [DiagnosticLogDialog],
    }).compileComponents();

    fixture = TestBed.createComponent(DiagnosticLogDialog);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('opens a formatted, newest-first diagnostic entry without exposing hidden payloads', () => {
    component.open({
      totalBytes: 128,
      truncated: false,
      entries: [{
        timestampMs: Date.UTC(2026, 6, 21, 10, 0),
        severity: 'error',
        subsystem: 'aiSupport',
        eventCode: 'ai_cli_authentication_required',
        message: 'AI commit-message generation failed',
        fields: { provider: 'claude' },
      }],
    });
    fixture.detectChanges();

    const host = fixture.nativeElement as HTMLElement;
    expect(host.querySelector('dialog')?.hasAttribute('open')).toBe(true);
    expect(host.textContent).toContain('ai_cli_authentication_required');
    expect(host.textContent).toContain('claude');
    expect(host.textContent).not.toContain('prompt');
  });
});
