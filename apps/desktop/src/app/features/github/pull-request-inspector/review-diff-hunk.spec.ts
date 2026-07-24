import { describe, expect, it } from 'vitest';

import { parseReviewDiffHunk } from './review-diff-hunk';

describe('parseReviewDiffHunk', () => {
  it('tracks old and new line numbers while preserving context and change kinds', () => {
    const rows = parseReviewDiffHunk(
      '@@ -10,3 +10,4 @@ function example() {\n context\n-deleted\n+added\n+second added\n tail',
    );

    expect(rows).toEqual([
      expect.objectContaining({ kind: 'header', oldLine: null, newLine: null }),
      expect.objectContaining({ kind: 'context', oldLine: 10, newLine: 10, content: 'context' }),
      expect.objectContaining({ kind: 'deletion', oldLine: 11, newLine: null, content: 'deleted' }),
      expect.objectContaining({ kind: 'addition', oldLine: null, newLine: 11, content: 'added' }),
      expect.objectContaining({ kind: 'addition', oldLine: null, newLine: 12, content: 'second added' }),
      expect.objectContaining({ kind: 'context', oldLine: 12, newLine: 13, content: 'tail' }),
    ]);
  });
});
