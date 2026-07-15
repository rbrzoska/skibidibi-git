import { describe, expect, it } from 'vitest';

import { createContextualDiffRows } from './contextual-diff';
import { parseUnifiedDiff } from './unified-diff';

function parsedRows(body: readonly string[]) {
  return parseUnifiedDiff(
    [
      'diff --git a/example.ts b/example.ts',
      'index 1111111..2222222 100644',
      '--- a/example.ts',
      '+++ b/example.ts',
      '@@ -1,30 +1,30 @@ section',
      ...body,
    ].join('\n'),
  ).rows;
}

function content(kind: string, rows: ReturnType<typeof createContextualDiffRows>) {
  return rows.filter((row) => row.kind === kind).map((row) => ('content' in row ? row.content : null));
}

describe('createContextualDiffRows', () => {
  it('keeps exactly three context lines before and after a single change', () => {
    const rows = createContextualDiffRows(
      parsedRows([
        ' first', ' second', ' third', ' fourth', ' fifth',
        '-old', '+new',
        ' sixth', ' seventh', ' eighth', ' ninth', ' tenth',
      ]),
    );

    expect(content('context', rows)).toEqual([
      'third', 'fourth', 'fifth', 'sixth', 'seventh', 'eighth',
    ]);
    expect(rows.filter((row) => row.kind === 'omitted-range')).toHaveLength(2);
  });

  it('merges overlapping context windows around nearby changes', () => {
    const rows = createContextualDiffRows(
      parsedRows([
        ' before', '-first-old', '+first-new',
        ' gap-1', ' gap-2', ' gap-3', ' gap-4',
        '-second-old', '+second-new', ' after',
      ]),
    );

    expect(rows.filter((row) => row.kind === 'omitted-range')).toHaveLength(0);
    expect(content('context', rows)).toEqual(['before', 'gap-1', 'gap-2', 'gap-3', 'gap-4', 'after']);
  });

  it('inserts a typed deterministic separator between distant change groups', () => {
    const source = parsedRows([
      '-first-old', '+first-new',
      ' one', ' two', ' three', ' four', ' five', ' six', ' seven',
      '-second-old', '+second-new',
    ]);
    const first = createContextualDiffRows(source);
    const second = createContextualDiffRows(source);
    const separators = first.filter((row) => row.kind === 'omitted-range');

    expect(separators).toHaveLength(1);
    expect(separators[0]).toMatchObject({
      omittedLineCount: 1,
      oldStartLine: 5,
      oldEndLine: 5,
      newStartLine: 5,
      newEndLine: 5,
    });
    expect(first.map((row) => row.key)).toEqual(second.map((row) => row.key));
  });

  it('does not invent separators at hunk boundaries', () => {
    const rows = createContextualDiffRows(
      parsedRows(['-old', '+new', ' one', ' two', ' three', ' four', ' five']),
    );
    const separators = rows.filter((row) => row.kind === 'omitted-range');

    expect(separators).toHaveLength(1);
    expect(rows.at(-1)).toBe(separators[0]);
    expect(content('context', rows)).toEqual(['one', 'two', 'three']);
  });

  it('preserves file headers, metadata and hunk headers in source order', () => {
    const source = parsedRows([' context', '-old', '+new', ' tail']);
    const rows = createContextualDiffRows(source);

    expect(rows.slice(0, 5).map((row) => row.kind)).toEqual([
      'file-header', 'metadata', 'metadata', 'metadata', 'hunk-header',
    ]);
    expect(rows.slice(0, 5).map((row) => row.key)).toEqual(source.slice(0, 5).map((row) => row.key));
  });

  it('handles a large continuous change block in linear space', () => {
    const additions = Array.from({ length: 20_000 }, (_, index) => `+line-${index}`);
    const rows = createContextualDiffRows(parsedRows(additions));

    expect(rows.filter((row) => row.kind === 'addition')).toHaveLength(20_000);
    expect(rows.some((row) => row.kind === 'omitted-range')).toBe(false);
  });
});
