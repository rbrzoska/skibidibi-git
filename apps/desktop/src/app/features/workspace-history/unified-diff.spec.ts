import { describe, expect, it } from 'vitest';

import { parseUnifiedDiff } from './unified-diff';

describe('parseUnifiedDiff', () => {
  it('returns an explicit empty model', () => {
    expect(parseUnifiedDiff('')).toEqual({
      files: [],
      rows: [],
      isEmpty: true,
      hasBinaryFiles: false,
    });
  });

  it('parses multiple hunks with exact content and old/new line numbers', () => {
    const result = parseUnifiedDiff(
      [
        'diff --git a/src/app.ts b/src/app.ts',
        'index 1111111..2222222 100644',
        '--- a/src/app.ts',
        '+++ b/src/app.ts',
        '@@ -2,3 +2,4 @@ function run() {',
        ' context',
        '-\told();',
        '+\tnewOne();',
        '+\tnewTwo();',
        ' tail',
        '@@ -20 +21 @@ second',
        '-before',
        '+after',
      ].join('\n'),
    );

    expect(result.files).toHaveLength(1);
    expect(result.files[0]).toMatchObject({
      oldPath: 'src/app.ts',
      newPath: 'src/app.ts',
      displayPath: 'src/app.ts',
      changeKind: 'modified',
      binary: false,
    });
    expect(result.files[0].hunks).toHaveLength(2);
    expect(result.files[0].hunks[0]).toMatchObject({
      oldStart: 2,
      oldCount: 3,
      newStart: 2,
      newCount: 4,
      section: 'function run() {',
    });
    expect(
      result.rows
        .filter((row) => ['context', 'addition', 'deletion'].includes(row.kind))
        .map((row) =>
          row.kind === 'metadata' || row.kind === 'file-header' || row.kind === 'hunk-header' || row.kind === 'no-newline' || row.kind === 'binary'
            ? null
            : [row.kind, row.content, row.oldLineNumber, row.newLineNumber],
        ),
    ).toEqual([
      ['context', 'context', 2, 2],
      ['deletion', '\told();', 3, null],
      ['addition', '\tnewOne();', null, 3],
      ['addition', '\tnewTwo();', null, 4],
      ['context', 'tail', 4, 5],
      ['deletion', 'before', 20, null],
      ['addition', 'after', null, 21],
    ]);
    expect(new Set(result.rows.map((row) => row.key)).size).toBe(result.rows.length);
  });

  it('keeps malicious HTML as inert plain text and never generates markup', () => {
    const attack = '<img src=x onerror=alert(1)>\t<script>steal()</script>';
    const result = parseUnifiedDiff(
      ['diff --git a/x b/x', '--- a/x', '+++ b/x', '@@ -0,0 +1 @@', `+${attack}`].join('\n'),
    );
    const addition = result.rows.find((row) => row.kind === 'addition');

    expect(addition).toMatchObject({ raw: `+${attack}`, content: attack });
    expect(JSON.stringify(result)).not.toContain('&lt;');
  });

  it('preserves a very long row without truncating it', () => {
    const content = `${'x'.repeat(200_000)}\tEND`;
    const result = parseUnifiedDiff(
      ['diff --git a/large b/large', '--- a/large', '+++ b/large', '@@ -1 +1 @@', `+${content}`].join('\n'),
    );
    const addition = result.rows.find((row) => row.kind === 'addition');

    expect(addition?.kind === 'addition' ? addition.content : null).toBe(content);
  });

  it('marks malformed and unsafe hunk headers as metadata instead of inventing line numbers', () => {
    const result = parseUnifiedDiff(
      [
        'diff --git a/x b/x',
        '@@ -one +1 @@',
        '@@ -1, +2 @@',
        '@@ -999999999999999999999999 +1 @@',
        '@@ -1 +2 @@missing-space',
      ].join('\n'),
    );

    expect(result.files[0].hunks).toEqual([]);
    expect(
      result.rows.filter((row) => row.kind === 'metadata').map((row) => row.metadataKind),
    ).toEqual([
      'invalid-hunk-header',
      'invalid-hunk-header',
      'invalid-hunk-header',
      'invalid-hunk-header',
    ]);
  });

  it('represents the no-newline marker without advancing either counter', () => {
    const result = parseUnifiedDiff(
      [
        'diff --git a/x b/x',
        '--- a/x',
        '+++ b/x',
        '@@ -4 +4 @@',
        '-old',
        '\\ No newline at end of file',
        '+new',
        '\\ No newline at end of file',
      ].join('\n'),
    );

    expect(result.rows.filter((row) => row.kind === 'no-newline')).toHaveLength(2);
    expect(result.rows.find((row) => row.kind === 'addition')).toMatchObject({
      oldLineNumber: null,
      newLineNumber: 4,
    });
  });

  it('detects textual and Git binary patch indicators', () => {
    const result = parseUnifiedDiff(
      [
        'diff --git a/image.png b/image.png',
        'index 111..222 100644',
        'Binary files a/image.png and b/image.png differ',
        'diff --git a/archive.bin b/archive.bin',
        'GIT binary patch',
        'literal 3',
        'KcmZQzU|?Vb0000',
      ].join('\n'),
    );

    expect(result.hasBinaryFiles).toBe(true);
    expect(result.files).toHaveLength(2);
    expect(result.files.map((file) => [file.binary, file.changeKind])).toEqual([
      [true, 'binary'],
      [true, 'binary'],
    ]);
    expect(result.rows.filter((row) => row.kind === 'binary')).toHaveLength(2);
  });

  it('captures rename and copy metadata and uses the destination as display path', () => {
    const renamed = parseUnifiedDiff(
      [
        'diff --git a/old name.ts b/new name.ts',
        'similarity index 98%',
        'rename from old name.ts',
        'rename to new name.ts',
      ].join('\n'),
    );
    const copied = parseUnifiedDiff(
      ['diff --git a/source.ts b/copy.ts', 'copy from source.ts', 'copy to copy.ts'].join('\n'),
    );

    expect(renamed.files[0]).toMatchObject({
      displayPath: 'new name.ts',
      changeKind: 'renamed',
    });
    expect(renamed.files[0].metadata.map((row) => row.metadataKind)).toEqual([
      'similarity',
      'rename-from',
      'rename-to',
    ]);
    expect(copied.files[0]).toMatchObject({ displayPath: 'copy.ts', changeKind: 'copied' });
  });

  it('recognizes added and deleted files through /dev/null and mode metadata', () => {
    const result = parseUnifiedDiff(
      [
        'diff --git a/new.txt b/new.txt',
        'new file mode 100644',
        '--- /dev/null',
        '+++ b/new.txt',
        '@@ -0,0 +1 @@',
        '+new',
        'diff --git a/old.txt b/old.txt',
        'deleted file mode 100644',
        '--- a/old.txt',
        '+++ /dev/null',
        '@@ -1 +0,0 @@',
        '-old',
      ].join('\n'),
    );

    expect(result.files.map((file) => [file.changeKind, file.oldPath, file.newPath])).toEqual([
      ['added', null, 'new.txt'],
      ['deleted', 'old.txt', null],
    ]);
  });

  it('supports a headerless unified patch and quoted Git paths', () => {
    const headerless = parseUnifiedDiff(
      ['--- a/plain.txt', '+++ b/plain.txt', '@@ -1 +1 @@', '-a', '+b'].join('\n'),
    );
    const quoted = parseUnifiedDiff(
      [
        'diff --git "a/folder/file\\tname.txt" "b/folder/file\\tname.txt"',
        '--- "a/folder/file\\tname.txt"',
        '+++ "b/folder/file\\tname.txt"',
      ].join('\n'),
    );

    expect(headerless.files[0]).toMatchObject({
      oldPath: 'plain.txt',
      newPath: 'plain.txt',
      displayPath: 'plain.txt',
    });
    expect(quoted.files[0]).toMatchObject({
      oldPath: 'folder/file\tname.txt',
      newPath: 'folder/file\tname.txt',
    });
  });

  it('creates stable keys for repeated parses and does not add a row for a terminal LF', () => {
    const patch = 'diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n';
    const first = parseUnifiedDiff(patch);
    const second = parseUnifiedDiff(patch);

    expect(first.rows.map((row) => row.key)).toEqual(second.rows.map((row) => row.key));
    expect(first.rows.at(-1)).toMatchObject({ kind: 'addition', raw: '+b' });
  });
});
