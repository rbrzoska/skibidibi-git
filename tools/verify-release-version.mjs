import { readFile } from 'node:fs/promises';

const rootPackage = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
const desktopPackage = JSON.parse(await readFile(new URL('../apps/desktop/package.json', import.meta.url), 'utf8'));
const tauriConfig = JSON.parse(await readFile(new URL('../apps/desktop/src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const cargoManifest = await readFile(new URL('../Cargo.toml', import.meta.url), 'utf8');
const cargoVersion = cargoManifest.match(/\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/)?.[1];
const versions = new Map([
  ['package.json', rootPackage.version],
  ['apps/desktop/package.json', desktopPackage.version],
  ['tauri.conf.json', tauriConfig.version],
  ['Cargo.toml', cargoVersion],
]);
const expected = tauriConfig.version;
const mismatches = [...versions].filter(([, version]) => version !== expected);

if (mismatches.length > 0) {
  throw new Error(`Release versions must match ${expected}: ${mismatches.map(([file, version]) => `${file}=${version ?? 'missing'}`).join(', ')}`);
}

const tag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : null;
if (tag !== null && tag !== `app-v${expected}`) {
  throw new Error(`Release tag ${tag} does not match application version app-v${expected}.`);
}

console.log(`Release version ${expected} is consistent.`);
