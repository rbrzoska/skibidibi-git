import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

export function releaseNotesForVersion(markdown, version) {
  const headings = [...markdown.matchAll(/^##\s+v?(\S+)(?:\s|$).*$/gm)];
  const headingIndex = headings.findIndex((heading) => heading[1] === version);
  if (headingIndex < 0) {
    throw new Error(`RELEASE_NOTES.md does not contain a section for version ${version}.`);
  }
  const start = headings[headingIndex].index;
  const end = headings[headingIndex + 1]?.index ?? markdown.length;
  const notes = markdown.slice(start, end).trim();
  if (notes.length > 32_768) {
    throw new Error(`Release notes for version ${version} exceed 32 KiB.`);
  }
  return notes;
}

async function main() {
  const rootPackage = JSON.parse(
    await readFile(new URL('../package.json', import.meta.url), 'utf8'),
  );
  const markdown = await readFile(
    new URL('../RELEASE_NOTES.md', import.meta.url),
    'utf8',
  );
  const notes = releaseNotesForVersion(markdown, rootPackage.version);
  if (process.argv.includes('--check')) {
    console.log(`Release notes for ${rootPackage.version} are valid.`);
    return;
  }
  process.stdout.write(`${notes}\n`);
}

if (
  process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await main();
}
