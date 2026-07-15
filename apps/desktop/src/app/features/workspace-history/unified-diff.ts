export type UnifiedDiffChangeKind =
  | 'modified'
  | 'added'
  | 'deleted'
  | 'renamed'
  | 'copied'
  | 'binary';

export type UnifiedDiffMetadataKind =
  | 'old-file'
  | 'new-file'
  | 'old-mode'
  | 'new-mode'
  | 'deleted-file-mode'
  | 'new-file-mode'
  | 'similarity'
  | 'dissimilarity'
  | 'rename-from'
  | 'rename-to'
  | 'copy-from'
  | 'copy-to'
  | 'index'
  | 'invalid-hunk-header'
  | 'other';

interface UnifiedDiffRowBase {
  readonly key: string;
  readonly fileKey: string;
  /** The source line exactly as received, excluding its LF separator. */
  readonly raw: string;
}

export interface UnifiedDiffFileHeaderRow extends UnifiedDiffRowBase {
  readonly kind: 'file-header';
  readonly oldPath: string | null;
  readonly newPath: string | null;
  readonly displayPath: string;
}

export interface UnifiedDiffMetadataRow extends UnifiedDiffRowBase {
  readonly kind: 'metadata';
  readonly metadataKind: UnifiedDiffMetadataKind;
}

export interface UnifiedDiffHunkHeaderRow extends UnifiedDiffRowBase {
  readonly kind: 'hunk-header';
  readonly hunkKey: string;
  readonly oldStart: number;
  readonly oldCount: number;
  readonly newStart: number;
  readonly newCount: number;
  readonly section: string;
}

export interface UnifiedDiffContentRow extends UnifiedDiffRowBase {
  readonly kind: 'context' | 'addition' | 'deletion';
  readonly hunkKey: string;
  /** Source line without the one-character diff marker. Tabs and all other text are intact. */
  readonly content: string;
  readonly oldLineNumber: number | null;
  readonly newLineNumber: number | null;
}

export interface UnifiedDiffNoNewlineRow extends UnifiedDiffRowBase {
  readonly kind: 'no-newline';
  readonly hunkKey: string | null;
}

export interface UnifiedDiffBinaryRow extends UnifiedDiffRowBase {
  readonly kind: 'binary';
}

export type UnifiedDiffRow =
  | UnifiedDiffFileHeaderRow
  | UnifiedDiffMetadataRow
  | UnifiedDiffHunkHeaderRow
  | UnifiedDiffContentRow
  | UnifiedDiffNoNewlineRow
  | UnifiedDiffBinaryRow;

export interface UnifiedDiffHunk {
  readonly key: string;
  readonly oldStart: number;
  readonly oldCount: number;
  readonly newStart: number;
  readonly newCount: number;
  readonly section: string;
  readonly rows: readonly UnifiedDiffRow[];
}

export interface UnifiedDiffFile {
  readonly key: string;
  readonly oldPath: string | null;
  readonly newPath: string | null;
  readonly displayPath: string;
  readonly changeKind: UnifiedDiffChangeKind;
  readonly metadata: readonly UnifiedDiffMetadataRow[];
  readonly hunks: readonly UnifiedDiffHunk[];
  readonly binary: boolean;
  readonly rows: readonly UnifiedDiffRow[];
}

export interface UnifiedDiffModel {
  readonly files: readonly UnifiedDiffFile[];
  /** All file rows in source order, ready for a flat Angular `@for (row of rows; track row.key)`. */
  readonly rows: readonly UnifiedDiffRow[];
  readonly isEmpty: boolean;
  readonly hasBinaryFiles: boolean;
}

interface MutableHunk {
  readonly key: string;
  readonly oldStart: number;
  readonly oldCount: number;
  readonly newStart: number;
  readonly newCount: number;
  readonly section: string;
  readonly rows: UnifiedDiffRow[];
  nextOldLine: number;
  nextNewLine: number;
}

interface MutableFile {
  readonly key: string;
  oldPath: string | null;
  newPath: string | null;
  renameFrom: string | null;
  renameTo: string | null;
  copyFrom: string | null;
  copyTo: string | null;
  added: boolean;
  deleted: boolean;
  binary: boolean;
  readonly rows: UnifiedDiffRow[];
  readonly metadata: UnifiedDiffMetadataRow[];
  readonly hunks: MutableHunk[];
}

