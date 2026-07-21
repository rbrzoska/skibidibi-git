import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';

import { SafeMarkdown } from './safe-markdown';

describe('SafeMarkdown', () => {
  it('renders GitHub Markdown and safe details while stripping active content', async () => {
    await TestBed.configureTestingModule({ imports: [SafeMarkdown] }).compileComponents();
    const fixture = TestBed.createComponent(SafeMarkdown);
    fixture.componentRef.setInput('source', `
<details open><summary>Diagnostics</summary><strong>inside</strong></details>

## Review notes

[safe](https://github.com/o/r) [unsafe](javascript:alert(1))

<img src="https://example.test/image.png" onerror="alert(1)">
<script>alert('xss')</script>
`);
    fixture.detectChanges();
    await fixture.whenStable();

    const root = fixture.nativeElement as HTMLElement;
    expect(root.querySelector('details summary')?.textContent).toBe('Diagnostics');
    expect(root.querySelector('details strong')?.textContent).toBe('inside');
    expect(root.querySelector('h2')?.textContent).toBe('Review notes');
    const safe = [...root.querySelectorAll<HTMLAnchorElement>('a')].find((link) => link.textContent === 'safe');
    expect(safe?.getAttribute('href')).toBe('https://github.com/o/r');
    expect(safe?.target).toBe('_blank');
    expect(safe?.rel).toBe('noreferrer');
    const unsafe = [...root.querySelectorAll<HTMLAnchorElement>('a')].find((link) => link.textContent === 'unsafe');
    expect(unsafe?.hasAttribute('href')).toBe(false);
    expect(root.querySelector('script')).toBeNull();
    expect(root.querySelector('img')?.hasAttribute('onerror')).toBe(false);
  });
});
