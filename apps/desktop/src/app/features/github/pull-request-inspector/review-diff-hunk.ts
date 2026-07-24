export type ReviewDiffLineKind = 'header' | 'context' | 'addition' | 'deletion' | 'meta';

export interface ReviewDiffLine {
  readonly kind: ReviewDiffLineKind;
  readonly oldLine: number | null;
  readonly newLine: number | null;
  readonly marker: string;
  readonly content: string;
}

const HUNK_HEADER = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;
const MAX_RENDERED_LINES = 240;

export function parseReviewDiffHunk(hunk: string): readonly ReviewDiffLine[] {
  let oldLine = 0;
  let newLine = 0;

  return hunk.split(/\r?\n/).slice(0, MAX_RENDERED_LINES).map((raw): ReviewDiffLine => {
    const header = raw.match(HUNK_HEADER);
    if (header !== null) {
      oldLine = Number(header[1]);
      newLine = Number(header[2]);
      return { kind: 'header', oldLine: null, newLine: null, marker: '', content: raw };
    }
    if (raw.startsWith('+') && !raw.startsWith('+++')) {
      const row = { kind: 'addition' as const, oldLine: null, newLine, marker: '+', content: raw.slice(1) };
      newLine += 1;
      return row;
    }
    if (raw.startsWith('-') && !raw.startsWith('---')) {
      const row = { kind: 'deletion' as const, oldLine, newLine: null, marker: '−', content: raw.slice(1) };
      oldLine += 1;
      return row;
    }
    if (raw.startsWith(' ')) {
      const row = { kind: 'context' as const, oldLine, newLine, marker: ' ', content: raw.slice(1) };
      oldLine += 1;
      newLine += 1;
      return row;
    }
    return { kind: 'meta', oldLine: null, newLine: null, marker: '', content: raw };
  });
}
