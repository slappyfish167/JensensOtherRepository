// Zips dist/pf2e-stockfish into dist/pf2e-stockfish.zip (release asset) and
// writes dist/module.json (the manifest URL target).
import { execFileSync } from 'node:child_process';
import { copyFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const dist = new URL('../dist', import.meta.url).pathname;
rmSync(join(dist, 'pf2e-stockfish.zip'), { force: true });
execFileSync('zip', ['-qr', '../pf2e-stockfish.zip', '.'], { cwd: join(dist, 'pf2e-stockfish'), stdio: 'inherit' });
copyFileSync(join(dist, 'pf2e-stockfish', 'module.json'), join(dist, 'module.json'));
console.log('dist/pf2e-stockfish.zip, dist/module.json');
