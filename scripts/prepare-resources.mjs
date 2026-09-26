import { access, constants } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const resources = [
  join(root, 'yt-dlp.exe'),
  join(root, 'lib', 'ffmpeg.exe'),
  join(root, 'lib', 'ffprobe.exe'),
];

const missing = [];
for (const resource of resources) {
  try {
    await access(resource, constants.X_OK);
  } catch {
    missing.push(resource);
  }
}
if (missing.length) {
  console.warn('Optional bundled runtime files are missing:');
  for (const resource of missing) console.warn(`  ${resource}`);
  console.warn('Development UI/build remains available; add licensed release binaries before packaging.');
}
