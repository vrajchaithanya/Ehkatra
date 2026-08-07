// Runs a wasm32-wasip1 binary under Node's WASI so the DP-A2 differential-replay
// gate is one command locally and in CI (docs/07: every recurring human step is a
// script or a CI gate).  Usage: node --no-warnings tools/run-wasi.mjs <file.wasm>
import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

const wasi = new WASI({ version: 'preview1', args: ['replay-check'] });
const wasm = await WebAssembly.compile(await readFile(process.argv[2]));
const instance = await WebAssembly.instantiate(wasm, wasi.getImportObject());
wasi.start(instance);
