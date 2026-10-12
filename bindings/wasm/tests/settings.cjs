const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const { test } = require('node:test');
const source = fs.readFileSync(path.join(__dirname, '../app.js'), 'utf8');
const declaration = source.match(/  const setting = .*;/)[0];
let value;
const context = vm.createContext({ element: () => ({ value }) });
vm.runInContext(declaration + '\nglobalThis.readSetting = setting;', context);
const runtime = path.join(__dirname, '../runtime.js');
if (fs.existsSync(runtime)) vm.runInContext(fs.readFileSync(runtime, 'utf8'), context);
test('unsigned settings reject fractions, overflow and nonfinite values', () => {
  for (value of ['-1', '1.5', '4294967296', 'Infinity', 'NaN']) {
    assert.throws(() => context.readSetting('budget'), /budget/);
  }
});
test('unsigned settings retain both endpoints and treat blank as zero', () => {
  for (value of ['', '0', '4294967295']) assert.equal(context.readSetting('seed'), Number(value));
});
