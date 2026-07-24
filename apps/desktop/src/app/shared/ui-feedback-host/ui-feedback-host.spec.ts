import { ComponentFixture, TestBed } from '@angular/core/testing';

import { UiFeedback } from '../../core/ui-feedback/ui-feedback';
import { UiFeedbackHost } from './ui-feedback-host';

describe('UiFeedbackHost', () => {
  let component: UiFeedbackHost;
  let fixture: ComponentFixture<UiFeedbackHost>;
  let feedback: UiFeedback;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [UiFeedbackHost],
    }).compileComponents();

    fixture = TestBed.createComponent(UiFeedbackHost);
    component = fixture.componentInstance;
    feedback = TestBed.inject(UiFeedback);
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('renders the central robot loader with its current operation label', () => {
    feedback.setLoading('test', true, 'Reading repository…');
    fixture.detectChanges();

    const overlay = fixture.nativeElement.querySelector('.skibi-loading-overlay') as HTMLElement;
    expect(overlay).not.toBeNull();
    expect(overlay.textContent).toContain('Reading repository…');
    expect(overlay.querySelector('.loader-arc')).not.toBeNull();
  });

  it('renders notifications in the bottom robot speech-bubble container', () => {
    feedback.show('error', 'Git needs attention', 'The branch could not be switched.', { timeoutMs: 0 });
    fixture.detectChanges();

    const toast = fixture.nativeElement.querySelector('.skibi-toast') as HTMLElement;
    expect(toast.getAttribute('data-kind')).toBe('error');
    expect(toast.textContent).toContain('The branch could not be switched.');
    expect(toast.querySelector('app-skibi-bot')).not.toBeNull();
  });
});