interface HunkHeader {
  readonly oldStart: number;
  readonly oldCount: number;
  readonly newStart: number;
  readonly newCount: number;
  readonly section: string;
}

function sourceLines(patch: string): readonly string[] {
  if (patch.length === 0) return [];
  const lines = patch.split('\n');
  // A terminal LF separates lines; it does not introduce a phantom patch row.
  if (lines.at(-1) === '') lines.pop();
  return lines;
}

function safeInteger(value: string): number | null {
  if (value.length === 0 || value.length > 15) return null;
  let result = 0;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index) - 48;
    if (code < 0 || code > 9) return null;
    result = result * 10 + code;
  }
  return Number.isSafeInteger(result) ? result : null;
}

function parseRange(value: string): { readonly start: number; readonly count: number } | null {
  const comma = value.indexOf(',');
  const startText = comma < 0 ? value : value.slice(0, comma);
  const countText = comma < 0 ? '1' : value.slice(comma + 1);
  const start = safeInteger(startText);
  const count = safeInteger(countText);
  return start === null || count === null ? null : { start, count };
}

function parseHunkHeader(line: string): HunkHeader | null {
  if (!line.startsWith('@@ -')) return null;
  const oldEnd = line.indexOf(' +', 4);
  if (oldEnd < 0) return null;
  const newEnd = line.indexOf(' @@', oldEnd + 2);
  if (newEnd < 0) return null;

  const oldRange = parseRange(line.slice(4, oldEnd));
  const newRange = parseRange(line.slice(oldEnd + 2, newEnd));
  if (!oldRange || !newRange) return null;

  const suffix = line.slice(newEnd + 3);
  if (suffix.length > 0 && !suffix.startsWith(' ')) return null;
  return {
    oldStart: oldRange.start,
    oldCount: oldRange.count,
    newStart: newRange.start,
    newCount: newRange.count,
    section: suffix.startsWith(' ') ? suffix.slice(1) : '',
  };
}

function unquoteGitPath(value: string): string {
  if (!value.startsWith('"')) return value;
  let result = '';
  for (let index = 1; index < value.length; index += 1) {
    const char = value[index];
    if (char === '"') return result;
    if (char !== '\\') {
      result += char;
      continue;
    }

    const escaped = value[index + 1];
    if (escaped === undefined) return result + '\\';
    index += 1;
    const simpleEscapes: Readonly<Record<string, string>> = {
      '"': '"',
      '\\': '\\',
      a: '\u0007',
      b: '\b',
      f: '\f',
      n: '\n',
      r: '\r',
      t: '\t',
      v: '\u000b',
    };
    const simple = simpleEscapes[escaped];
    if (simple !== undefined) {
      result += simple;
      continue;
    }

    if (escaped >= '0' && escaped <= '7') {
      const bytes: number[] = [];
      let firstDigit = escaped;
      while (true) {
        let octal = firstDigit;
        for (let count = 0; count < 2; count += 1) {
          const next = value[index + 1];
          if (next === undefined || next < '0' || next > '7') break;
          octal += next;
          index += 1;
        }
        bytes.push(Number.parseInt(octal, 8));
        if (value[index + 1] !== '\\') break;
        const nextDigit = value[index + 2];
        if (nextDigit === undefined || nextDigit < '0' || nextDigit > '7') break;
        index += 2;
        firstDigit = nextDigit;
      }
      result += new TextDecoder().decode(Uint8Array.from(bytes));
      continue;
    }

    // Unknown escapes remain visible instead of silently corrupting a path.
    result += `\\${escaped}`;
  }
  return result;
}

function normalizedPath(value: string): string | null {
  const decoded = unquoteGitPath(value);
  if (decoded === '/dev/null') return null;
  return decoded.startsWith('a/') || decoded.startsWith('b/') ? decoded.slice(2) : decoded;
}

