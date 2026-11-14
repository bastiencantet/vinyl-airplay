// Build exactly the host-native binaries which Tauri will embed in the app.
import { execFileSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import './check-desktop-version.mjs';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const profile = process.argv.includes('--debug') ? 'debug' : 'release';
const host = /^host: (.+)$/m.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))[1];
const target = resolve(root, process.env.CARGO_TARGET_DIR || 'target');
const env = { ...process.env, CARGO_TARGET_DIR: target };
const build = ['build', '--locked', ...(profile === 'release' ? ['--release'] : [])];
for (const [name, args] of [
  ['vinyl-airplay', ['--bin', 'vinyl-airplay']],
  ['vinyl-shazam-fingerprint', ['--manifest-path', 'tools/shazam-fingerprint/Cargo.toml']],
]) {
  execFileSync('cargo', [...build, ...args], { cwd: root, env, stdio: 'inherit' });
  const suffix = process.platform === 'win32' ? '.exe' : '';
  mkdirSync(join(root, 'src-tauri/binaries'), { recursive: true });
  copyFileSync(join(target, profile, name + suffix), join(root, 'src-tauri/binaries', `${name}-${host}${suffix}`));
}
console.log(`Prepared ${host} sidecars (${profile}).`);
