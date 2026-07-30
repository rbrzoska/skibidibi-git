import { readFile } from 'node:fs/promises';

const configPath = new URL('../apps/desktop/src-tauri/tauri.conf.json', import.meta.url);
const config = JSON.parse(await readFile(configPath, 'utf8'));
const windowsBundle = config.bundle?.windows;

if (config.productName !== 'Skibidibi Git') {
  throw new Error(
    'The Windows Program Files directory is derived from productName, which must remain "Skibidibi Git".',
  );
}

if (windowsBundle?.nsis?.installMode !== 'perMachine') {
  throw new Error(
    'Windows NSIS releases must use bundle.windows.nsis.installMode = "perMachine" so the app installs in Program Files with UAC.',
  );
}

if (windowsBundle?.wix?.enableElevatedUpdateTask !== true) {
  throw new Error(
    'Windows MSI releases must enable bundle.windows.wix.enableElevatedUpdateTask so updates can replace a Program Files installation.',
  );
}

console.log('Windows installer policy is valid.');
