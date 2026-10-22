import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync, realpathSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
// Node resolves module URLs through symlinks; argv may retain /var or an alias.
export const isMain = url => Boolean(process.argv[1] && realpathSync(resolve(process.argv[1])) === fileURLToPath(url));
export const platforms = [
  { key: 'win32-x64', target: 'x86_64-pc-windows-msvc', runner: 'windows-2022' },
  { key: 'linux-x64', target: 'x86_64-unknown-linux-musl', runner: 'ubuntu-24.04' },
  { key: 'linux-arm64', target: 'aarch64-unknown-linux-musl', runner: 'ubuntu-24.04-arm' },
  { key: 'darwin-arm64', target: 'aarch64-apple-darwin', runner: 'macos-14' },
  { key: 'darwin-x64', target: 'x86_64-apple-darwin', runner: 'macos-15-intel' },
];
export const json = path => JSON.parse(readFileSync(path, 'utf8'));
export const sha = (bytes, algorithm = 'sha256') => createHash(algorithm).update(bytes).digest('hex');
export const integrity = bytes => `sha512-${createHash('sha512').update(bytes).digest('base64')}`;
export const manifestPath = key => join(root, key === 'main' ? 'npm/myagent/package.json' : `npm/platforms/${key}/package.json`);
export const binaryName = key => key.startsWith('win32-') ? 'myagent.exe' : 'myagent';
export function validate(tag = undefined, base = root) {
  const cargo = readFileSync(join(base, 'Cargo.toml'), 'utf8');
  const version = cargo.match(/\[workspace.package\][\s\S]*?^version = "([^"]+)"/m)?.[1];
  assert.match(version ?? '', /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/, 'release requires a stable SemVer');
  if (tag !== undefined) assert.equal(tag, `v${version}`, 'tag must exactly match the Rust version');
  const lock = readFileSync(join(base, 'Cargo.lock'), 'utf8');
  const locked = [...lock.matchAll(/\[\[package\]\]\r?\nname = "(myagent-[^"]+)"\r?\nversion = "([^"]+)"/g)];
  assert.equal(locked.length, 9, 'all workspace packages must be locked');
  for (const [, name, found] of locked) assert.equal(found, version, `${name} lock version`);
  const main = json(join(base, 'npm/myagent/package.json'));
  assert.equal(main.name, '@j2008/myagent');
  assert.equal(main.version, version);
  assert.deepEqual(main.bin, { myagent: 'bin/myagent.js' });
  assert.equal(main.engines.node, '>=22.14.0');
  assert.deepEqual(main.files, ['bin/myagent.js']);
  assert.deepEqual(main.optionalDependencies, Object.fromEntries(platforms.map(p => [`@j2008/myagent-${p.key}`, version])));
  for (const [key, manifest] of [['main', main], ...platforms.map(p => [p.key, json(join(base, `npm/platforms/${p.key}/package.json`))])]) {
    assert.equal(manifest.version, version);
    assert.equal(manifest.license, 'MIT');
    assert.equal(manifest.repository.url, 'git+https://github.com/mrJoao28/My-Agent.git');
    assert.deepEqual(manifest.publishConfig, { access: 'public', registry: 'https://registry.npmjs.org/' });
    for (const field of ['scripts', 'dependencies', 'devDependencies', 'bundledDependencies']) assert.equal(manifest[field], undefined, `${key}: forbidden field ${field}`);
    if (key !== 'main') {
      assert.equal(manifest.name, `@j2008/myagent-${key}`);
      assert.deepEqual(manifest.os, [key.split('-')[0]]);
      assert.deepEqual(manifest.cpu, [key.split('-')[1]]);
      assert.deepEqual(manifest.files, [`bin/${binaryName(key)}`]);
      assert.equal(manifest.optionalDependencies, undefined);
    }
  }
  const shim = readFileSync(join(base, 'npm/myagent/bin/myagent.js'), 'utf8');
  assert.ok(shim.startsWith('#!/usr/bin/env node\n'), 'shim shebang must use LF');
  return version;
}
export function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 180000, ...options, shell: false });
  if (result.error || result.status !== 0) throw new Error(`${command} failed (${result.status}): ${result.error?.message ?? result.stderr ?? ''}`);
  return result.stdout;
}
export function npmCli() {
  const candidates = [process.env.npm_execpath];
  for (const dir of (process.env.PATH ?? '').split(process.platform === 'win32' ? ';' : ':')) {
    const executable = join(dir, 'npm');
    if (existsSync(executable)) candidates.push(realpathSync(executable));
    candidates.push(join(dir, 'node_modules/npm/bin/npm-cli.js'), join(dir, '../lib/node_modules/npm/bin/npm-cli.js'));
  }
  candidates.push(join(dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js'), join(dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js'));
  const found = candidates.find(p => p && existsSync(p) && /npm-cli\.js$/.test(p));
  assert.ok(found, 'npm CLI not found: run scripts through Node installed alongside npm');
  return resolve(found);
}
export const npm = (args, options) => run(process.execPath, [npmCli(), ...args], options);
export function tarballs(directory) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...tarballs(path));
    else if (entry.name.endsWith('.tgz')) found.push(path);
  }
  return found;
}
