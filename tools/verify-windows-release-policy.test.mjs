import assert from 'node:assert/strict';
import test from 'node:test';

import { verifyWindowsReleasePolicy } from './verify-windows-release-policy.mjs';

const config = {
  productName: 'Skibidibi Git',
  bundle: {
    windows: {
      nsis: { installMode: 'perMachine' },
      wix: { enableElevatedUpdateTask: true },
    },
  },
};

test('accepts the Windows GUI subsystem attribute with LF or CRLF line endings', () => {
  const attribute = '#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]';

  assert.doesNotThrow(() => verifyWindowsReleasePolicy(config, `${attribute}\n\nfn main() {}`));
  assert.doesNotThrow(() => verifyWindowsReleasePolicy(config, `${attribute}\r\n\r\nfn main() {}`));
});

test('rejects a production entry point without the Windows GUI subsystem attribute', () => {
  assert.throws(
    () => verifyWindowsReleasePolicy(config, 'fn main() {}\n'),
    /Windows GUI subsystem/,
  );
});
