import { test } from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { buildBundle } from './build-bundle.mjs';
async function fixture(fn) {
  const root = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), 'rx-ui-bundle-')));
  try {
    await fs.mkdir(path.join(root, 'assets'));
    await fs.writeFile(path.join(root, 'index.html'), '<html lang="en"><body>RX</body></html>');
    await fs.writeFile(path.join(root, 'assets/app.js'), 'console.log("RX");');
    await fn(root);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
}
test('manifest is reproducible, ordered, exact-size and excludes itself', async () => fixture(async root => {
  const first = await buildBundle(root);
  assert.deepEqual(await buildBundle(root), first);
  const manifest = JSON.parse(await fs.readFile(path.join(root, 'operator-bundle.json')));
  assert.equal(manifest.schema, 'rx.operator-ui-bundle.v1');
  assert.deepEqual(manifest.files.map(f => f.path), ['assets/app.js', 'index.html']);
  assert.equal(manifest.files[0].size_bytes, String(Buffer.byteLength('console.log("RX");')));
  await fs.appendFile(path.join(root, 'assets/app.js'), '\n');
  assert.notEqual((await buildBundle(root)).sha256, first.sha256);
}));
test('unlisted file types, reserved names and symlinks cannot enter a bundle', async () => fixture(async root => {
  await fs.writeFile(path.join(root, 'unexpected.txt'), 'extra');
  await assert.rejects(buildBundle(root), /unexpected bundle file/);
  await fs.rm(path.join(root, 'unexpected.txt'));
  await fs.symlink(path.join(root, 'index.html'), path.join(root, 'assets/linked.html'));
  await assert.rejects(buildBundle(root), /symlink/);
  await fs.rm(path.join(root, 'assets/linked.html'));
  await fs.writeFile(path.join(root, 'assets/CON.js'), 'reserved');
  await assert.rejects(buildBundle(root), /unsupported bundle path/);
}));
test('missing entry point and oversized files fail instead of producing a partial manifest', async () => fixture(async root => {
  await fs.rm(path.join(root, 'index.html'));
  await assert.rejects(buildBundle(root), /index.html missing/);
  await fs.writeFile(path.join(root, 'index.html'), '<html></html>');
  await fs.writeFile(path.join(root, 'assets/large.js'), Buffer.alloc(8 * 1024 * 1024 + 1));
  await assert.rejects(buildBundle(root), /file size invalid/);
  await assert.rejects(fs.stat(path.join(root, 'operator-bundle.json')), {code:'ENOENT'});
}));