function parseDiffGitPaths(line: string): readonly [string | null, string | null] {
  const value = line.slice('diff --git '.length);
  if (value.startsWith('"')) {
    let escaped = false;
    let end = -1;
    for (let index = 1; index < value.length; index += 1) {
      if (!escaped && value[index] === '"') {
        end = index;
        break;
      }
      escaped = !escaped && value[index] === '\\';
      if (value[index] !== '\\') escaped = false;
    }
    if (end >= 0 && value[end + 1] === ' ') {
      return [normalizedPath(value.slice(0, end + 1)), normalizedPath(value.slice(end + 2))];
    }
  }

  const separator = value.indexOf(' b/');
  return separator < 0
    ? [null, null]
    : [normalizedPath(value.slice(0, separator)), normalizedPath(value.slice(separator + 1))];
}

function metadataKind(line: string): UnifiedDiffMetadataKind {
  if (line.startsWith('--- ')) return 'old-file';
  if (line.startsWith('+++ ')) return 'new-file';
  if (line.startsWith('old mode ')) return 'old-mode';
  if (line.startsWith('new mode ')) return 'new-mode';
  if (line.startsWith('deleted file mode ')) return 'deleted-file-mode';
  if (line.startsWith('new file mode ')) return 'new-file-mode';
  if (line.startsWith('similarity index ')) return 'similarity';
  if (line.startsWith('dissimilarity index ')) return 'dissimilarity';
  if (line.startsWith('rename from ')) return 'rename-from';
  if (line.startsWith('rename to ')) return 'rename-to';
  if (line.startsWith('copy from ')) return 'copy-from';
  if (line.startsWith('copy to ')) return 'copy-to';
  if (line.startsWith('index ')) return 'index';
  if (line.startsWith('@@')) return 'invalid-hunk-header';
  return 'other';
}

function displayPath(file: MutableFile): string {
  return file.renameTo ?? file.copyTo ?? file.newPath ?? file.renameFrom ?? file.copyFrom ?? file.oldPath ?? '(unknown file)';
}

function changeKind(file: MutableFile): UnifiedDiffChangeKind {
  if (file.binary) return 'binary';
  if (file.renameFrom !== null || file.renameTo !== null) return 'renamed';
  if (file.copyFrom !== null || file.copyTo !== null) return 'copied';
  if (file.added || file.oldPath === null) return 'added';
  if (file.deleted || file.newPath === null) return 'deleted';
  return 'modified';
}

function newFile(index: number, oldPath: string | null = null, newPath: string | null = null): MutableFile {
  return {
    key: `file-${index}`,
    oldPath,
    newPath,
    renameFrom: null,
    renameTo: null,
    copyFrom: null,
    copyTo: null,
    added: false,
    deleted: false,
    binary: false,
    rows: [],
    metadata: [],
    hunks: [],
  };
}

