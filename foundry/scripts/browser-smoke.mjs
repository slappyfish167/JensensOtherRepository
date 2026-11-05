// End-to-end smoke test of the *built* module in a real browser:
// serves dist/ under /modules/pf2e-stockfish/ (as Foundry does), starts the
// bundled module worker, loads the WASM engine, and runs every analysis on
// the shared fixture. Requires Playwright + Chromium.
//   npm run build && node scripts/browser-smoke.mjs
import { createServer } from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { join, extname } from 'node:path';
import { createRequire } from 'node:module';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'dist');
const fixture = readFileSync(join(root, 'test/fixtures/crypt-ambush.snapshot.json'), 'utf8');
const types = { '.js': 'text/javascript', '.wasm': 'application/wasm', '.html': 'text/html', '.json': 'application/json' };

const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  if (url.pathname === '/') {
    res.writeHead(200, { 'content-type': 'text/html' });
    return res.end('<!doctype html><title>smoke</title>');
  }
  const m = url.pathname.match(/^\/modules\/(pf2e-stockfish\/.*)$/);
  const file = m ? join(dist, m[1]) : null;
  if (!file || !existsSync(file)) {
    res.writeHead(404);
    return res.end();
  }
  res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' });
  res.end(readFileSync(file));
});
await new Promise((r) => server.listen(0, r));
const port = server.address().port;

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const browser = await chromium.launch();
const page = await browser.newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
await page.goto(`http://127.0.0.1:${port}/`);
const result = await page.evaluate(async (snapshot) => {
  const w = new Worker('/modules/pf2e-stockfish/workers/engine-worker.js', { type: 'module' });
  let next = 1;
  const pending = new Map();
  const hello = new Promise((resolve, reject) => {
    w.onmessage = (ev) => {
      if (ev.data.hello) return ev.data.failed ? reject(new Error(ev.data.failed)) : resolve(ev.data.version);
      const p = pending.get(ev.data.id);
      pending.delete(ev.data.id);
      ev.data.ok ? p.resolve(ev.data.result) : p.reject(new Error(ev.data.error));
    };
    w.onerror = (e) => reject(new Error(e.message));
  });
  const call = (op) => new Promise((resolve, reject) => {
    const id = next++;
    pending.set(id, { resolve, reject });
    w.postMessage({ id, ...op });
  });
  const t = async (f) => { const s = performance.now(); const r = await f(); return [r, performance.now() - s]; };
  const version = await hello;
  const [load] = await t(() => call({ op: 'load', snapshot }));
  const summary = JSON.parse(load);
  const [ev, evMs] = await t(() => call({ op: 'evaluate' }));
  const [turn, turnMs] = await t(() => call({ op: 'bestTurn', token: summary.current_token, options: '', deadlineMs: 100 }));
  const [plan, planMs] = await t(() => call({ op: 'sidePlan', side: '"enemy"', options: '', deadlineMs: 400 }));
  const [tally, fcMs] = await t(() => call({ op: 'forecastSlice', options: JSON.stringify({ runs: 200, seed: 1 }) }));
  const forecast = JSON.parse(await call({ op: 'mergeForecast', tallies: `[${tally}]` }));
  let errorCode = null;
  try { await call({ op: 'bestTurn', token: 'missing', options: '', deadlineMs: 0 }); } catch (e) { errorCode = JSON.parse(e.message).code; }
  return {
    version,
    score: JSON.parse(ev).score,
    evMs,
    turn: JSON.parse(turn).summary,
    turnMs,
    creatures: JSON.parse(plan).creatures.length,
    planMs,
    victory: forecast.victory,
    fcMs,
    errorCode,
  };
}, fixture);
await browser.close();
server.close();
console.log(JSON.stringify(result, null, 2));
if (errors.length) throw new Error(errors.join('\n'));
const fail = (m) => { console.error(`FAIL: ${m}`); process.exit(1); };
if (!result.version.endsWith('snapshot.1')) fail('version');
if (result.creatures !== 3) fail('side plan');
if (result.errorCode !== 'unknown_token') fail('error mapping');
if (result.turnMs > 1000) fail(`best turn took ${result.turnMs} ms`);
console.log('browser smoke test passed');
