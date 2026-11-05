// Copies static module files next to the Vite output and validates the
// result: dist/pf2e-stockfish/ is exactly the folder Foundry installs.
import { cpSync, existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const out = join(root, 'dist', 'pf2e-stockfish');
for (const dir of ['templates', 'styles', 'lang']) cpSync(join(root, dir), join(out, dir), { recursive: true });

const manifest = JSON.parse(readFileSync(join(root, 'module.json'), 'utf8'));
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
if (manifest.version !== pkg.version) throw new Error(`module.json ${manifest.version} != package.json ${pkg.version}`);
writeFileSync(join(out, 'module.json'), JSON.stringify(manifest, null, 2) + '\n');

const required = [
  ...manifest.esmodules,
  ...manifest.styles,
  ...manifest.languages.map((l) => l.path),
  'workers/engine-worker.js',
  'wasm/pf2e_stockfish_bg.wasm',
  'templates/best-turn.hbs',
  'templates/gm-assistant.hbs',
  'templates/forecast.hbs',
  'templates/explain.hbs',
];
for (const f of required) {
  const p = join(out, f);
  if (!existsSync(p)) throw new Error(`missing ${f} in build output`);
  console.log(`${f.padEnd(36)} ${(statSync(p).size / 1024).toFixed(1).padStart(8)} KiB`);
}
