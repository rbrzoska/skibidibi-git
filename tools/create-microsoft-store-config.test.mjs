import assert from 'node:assert/strict';
import test from 'node:test';

import { signedMicrosoftStoreConfig } from './create-microsoft-store-config.mjs';

const template = {
  bundle: {
    publisher: 'Rafal Brzoska',
    windows: {
      allowDowngrades: false,
      webviewInstallMode: { type: 'offlineInstaller', silent: true },
    },
  },
};

test('adds Authenticode settings without dropping the offline Store policy', () => {
  const config = signedMicrosoftStoreConfig(
    template,
    'aa bb cc dd ee ff 00 11 22 33 44 55 66 77 88 99 aa bb cc dd',
    'http://timestamp.example.test',
  );

  assert.deepEqual(config.bundle.windows, {
    allowDowngrades: false,
    webviewInstallMode: { type: 'offlineInstaller', silent: true },
    certificateThumbprint: 'AABBCCDDEEFF00112233445566778899AABBCCDD',
    digestAlgorithm: 'sha256',
    timestampUrl: 'http://timestamp.example.test',
  });
});

test('rejects an invalid thumbprint or timestamp endpoint', () => {
  assert.throws(
    () => signedMicrosoftStoreConfig(template, 'not-a-thumbprint', 'http://timestamp.example.test'),
    /40 hexadecimal/,
  );
  assert.throws(
    () => signedMicrosoftStoreConfig(template, 'A'.repeat(40), 'timestamp.example.test'),
    /absolute HTTP\(S\)/,
  );
});
