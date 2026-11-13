// Writes dist/checksums.json, the archive checksums a release build of the extension downloads
// against, from the release archives the same workflow run built:
//
//   npm run checksums -- <version> <archives-dir>
//
// Run only for a release; a .vsix without this file refuses to download groovyfmt.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { checksumsOf } from './binary';

const [version, dir] = process.argv.slice(2);
if (!version || !dir) {
  console.error('usage: npm run checksums -- <version> <archives-dir>');
  process.exit(2);
}
const sums = checksumsOf(version.replace(/^v/, ''), dir);
mkdirSync('dist', { recursive: true });
writeFileSync(join('dist', 'checksums.json'), `${JSON.stringify(sums, null, 2)}\n`);
console.log(`dist/checksums.json: ${Object.keys(sums.sha256).length} archives for ${sums.version}`);
