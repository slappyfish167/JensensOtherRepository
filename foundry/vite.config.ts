import { defineConfig } from 'vite';

// Two ES-module entry points:
//   scripts/main.js           — loaded by Foundry (module.json "esmodules")
//   workers/engine-worker.js  — the analysis worker (loads the WASM engine)
// The WASM file referenced by the wasm-bindgen glue is emitted to wasm/.
export default defineConfig({
  // Relative asset URLs: the module is served from modules/pf2e-stockfish/
  // (plus any Foundry route prefix), never from the server root.
  base: './',
  build: {
    outDir: 'dist/pf2e-stockfish',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: true,
    minify: true,
    assetsInlineLimit: 0,
    rollupOptions: {
      input: {
        main: 'src/main.ts',
        'engine-worker': 'src/engine/worker.ts',
      },
      preserveEntrySignatures: 'strict',
      output: {
        format: 'es',
        entryFileNames: (chunk) => (chunk.name === 'engine-worker' ? 'workers/engine-worker.js' : 'scripts/[name].js'),
        chunkFileNames: 'scripts/chunks/[name]-[hash].js',
        assetFileNames: (asset) => ((asset.names?.[0] ?? asset.name ?? '').endsWith('.wasm') ? 'wasm/[name][extname]' : 'assets/[name][extname]'),
      },
    },
  },
  test: {
    include: ['test/**/*.test.ts'],
    environment: 'node',
  },
});
