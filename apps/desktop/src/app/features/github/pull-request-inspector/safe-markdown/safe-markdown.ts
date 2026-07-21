import {
  ChangeDetectionStrategy,
  Component,
  ViewEncapsulation,
  computed,
  inject,
  input,
} from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import DOMPurify from 'dompurify';
import { Marked, Renderer, type Tokens } from 'marked';

const renderer = new Renderer();

renderer.link = function ({ href, title, tokens }: Tokens.Link): string {
  const label = this.parser.parseInline(tokens);
  const titleAttribute = title == null ? '' : ` title="${escapeAttribute(title)}"`;
  return `<a href="${escapeAttribute(href)}" target="_blank" rel="noreferrer"${titleAttribute}>${label}</a>`;
};

const markdown = new Marked({
  async: false,
  breaks: true,
  gfm: true,
  renderer,
});

@Component({
  selector: 'app-safe-markdown',
  imports: [],
  templateUrl: './safe-markdown.html',
  styleUrl: './safe-markdown.css',
  host: { class: 'safe-markdown' },
  encapsulation: ViewEncapsulation.None,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SafeMarkdown {
  private readonly sanitizer = inject(DomSanitizer);

  readonly source = input('');
  protected readonly rendered = computed(() => {
    const parsed = markdown.parse(this.source()) as string;
    const clean = DOMPurify.sanitize(parsed, {
      ALLOWED_TAGS: [
        'a', 'blockquote', 'br', 'code', 'del', 'details', 'em', 'h1', 'h2', 'h3',
        'h4', 'h5', 'h6', 'hr', 'img', 'input', 'li', 'ol', 'p', 'pre', 's',
        'strong', 'summary', 'table', 'tbody', 'td', 'th', 'thead', 'tr', 'ul',
      ],
      ALLOWED_ATTR: [
        'alt', 'checked', 'class', 'disabled', 'height', 'href', 'open', 'rel', 'src',
        'target', 'title', 'type', 'width',
      ],
      ALLOW_DATA_ATTR: false,
    });
    // DOMPurify is the security boundary. Trusting only its output avoids Angular
    // reinterpreting Markdown while keeping event handlers and unsafe URLs out.
    return this.sanitizer.bypassSecurityTrustHtml(clean);
  });
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}

function escapeAttribute(value: string): string {
  return escapeHtml(value).replaceAll('"', '&quot;');
}
