const assert = require('node:assert/strict');
const path = require('node:path');
const runtime = require('../runtime.js');
const createGoatd = require(path.resolve(process.argv[2]));
(async () => {
  const solver = await createGoatd();
  const graph = 'p tw 4 5\n1 2\n2 3\n3 4\n4 1\n1 3\n';
  // Exceeds the module stack while retaining a small graph.
  for (const input of ['c ' + 'x'.repeat(9 * 1024 * 1024) + '\n' + graph, graph]) {
    const td = runtime.decompose(solver, input, 0, 0, 0);
    const header = td.split('\n', 1)[0].split(' ');
    assert.equal(header[0], 's');
    assert.equal(header[1], 'td');
    assert.equal(Number(header[3]) - 1, 2);
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
