import { readFileSync } from 'node:fs';

const indexPath = new URL('../dist/desktop/browser/index.html', import.meta.url);
const html = readFileSync(indexPath, 'utf8');

if (!/<link\s+rel=["']stylesheet["'][^>]*href=["']styles-[^"']+\.css["'][^>]*>/i.test(html)) {
  throw new Error('Production index does not contain the extracted global stylesheet.');
}

if (/media=["']print["']|onload\s*=/i.test(html)) {
  throw new Error(
    'Production stylesheet uses an inline load handler that is blocked by the Tauri CSP.',
  );
}

process.stdout.write('Production stylesheet link is compatible with the Tauri CSP.\n');
