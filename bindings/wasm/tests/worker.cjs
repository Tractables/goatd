const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const { test } = require('node:test');
const root = path.join(__dirname, '..');
function worker(solver) {
  const messages = [];
  const context = vm.createContext({
    location: { search: '' }, performance, TextEncoder,
    createGoatd: () => Promise.resolve(solver),
    postMessage: message => messages.push(message),
  });
  context.importScripts = (...files) => {
    for (const file of files) if (file !== 'goatd.js') vm.runInContext(fs.readFileSync(path.join(root, file), 'utf8'), context);
  };
  vm.runInContext(fs.readFileSync(path.join(root, 'worker.js'), 'utf8'), context);
  return { context, messages };
}
test('graph input uses the heap and both allocations are freed', async () => {
  const freed = [];
  const solver = {
    HEAPU8: new Uint8Array(10 * 1024 * 1024),
    _malloc: () => 16, _free: ptr => freed.push(ptr),
    _goatd_decompose: ptr => { assert.equal(ptr, 16); return 8; },
    _goatd_string_free: ptr => freed.push(ptr),
    UTF8ToString: () => 's td 1 1 1',
    ccall: (_name, _result, types) => assert.equal(types[0], 'number', 'graph must not be copied onto the Wasm stack'),
  };
  const { context, messages } = worker(solver);
  await new Promise(resolve => setImmediate(resolve));
  context.onmessage({ data: { graph: 'c'.repeat(9 * 1024 * 1024), order: 0, seed: 0, budget: 0 } });
  assert.equal(messages.at(-1).td, 's td 1 1 1', JSON.stringify(messages));
  assert.deepEqual(freed.sort((a,b) => a-b), [8,16]);
});
test('synchronous module-load failures are reported to the page', async () => {
  const messages = [];
  const context = vm.createContext({ location: { search: '' }, postMessage: m => messages.push(m),
    importScripts: () => { throw new Error('module unavailable'); } });
  vm.runInContext(fs.readFileSync(path.join(root, 'worker.js'), 'utf8'), context);
  await new Promise(resolve => setImmediate(resolve));
  assert.match(messages[0].loadError, /module unavailable/);
});

test('asynchronous module-load failures are reported to the page', async () => {
  const messages = [];
  const context = vm.createContext({ location: { search: '' }, postMessage: m => messages.push(m),
    importScripts: () => {}, createGoatd: () => Promise.reject(new Error('wasm unavailable')) });
  vm.runInContext(fs.readFileSync(path.join(root, 'worker.js'), 'utf8'), context);
  await new Promise(resolve => setImmediate(resolve));
  assert.match(messages[0].loadError, /wasm unavailable/);
});

test('a readback failure still frees input and output', async () => {
  const freed = [];
  const { context, messages } = worker({ HEAPU8: new Uint8Array(128),
    _malloc: () => 16, _free: ptr => freed.push(ptr),
    _goatd_decompose: () => 8, _goatd_string_free: ptr => freed.push(ptr),
    UTF8ToString: () => { throw new Error('readback failed'); } });
  await new Promise(resolve => setImmediate(resolve));
  context.onmessage({ data: { graph: 'p tw 0 0', order: 0, seed: 0, budget: 0 } });
  assert.match(messages.at(-1).error, /readback failed/);
  assert.deepEqual(freed.sort((a,b) => a-b), [8,16]);
});