/** Parses Git's unified patch format without interpreting row content as markup. */
export function parseUnifiedDiff(patch: string): UnifiedDiffModel {
  const lines = sourceLines(patch);
  if (lines.length === 0) {
    return { files: [], rows: [], isEmpty: true, hasBinaryFiles: false };
  }

  const mutableFiles: MutableFile[] = [];
  let file: MutableFile | null = null;
  let hunk: MutableHunk | null = null;

  const ensureFile = (): MutableFile => {
    if (file === null) {
      file = newFile(mutableFiles.length);
      mutableFiles.push(file);
    }
    return file;
  };

  for (const line of lines) {
    if (line.startsWith('diff --git ')) {
      const [oldPath, newPath] = parseDiffGitPaths(line);
      file = newFile(mutableFiles.length, oldPath, newPath);
      mutableFiles.push(file);
      hunk = null;
      file.rows.push({
        kind: 'file-header',
        key: `${file.key}:row-0`,
        fileKey: file.key,
        raw: line,
        oldPath,
        newPath,
        displayPath: newPath ?? oldPath ?? '(unknown file)',
      });
      continue;
    }

    const activeFile = ensureFile();
    const rowKey = `${activeFile.key}:row-${activeFile.rows.length}`;
    const parsedHeader = parseHunkHeader(line);
    if (parsedHeader !== null) {
      const hunkKey = `${activeFile.key}:hunk-${activeFile.hunks.length}`;
      hunk = { ...parsedHeader, key: hunkKey, rows: [], nextOldLine: parsedHeader.oldStart, nextNewLine: parsedHeader.newStart };
      activeFile.hunks.push(hunk);
      const row: UnifiedDiffHunkHeaderRow = {
        kind: 'hunk-header', key: rowKey, fileKey: activeFile.key, hunkKey, raw: line, ...parsedHeader,
      };
      activeFile.rows.push(row);
      hunk.rows.push(row);
      continue;
    }

    if (line === '\\ No newline at end of file') {
      const row: UnifiedDiffNoNewlineRow = {
        kind: 'no-newline', key: rowKey, fileKey: activeFile.key, hunkKey: hunk?.key ?? null, raw: line,
      };
      activeFile.rows.push(row);
      hunk?.rows.push(row);
      continue;
    }

    if (hunk !== null && (line.startsWith(' ') || line.startsWith('+') || line.startsWith('-'))) {
      const marker = line[0];
      const row: UnifiedDiffContentRow = {
        kind: marker === '+' ? 'addition' : marker === '-' ? 'deletion' : 'context',
        key: rowKey,
        fileKey: activeFile.key,
        hunkKey: hunk.key,
        raw: line,
        content: line.slice(1),
        oldLineNumber: marker === '+' ? null : hunk.nextOldLine,
        newLineNumber: marker === '-' ? null : hunk.nextNewLine,
      };
      if (marker !== '+') hunk.nextOldLine += 1;
      if (marker !== '-') hunk.nextNewLine += 1;
      activeFile.rows.push(row);
      hunk.rows.push(row);
      continue;
    }

    if (line === 'GIT binary patch' || (line.startsWith('Binary files ') && line.endsWith(' differ'))) {
      activeFile.binary = true;
      hunk = null;
      activeFile.rows.push({ kind: 'binary', key: rowKey, fileKey: activeFile.key, raw: line });
      continue;
    }

    hunk = null;
    const kind = metadataKind(line);
    if (kind === 'old-file') activeFile.oldPath = normalizedPath(line.slice(4));
    if (kind === 'new-file') activeFile.newPath = normalizedPath(line.slice(4));
    if (kind === 'new-file-mode') activeFile.added = true;
    if (kind === 'deleted-file-mode') activeFile.deleted = true;
    if (kind === 'rename-from') activeFile.renameFrom = unquoteGitPath(line.slice('rename from '.length));
    if (kind === 'rename-to') activeFile.renameTo = unquoteGitPath(line.slice('rename to '.length));
    if (kind === 'copy-from') activeFile.copyFrom = unquoteGitPath(line.slice('copy from '.length));
    if (kind === 'copy-to') activeFile.copyTo = unquoteGitPath(line.slice('copy to '.length));
    const row: UnifiedDiffMetadataRow = {
      kind: 'metadata', key: rowKey, fileKey: activeFile.key, raw: line, metadataKind: kind,
    };
    activeFile.rows.push(row);
    activeFile.metadata.push(row);
  }

  const files: readonly UnifiedDiffFile[] = mutableFiles.map((candidate) => {
    const finalDisplayPath = displayPath(candidate);
    const finalRows = candidate.rows.map((row): UnifiedDiffRow =>
      row.kind === 'file-header'
        ? { ...row, oldPath: candidate.oldPath, newPath: candidate.newPath, displayPath: finalDisplayPath }
        : row,
    );
    const rowByKey = new Map(finalRows.map((row) => [row.key, row]));
    return {
      key: candidate.key,
      oldPath: candidate.oldPath,
      newPath: candidate.newPath,
      displayPath: finalDisplayPath,
      changeKind: changeKind(candidate),
      metadata: candidate.metadata,
      hunks: candidate.hunks.map((item) => ({
        key: item.key,
        oldStart: item.oldStart,
        oldCount: item.oldCount,
        newStart: item.newStart,
        newCount: item.newCount,
        section: item.section,
        rows: item.rows.map((row) => rowByKey.get(row.key) ?? row),
      })),
      binary: candidate.binary,
      rows: finalRows,
    };
  });

  return {
    files,
    rows: files.flatMap((candidate) => candidate.rows),
    isEmpty: false,
    hasBinaryFiles: files.some((candidate) => candidate.binary),
  };
}
