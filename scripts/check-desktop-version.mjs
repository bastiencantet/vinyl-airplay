// Fail before compiling if installers and app report different versions.
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = path => readFileSync(resolve(root, path), 'utf8');
const version = JSON.parse(read('package.json')).version;
assert.equal(JSON.parse(read('src-tauri/tauri.conf.json')).version, version);
for (const path of ['Cargo.toml', 'src-tauri/Cargo.toml']) {
  assert.equal(/^version\s*=\s*"([^"]+)"/m.exec(read(path))?.[1], version, `${path}: version mismatch`);
}
console.log(`Desktop, engine and installer versions agree: ${version}`);
