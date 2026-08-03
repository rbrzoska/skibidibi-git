import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const ARCHITECTURES = new Set(['x64', 'x86', 'arm64']);

function xml(value) {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&apos;');
}

export function storeVersion(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(version);
  if (!match) {
    throw new Error(`The application version "${version}" is not a supported semantic version.`);
  }

  const parts = match.slice(1).map(Number);
  if (parts.some((part) => part > 65_535)) {
    throw new Error('Each Microsoft Store version component must be between 0 and 65535.');
  }

  return `${parts.join('.')}.0`;
}

export function createMicrosoftStoreManifest({
  identityName,
  publisher,
  publisherDisplayName,
  version,
  architecture = 'x64',
  executable = 'SkibidibiGit.exe',
}) {
  if (!/^[A-Za-z0-9.-]{3,50}$/.test(identityName)) {
    throw new Error('Package/Identity/Name must be copied exactly from Partner Center.');
  }
  if (!publisher.startsWith('CN=') || publisher.length > 8192) {
    throw new Error('Package/Identity/Publisher must be the CN= value from Partner Center.');
  }
  if (!publisherDisplayName.trim()) {
    throw new Error('Publisher display name is required.');
  }
  if (!ARCHITECTURES.has(architecture)) {
    throw new Error(`Unsupported MSIX architecture: ${architecture}.`);
  }
  if (!/^[A-Za-z0-9._-]+\.exe$/i.test(executable)) {
    throw new Error('The packaged executable must be a simple .exe file name.');
  }

  const packageVersion = storeVersion(version);
  const values = Object.fromEntries(
    Object.entries({
      identityName,
      publisher,
      publisherDisplayName,
      packageVersion,
      architecture,
      executable,
    }).map(([key, value]) => [key, xml(value)]),
  );

  return `<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap rescap">
  <Identity Name="${values.identityName}" Publisher="${values.publisher}" Version="${values.packageVersion}" ProcessorArchitecture="${values.architecture}" />
  <Properties>
    <DisplayName>Skibidibi Git</DisplayName>
    <PublisherDisplayName>${values.publisherDisplayName}</PublisherDisplayName>
    <Description>A fast local-first desktop Git client.</Description>
    <Logo>Assets\\StoreLogo.png</Logo>
  </Properties>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Applications>
    <Application Id="SkibidibiGit" Executable="${values.executable}" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="Skibidibi Git"
        Description="A fast local-first desktop Git client."
        BackgroundColor="transparent"
        Square44x44Logo="Assets\\Square44x44Logo.png"
        Square150x150Logo="Assets\\Square150x150Logo.png">
        <uap:DefaultTile Square310x310Logo="Assets\\Square310x310Logo.png" />
      </uap:VisualElements>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
`;
}

function argument(name) {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function main() {
  const output = argument('output');
  const configPath = new URL('../apps/desktop/src-tauri/tauri.conf.json', import.meta.url);
  if (!output) {
    throw new Error('Missing --output path.');
  }

  const config = JSON.parse(await readFile(configPath, 'utf8'));
  const manifest = createMicrosoftStoreManifest({
    identityName: argument('identity-name') ?? '',
    publisher: argument('publisher') ?? '',
    publisherDisplayName: argument('publisher-display-name') ?? '',
    architecture: argument('architecture') ?? 'x64',
    executable: argument('executable') ?? 'SkibidibiGit.exe',
    version: config.version,
  });
  await writeFile(output, manifest);
  console.log(`Created Microsoft Store manifest ${output}.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
