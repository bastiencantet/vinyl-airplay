// Collect only distributable installers; hash bytes that users will download.
import { createReadStream } from 'node:fs';
import { mkdir, readdir, copyFile, writeFile, readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { resolve, dirname, join, extname, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const platform = process.argv[2];
const expected = {
  'linux-x64': { host: 'x86_64-unknown-linux-gnu', extensions: ['.deb', '.AppImage'] },
  'windows-x64': { host: 'x86_64-pc-windows-msvc', extensions: ['.exe'] },
  'macos-arm64': { host: 'aarch64-apple-darwin', extensions: ['.dmg'] },
  'macos-x64': { host: 'x86_64-apple-darwin', extensions: ['.dmg'] },
}[platform];
assert.ok(expected, 'Pass a supported platform name');
const host = /^host: (.+)$/m.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))[1];
assert.equal(host, expected.host, 'Build must run on the selected native architecture');
const version = JSON.parse(await readFile(join(root, 'package.json'), 'utf8')).version;
const target = resolve(root, process.env.CARGO_TARGET_DIR || 'src-tauri/target');
const bundle = join(target, 'release/bundle');
const output = join(root, 'dist/desktop');
await mkdir(output, { recursive: true });
async function* files(path) {
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const full = join(path, entry.name);
    if (entry.isDirectory() && !entry.name.endsWith('.app')) yield* files(full);
    else if (entry.isFile()) yield full;
  }
}
const artifacts = [];
for await (const path of files(bundle)) {
  if (!expected.extensions.includes(extname(path))) continue;
  const name = `vinyl-airplay-${version}-${platform}-${basename(path).replaceAll(' ', '-')}`;
  const destination = join(output, name);
  await copyFile(path, destination);
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(destination)) hash.update(chunk);
  artifacts.push({ name, sha256: hash.digest('hex') });
}
for (const extension of expected.extensions) {
  assert.ok(artifacts.some(file => file.name.endsWith(extension)), `Missing ${extension} installer`);
}
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
await writeFile(join(output, 'SHA256SUMS.txt'), artifacts.map(a => `${a.sha256}  ${a.name}\n`).join(''));
await writeFile(join(output, 'build.json'), JSON.stringify({
  version, platform, host, commit, channel: 'alpha', signed: false, artifacts,
}, null, 2) + '\n');
for (const name of ['LICENSE', 'NOTICE']) await copyFile(join(root, name), join(output, name));
await copyFile(join(root, 'docs/DESKTOP.md'), join(output, 'DESKTOP.md'));
console.log(`Collected ${artifacts.length} ${platform} installers from ${commit}`);
