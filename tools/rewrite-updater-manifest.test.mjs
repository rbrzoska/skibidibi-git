import assert from 'node:assert/strict';
import test from 'node:test';

import { rewriteUpdaterManifest } from './rewrite-updater-manifest.mjs';

test('rewrites GitHub API asset URLs to public release downloads', () => {
  const manifest = {
    version: '0.1.3',
    platforms: {
      'darwin-aarch64': {
        signature: 'signature',
        url: 'https://api.github.com/repos/example/app/releases/assets/42',
      },
    },
  };
  const releaseAssets = [{
    apiUrl: 'https://api.github.com/repos/example/app/releases/assets/42',
    name: 'Example App.app.tar.gz',
  }];

  assert.deepEqual(
    rewriteUpdaterManifest(manifest, releaseAssets, 'example/app', 'app-v0.1.3'),
    {
      version: '0.1.3',
      platforms: {
        'darwin-aarch64': {
          signature: 'signature',
          url: 'https://github.com/example/app/releases/download/app-v0.1.3/Example%20App.app.tar.gz',
        },
      },
    },
  );
});

test('fails when an updater asset cannot be matched to the release', () => {
  assert.throws(
    () => rewriteUpdaterManifest(
      {
        platforms: {
          'windows-x86_64': {
            url: 'https://api.github.com/repos/example/app/releases/assets/404',
          },
        },
      },
      [],
      'example/app',
      'app-v0.1.3',
    ),
    /missing from the release/,
  );
});
