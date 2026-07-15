import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';

import { App } from './app';
import { DESKTOP_IPC, DesktopIpc } from './core/ipc/desktop-ipc';

describe('App', () => {
  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [provideRouter([]), { provide: DESKTOP_IPC, useExisting: DesktopIpc }],
    }).compileComponents();
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
});
