import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const packageJson = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'));
const rustMetadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--locked', '--no-deps'], {
  cwd: root,
  encoding: 'utf8',
  maxBuffer: 64 * 1024 * 1024,
}));
const rustPackages = rustMetadata.packages.map((pkg) => ({
  name: pkg.name,
  version: pkg.version,
  license: pkg.license ?? 'UNKNOWN',
}));
const npmDependencies = Object.entries({
  ...(packageJson.dependencies ?? {}),
  ...(packageJson.devDependencies ?? {}),
}).map(([name, version]) => ({ name, version }));
const date = new Date().toISOString().slice(0, 10);
const lines = [
  '# AutoFox Third-Party Notices',
  '',
  `Generated: ${date}`,
  '',
  'This inventory is a release-review input, not a substitute for reviewing each license text.',
  '',
  '## Rust packages',
  '',
  '| Package | Version | License |',
  '| --- | --- | --- |',
  ...rustPackages.map((pkg) => `| ${pkg.name} | ${pkg.version} | ${pkg.license} |`),
  '',
  '## JavaScript packages',
  '',
  '| Package | Declared range |',
  '| --- | --- |',
  ...npmDependencies.map((pkg) => `| ${pkg.name} | ${pkg.version} |`),
  '',
  '## Bundled runtime binaries',
  '',
  '| Binary | Planned path | Version evidence |',
  '| --- | --- | --- |',
  '| yt-dlp.exe | beside AutoFox.exe | Release owner must record `yt-dlp --version` and SHA-256 |',
  '| ffmpeg.exe | lib/ffmpeg.exe | Release owner must record `ffmpeg -version` and SHA-256 |',
  '| ffprobe.exe | lib/ffprobe.exe | Release owner must record `ffprobe -version` and SHA-256 |',
  '',
  '## Review status',
  '',
  '- License texts and NOTICE obligations require release-owner review before distribution.',
  '- Unknown Rust licenses must be resolved before publishing.',
];
writeFileSync(resolve(root, 'THIRD_PARTY_NOTICES.md'), `${lines.join('\n')}\n`);
console.log(`wrote THIRD_PARTY_NOTICES.md (${rustPackages.length} Rust packages, ${npmDependencies.length} npm packages)`);
