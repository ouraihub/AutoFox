import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const expectedNode = readFileSync(new URL('../.nvmrc', import.meta.url), 'utf8').trim();
const expectedPnpm = '12.6.0';
if (process.versions.node !== expectedNode) {
  console.error(`Expected Node ${expectedNode}, got ${process.versions.node}`);
  process.exit(1);
}
let pnpm;
try {
  pnpm = (process.platform === 'win32'
    ? execFileSync(process.env.COMSPEC ?? 'cmd.exe', ['/d', '/s', '/c', 'pnpm --version'], { encoding: 'utf8' })
    : execFileSync('pnpm', ['--version'], { encoding: 'utf8' })).trim();
} catch {
  console.error(`Expected pnpm ${expectedPnpm}, but pnpm was not found`);
  process.exit(1);
}
if (pnpm !== expectedPnpm) {
  console.error(`Expected pnpm ${expectedPnpm}, got ${pnpm}`);
  process.exit(1);
}
