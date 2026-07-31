import assert from 'node:assert/strict';
import test from 'node:test';

import { releaseNotesForVersion } from './current-release-notes.mjs';

test('extracts one exact release without including the next version', () => {
  const markdown = [
    '# Releases',
    '',
    '## 1.2.0 — today',
    '',
    '- New',
    '',
    '## 1.1.0 — yesterday',
    '',
    '- Old',
  ].join('\n');

  assert.equal(
    releaseNotesForVersion(markdown, '1.2.0'),
    '## 1.2.0 — today\n\n- New',
  );
});

test('supports prerelease versions and rejects a missing section', () => {
  assert.equal(
    releaseNotesForVersion('## 2.0.0-beta.1\n\n- Preview', '2.0.0-beta.1'),
    '## 2.0.0-beta.1\n\n- Preview',
  );
  assert.throws(
    () => releaseNotesForVersion('## 1.0.0\n', '2.0.0'),
    /does not contain a section/,
  );
});
