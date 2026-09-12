import { createHash, randomUUID } from 'node:crypto';
import { promises as fs, constants } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const manifestName = 'operator-bundle.json';
const allowed = new Set(['js', 'css', 'woff2', 'woff', 'png', 'jpg', 'jpeg', 'svg', 'ico', 'webp', 'json']);
export async function buildBundle(directory) {
  const root = path.resolve(directory);
  if (await fs.realpath(root) !== root || !(await fs.lstat(root)).isDirectory()) {
    throw new Error('bundle root must be a real directory without symlink components');
  }
  const files = [];
  const aliases = new Set();
  let total = 0;
  let entries = 0;
  async function visit(folder, prefix = '') {
    for await (const entry of await fs.opendir(folder)) {
      const item = entry.name;
      if (++entries > 2048 || prefix.split("/").length > 16) throw new Error("bundle directory limit");
      const name = prefix + item;
      if (name === manifestName) continue;
      if (name.length > 240 || !/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(name) || name.split('/').some(p =>
          p.length > 100 || p.endsWith('.') || p === '.' || p === '..' || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(p))) {
        throw new Error('unsupported bundle path: ' + name);
      }
      const file = path.join(folder, item);
      const stat = await fs.lstat(file);
      if (stat.isSymbolicLink()) throw new Error('bundle symlink refused: ' + name);
      if (stat.isDirectory()) {
        if (name !== 'assets' && !name.startsWith('assets/')) throw new Error('unexpected bundle directory: ' + name);
        await visit(file, name + '/');
        continue;
      }
      if (!stat.isFile() || (name !== 'index.html' && (!name.startsWith('assets/') || !allowed.has(name.split('.').at(-1))))) {
        throw new Error('unexpected bundle file: ' + name);
      }
      const alias = name.toLowerCase();
      if (aliases.has(alias)) throw new Error('case-aliased bundle paths');
      aliases.add(alias);
      if (stat.size === 0 || stat.size > 8 * 1024 * 1024) throw new Error('bundle file size invalid');
      const handle = await fs.open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0) | (constants.O_NONBLOCK ?? 0));
      let data;
      try {
        const opened = await handle.stat();
        if (!opened.isFile() || opened.size !== stat.size) throw new Error('bundle file changed during acquisition');
        const buffer = Buffer.alloc(opened.size + 1);
        let length = 0;
        while (length < buffer.length) {
          const read = await handle.read(buffer, length, buffer.length - length, null);
          if (read.bytesRead === 0) break;
          length += read.bytesRead;
        }
        if (length !== opened.size) throw new Error('bundle file changed during acquisition');
        data = buffer.subarray(0, length);
      } finally { await handle.close(); }
      total += data.length;
      if (files.length >= 1024 || total > 64 * 1024 * 1024) throw new Error('bundle size limit');
      files.push({ path: name, sha256: createHash('sha256').update(data).digest('hex'), size_bytes: String(data.length) });
    }
  }
  await visit(root);
  files.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  if (!files.some(f => f.path === 'index.html')) throw new Error('index.html missing');
  const manifest = { schema: 'rx.operator-ui-bundle.v1', api_schema: 'rx.operator-api.v1', files };
  const bytes = Buffer.from(JSON.stringify(manifest) + '\n');
  const destination = path.join(root, manifestName);
  try {
    if ((await fs.lstat(destination)).isSymbolicLink()) throw new Error('manifest symlink refused');
  } catch (e) { if (e.code !== 'ENOENT') throw e; }
  const temporary = path.join(root, `.operator-bundle-${randomUUID()}.tmp`);
  let owned = false;
  try {
    await fs.writeFile(temporary, bytes, { flag: 'wx' });
    owned = true;
    await fs.rename(temporary, destination);
    owned = false;
  } finally { if (owned) await fs.rm(temporary, { force: true }); }
  return { manifest: manifestName, sha256: createHash('sha256').update(bytes).digest('hex'), files: files.length, content_bytes: total };
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 3) throw new Error('usage: node build-bundle.mjs DIST_DIRECTORY');
  console.log(JSON.stringify(await buildBundle(process.argv[2])));
}
