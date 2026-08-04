import assert from 'node:assert/strict';
import test from 'node:test';

import {
  createMicrosoftStoreManifest,
  storeVersion,
} from './create-microsoft-store-msix-manifest.mjs';

test('converts an application semver to a four-part Store version', () => {
  assert.equal(storeVersion('0.1.6'), '0.1.6.0');
  assert.equal(storeVersion('1.2.3-beta.1'), '1.2.3.0');
});

test('creates a full-trust desktop MSIX manifest with Store identity', () => {
  const manifest = createMicrosoftStoreManifest({
    identityName: '12345RafalBrzoska.SkibidibiGit',
    publisher: 'CN=01234567-89ab-cdef-0123-456789abcdef',
    publisherDisplayName: 'Rafal & Brzoska',
    version: '0.1.6',
  });

  assert.match(manifest, /EntryPoint="Windows\.FullTrustApplication"/);
  assert.match(manifest, /rescap:Capability Name="runFullTrust"/);
  assert.match(manifest, /Version="0\.1\.6\.0"/);
  assert.match(manifest, /Rafal &amp; Brzoska/);
  assert.doesNotMatch(manifest, /Square310x310Logo/);
});

test('rejects guessed or malformed Store identity values', () => {
  assert.throws(
    () => createMicrosoftStoreManifest({
      identityName: 'not valid!',
      publisher: 'Rafal Brzoska',
      publisherDisplayName: 'Rafal Brzoska',
      version: '0.1.6',
    }),
    /copied exactly from Partner Center/,
  );
});
