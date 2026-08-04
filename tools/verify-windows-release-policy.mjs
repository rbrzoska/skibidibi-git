import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const configPath = new URL('../apps/desktop/src-tauri/tauri.conf.json', import.meta.url);
const mainPath = new URL('../apps/desktop/src-tauri/src/main.rs', import.meta.url);

export function verifyWindowsReleasePolicy(config, mainSource) {
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

  if (
    !/^#!\[cfg_attr\(not\(debug_assertions\), windows_subsystem = "windows"\)\]\r?\n/.test(
      mainSource,
    )
  ) {
    throw new Error(
      'Production builds must use the Windows GUI subsystem so launching the desktop app does not open a controlling console window.',
    );
  }
}

export async function verifyRepositoryWindowsReleasePolicy() {
  const config = JSON.parse(await readFile(configPath, 'utf8'));
  const mainSource = await readFile(mainPath, 'utf8');
  verifyWindowsReleasePolicy(config, mainSource);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await verifyRepositoryWindowsReleasePolicy();
  console.log('Windows release policy is valid.');
}
