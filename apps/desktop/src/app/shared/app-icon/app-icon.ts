import { ChangeDetectionStrategy, Component, input } from '@angular/core';

export type AppIconName =
  | 'amend'
  | 'branch'
  | 'close'
  | 'download'
  | 'folder-remove'
  | 'refresh'
  | 'shield'
  | 'stash'
  | 'trash'
  | 'upload'
  | 'warning';

@Component({
  selector: 'app-icon',
  imports: [],
  template: `
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      @switch (name()) {
        @case ('branch') { <path d="M6 3v12a4 4 0 0 0 4 4h5"/><circle cx="6" cy="3" r="2"/><circle cx="17" cy="6" r="2"/><circle cx="17" cy="19" r="2"/><path d="M8 8h5a4 4 0 0 0 4-2"/> }
        @case ('close') { <path d="m6 6 12 12M18 6 6 18"/> }
        @case ('amend') { <path d="M4 20h4l10.5-10.5a2.1 2.1 0 0 0-4-4L4 16v4Z"/><path d="m13.5 6.5 4 4"/><path d="M12 20h8"/> }
        @case ('download') { <path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M4 19h16"/> }
        @case ('upload') { <path d="M12 16V4"/><path d="m7 9 5-5 5 5"/><path d="M4 20h16"/> }
        @case ('refresh') { <path d="M20 6v5h-5"/><path d="M4 18v-5h5"/><path d="M18.5 9a7 7 0 0 0-12-2L4 11"/><path d="M5.5 15a7 7 0 0 0 12 2l2.5-4"/> }
        @case ('stash') { <path d="M4 7h16v13H4z"/><path d="M3 4h18v3H3z"/><path d="M9 11h6"/> }
        @case ('trash') { <path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="m7 7 1 13h8l1-13"/><path d="M10 11v5M14 11v5"/> }
        @case ('folder-remove') { <path d="M3 6h7l2 2h9v11H3z"/><path d="M9 14h6"/> }
        @case ('shield') { <path d="M12 3 5 6v5c0 4.5 2.8 7.7 7 10 4.2-2.3 7-5.5 7-10V6l-7-3Z"/><path d="m9 12 2 2 4-5"/> }
        @case ('warning') { <path d="M12 4 3 20h18L12 4Z"/><path d="M12 9v5"/><path d="M12 17h.01"/> }
      }
    </svg>
  `,
  styles: `:host{display:inline-grid;place-items:center;flex:0 0 auto;width:1em;height:1em}svg{display:block;width:100%;height:100%;stroke-width:1.8}`,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AppIcon {
  readonly name = input.required<AppIconName>();
}
