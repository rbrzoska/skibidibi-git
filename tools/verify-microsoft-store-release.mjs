import { readFile } from 'node:fs/promises';

const readJson = async (relativePath) => JSON.parse(
  await readFile(new URL(relativePath, import.meta.url), 'utf8'),
);
const readText = (relativePath) => readFile(new URL(relativePath, import.meta.url), 'utf8');

const [config, storeConfig, privacy, support] = await Promise.all([
  readJson('../apps/desktop/src-tauri/tauri.conf.json'),
  readJson('../apps/desktop/src-tauri/tauri.microsoftstore.conf.json'),
  readText('../PRIVACY.md'),
  readText('../SUPPORT.md'),
]);

const storeWindows = storeConfig.bundle?.windows;
if (storeWindows?.webviewInstallMode?.type !== 'offlineInstaller' || storeWindows.webviewInstallMode.silent !== true) {
  throw new Error('The Microsoft Store installer must embed and silently install the offline WebView2 runtime.');
}
if (storeWindows.allowDowngrades !== false) {
  throw new Error('The Microsoft Store installer must reject downgrades.');
}
if (storeConfig.bundle?.createUpdaterArtifacts !== false) {
  throw new Error('Microsoft Store builds must not create standalone updater artifacts.');
}
if (storeWindows.wix?.enableElevatedUpdateTask !== false) {
  throw new Error('Microsoft Store builds must not install the elevated self-update scheduled task.');
}
if (!storeConfig.bundle?.publisher || storeConfig.bundle.publisher === config.productName) {
  throw new Error('The Microsoft Store publisher must be explicit and different from the product name.');
}
if (!privacy.includes('## Information processed locally') || !privacy.includes('## Contact')) {
  throw new Error('PRIVACY.md is missing required disclosure or contact sections.');
}
if (!support.includes('## System requirements') || !support.includes('Git installation available on `PATH`')) {
  throw new Error('SUPPORT.md must document the external Git requirement.');
}
if (/TODO|example\.com|replace me/i.test(`${privacy}\n${support}`)) {
  throw new Error('Privacy and support documents must not contain placeholders.');
}

console.log('Microsoft Store source policy is valid. Store MSIX signing is handled by Microsoft after certification.');
