import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const templatePath = new URL('../apps/desktop/src-tauri/tauri.microsoftstore.conf.json', import.meta.url);

export function signedMicrosoftStoreConfig(template, certificateThumbprint, timestampUrl) {
  const thumbprint = certificateThumbprint.replaceAll(/\s/g, '').toUpperCase();
  if (!/^[0-9A-F]{40}$/.test(thumbprint)) {
    throw new Error('The Windows signing certificate thumbprint must contain exactly 40 hexadecimal characters.');
  }
  if (!/^https?:\/\/[^/\s]+(?:\/.*)?$/i.test(timestampUrl)) {
    throw new Error('The Windows timestamp URL must be an absolute HTTP(S) URL supplied by the certificate provider.');
  }

  return {
    ...template,
    bundle: {
      ...template.bundle,
      windows: {
        ...template.bundle?.windows,
        certificateThumbprint: thumbprint,
        digestAlgorithm: 'sha256',
        timestampUrl,
      },
    },
  };
}

async function main() {
  const outputIndex = process.argv.indexOf('--output');
  const outputPath = outputIndex >= 0 ? process.argv[outputIndex + 1] : undefined;
  if (!outputPath) {
    throw new Error('Usage: node tools/create-microsoft-store-config.mjs --output <path>');
  }

  const thumbprint = process.env.WINDOWS_CERTIFICATE_THUMBPRINT ?? '';
  const timestampUrl = process.env.WINDOWS_TIMESTAMP_URL ?? '';
  const template = JSON.parse(await readFile(templatePath, 'utf8'));
  const config = signedMicrosoftStoreConfig(template, thumbprint, timestampUrl);
  await writeFile(outputPath, `${JSON.stringify(config, null, 2)}\n`, { mode: 0o600 });
  console.log(`Created signed Microsoft Store Tauri config at ${outputPath}.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
