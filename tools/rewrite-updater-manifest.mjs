import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

export function rewriteUpdaterManifest(manifest, assets, repository, tag) {
  if (!repository.includes('/') || tag.length === 0) {
    throw new Error('Repository and release tag are required.');
  }

  const assetNamesByApiUrl = new Map(
    assets.map((asset) => [asset.apiUrl, asset.name]),
  );
  let rewrittenCount = 0;

  for (const platform of Object.values(manifest.platforms ?? {})) {
    if (typeof platform?.url !== 'string' || !platform.url.startsWith('https://api.github.com/repos/')) {
      continue;
    }

    const assetName = assetNamesByApiUrl.get(platform.url);
    if (assetName === undefined) {
      throw new Error(`Updater asset is missing from the release: ${platform.url}`);
    }

    platform.url = `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(assetName)}`;
    rewrittenCount += 1;
  }

  if (rewrittenCount === 0) {
    throw new Error('Updater manifest did not contain any GitHub API asset URLs.');
  }

  return manifest;
}

async function main() {
  const [manifestPath, assetsPath, repository, tag] = process.argv.slice(2);
  if (manifestPath === undefined || assetsPath === undefined || repository === undefined || tag === undefined) {
    throw new Error('Usage: rewrite-updater-manifest.mjs <manifest> <assets> <repository> <tag>');
  }

  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  const release = JSON.parse(await readFile(assetsPath, 'utf8'));
  const rewritten = rewriteUpdaterManifest(manifest, release.assets ?? [], repository, tag);
  await writeFile(manifestPath, `${JSON.stringify(rewritten, null, 2)}\n`);
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
