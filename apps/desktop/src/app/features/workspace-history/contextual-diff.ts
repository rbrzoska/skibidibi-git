import type { UnifiedDiffContentRow, UnifiedDiffRow } from './unified-diff';

export interface ContextualDiffOmittedRangeRow {
  readonly kind: 'omitted-range';
  readonly key: string;
  readonly fileKey: string;
  readonly hunkKey: string;
  readonly omittedLineCount: number;
  readonly oldStartLine: number | null;
  readonly oldEndLine: number | null;
  readonly newStartLine: number | null;
  readonly newEndLine: number | null;
}

export type ContextualDiffRow = UnifiedDiffRow | ContextualDiffOmittedRangeRow;

const DEFAULT_CONTEXT_LINES = 3;

function normalizedContextLines(contextLines: number): number {
  return Number.isSafeInteger(contextLines) && contextLines >= 0
    ? contextLines
    : DEFAULT_CONTEXT_LINES;
}

function isContentRow(row: UnifiedDiffRow): row is UnifiedDiffContentRow {
  return row.kind === 'context' || row.kind === 'addition' || row.kind === 'deletion';
}

function omittedRange(
  rows: readonly UnifiedDiffContentRow[],
  hunkKey: string,
  rangeIndex: number,
): ContextualDiffOmittedRangeRow {
  const first = rows[0];
  const last = rows.at(-1) ?? first;
  return {
    kind: 'omitted-range',
    key: `${hunkKey}:omitted-${rangeIndex}:${first.key}-${last.key}`,
    fileKey: first.fileKey,
    hunkKey,
    omittedLineCount: rows.length,
    oldStartLine: first.oldLineNumber,
    oldEndLine: last.oldLineNumber,
    newStartLine: first.newLineNumber,
    newEndLine: last.newLineNumber,
  };
}

function contextualHunkBody(
  rows: readonly UnifiedDiffRow[],
  hunkKey: string,
  contextLines: number,
): readonly ContextualDiffRow[] {
  const contentRows = rows.filter(isContentRow);
  const visible = contentRows.map((row) => row.kind !== 'context');
  let contextDistance = contextLines + 1;

  for (let index = 0; index < contentRows.length; index += 1) {
    const row = contentRows[index];
    if (row.kind !== 'context') {
      contextDistance = 0;
      continue;
    }
    contextDistance += 1;
    visible[index] ||= contextDistance <= contextLines;
  }

  contextDistance = contextLines + 1;
  for (let index = contentRows.length - 1; index >= 0; index -= 1) {
    const row = contentRows[index];
    if (row.kind !== 'context') {
      contextDistance = 0;
      continue;
    }
    contextDistance += 1;
    visible[index] ||= contextDistance <= contextLines;
  }

  const result: ContextualDiffRow[] = [];
  let omitted: UnifiedDiffContentRow[] = [];
  let omittedRangeIndex = 0;
  let contentIndex = 0;

  const flushOmitted = (): void => {
    if (omitted.length === 0) return;
    result.push(omittedRange(omitted, hunkKey, omittedRangeIndex));
    omittedRangeIndex += 1;
    omitted = [];
  };

  for (const row of rows) {
    if (isContentRow(row)) {
      const rowIsVisible = visible[contentIndex];
      contentIndex += 1;
      if (!rowIsVisible) {
        omitted.push(row);
        continue;
      }
      flushOmitted();
      result.push(row);
      continue;
    }

    // A no-newline marker describes an adjacent changed row, which is always visible.
    flushOmitted();
    result.push(row);
  }
  flushOmitted();
  return result;
}

/**
 * Produces a compact, presentation-only view of a parsed unified diff.
 * File headers and metadata remain untouched; each hunk keeps at most the requested
 * number of context lines around changes and receives typed separators for omissions.
 */
export function createContextualDiffRows(
  rows: readonly UnifiedDiffRow[],
  contextLines = DEFAULT_CONTEXT_LINES,
): readonly ContextualDiffRow[] {
  const context = normalizedContextLines(contextLines);
  const result: ContextualDiffRow[] = [];

  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index];
    result.push(row);
    if (row.kind !== 'hunk-header') continue;

    let bodyEnd = index + 1;
    while (
      bodyEnd < rows.length &&
      rows[bodyEnd].kind !== 'hunk-header' &&
      rows[bodyEnd].kind !== 'file-header'
    ) {
      bodyEnd += 1;
    }
    result.push(...contextualHunkBody(rows.slice(index + 1, bodyEnd), row.hunkKey, context));
    index = bodyEnd - 1;
  }

  return result;
}
