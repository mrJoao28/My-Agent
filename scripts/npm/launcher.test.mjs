import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, chmodSync, symlinkSync, lstatSync, rmSync, realpathSync } from 'node:fs';
import { join, resolve, dirname, basename } from 'node:path';
import { tmpdir } from 'node:os';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { runInNewContext } from 'node:vm';
import { root, validate, json, platforms } from './common.mjs';
import { release } from './publish.mjs';
import { checkBinary } from './binary.mjs';

const shim = join(root, 'npm/myagent/bin/myagent.js');
const owned = [];
function temporary(prefix) {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  owned.push({ dir, prefix });
  return dir;
}
after(() => {
  for (const { dir, prefix } of owned) {
    assert.equal(dirname(resolve(dir)), resolve(tmpdir()));
    assert.ok(basename(dir).startsWith(prefix));
    assert.ok(lstatSync(dir).isDirectory() && !lstatSync(dir).isSymbolicLink());
    rmSync(dir, { recursive: true, maxRetries: 5, retryDelay: 100 });
  }
});
function fixture(binary = true, version = validate()) {
  const dir = temporary('myagent launcher test ');
  mkdirSync(join(dir, 'bin'));
  copyFileSync(shim, join(dir, 'bin/myagent.js'));
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ version: validate() }));
  const native = join(dir, 'node_modules', `@j2008/myagent-${process.platform}-${process.arch}`);
  mkdirSync(join(native, 'bin'), { recursive: true });
  writeFileSync(join(native, 'package.json'), JSON.stringify({ name: `@j2008/myagent-${process.platform}-${process.arch}`, version }));
  if (binary) {
    const path = join(native, 'bin', process.platform === 'win32' ? 'myagent.exe' : 'myagent');
    // Node is only a deterministic child fixture here, never a release binary.
    if (process.platform === 'win32') copyFileSync(process.execPath, path);
    else symlinkSync(process.execPath, path);
    chmodSync(path, 0o755);
  }
  return { dir, entry: join(dir, 'bin/myagent.js') };
}
test('versions and exact optional dependency set match Rust and Cargo.lock', () => {
  assert.equal(validate(`v${validate()}`), validate());
  assert.throws(() => validate('v999.0.0'), /tag/);
});
test('version update keeps Rust, lockfile, six manifests and exact dependencies synchronized', () => {
  const dir = temporary('myagent version fixture ');
  mkdirSync(join(dir, 'scripts/npm'), { recursive: true });
  for (const name of ['common.mjs', 'version.mjs']) copyFileSync(join(root, 'scripts/npm', name), join(dir, 'scripts/npm', name));
  for (const name of ['Cargo.toml', 'Cargo.lock']) copyFileSync(join(root, name), join(dir, name));
  for (const part of ['myagent', ...platforms.map(p => `platforms/${p.key}`)]) {
    const dest = join(dir, 'npm', part);
    mkdirSync(dest, { recursive: true });
    copyFileSync(join(root, 'npm', part, 'package.json'), join(dest, 'package.json'));
  }
  mkdirSync(join(dir, 'npm/myagent/bin'));
  copyFileSync(shim, join(dir, 'npm/myagent/bin/myagent.js'));
  const result = spawnSync(process.execPath, [join(dir, 'scripts/npm/version.mjs'), '--set', '9.8.7'], { encoding: 'utf8', timeout: 15000 });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(validate('v9.8.7', dir), '9.8.7');
  const main = join(dir, 'npm/myagent/package.json');
  const changed = json(main);
  changed.optionalDependencies['@j2008/myagent-win32-x64'] = '^9.8.7';
  writeFileSync(main, JSON.stringify(changed));
  assert.throws(() => validate(undefined, dir));
});
test('launcher preserves argv, cwd, env, stdin/stdout/stderr and exit codes without a shell', () => {
  const f = fixture();
  for (const code of [0, 1, 2, 3, 130]) {
    const args = ['space value', '"quotes"', '$HOME; & | >', 'ação/文件'];
    const js = `const fs=require('node:fs'); console.log(JSON.stringify({args:process.argv.slice(1),cwd:process.cwd(),env:process.env.MYAGENT_TEST_SENTINEL,input:fs.readFileSync(0,'utf8'),tty:!!process.stdin.isTTY})); console.error('child stderr'); process.exitCode=${code}`;
    const result = spawnSync(process.execPath, [f.entry, '-e', js, ...args], { cwd: f.dir, env: { ...process.env, MYAGENT_TEST_SENTINEL: 'fictitious' }, input: 'stdin fixture', encoding: 'utf8', timeout: 15000, shell: false });
    assert.equal(result.status, code, result.stderr);
    const found = JSON.parse(result.stdout);
    assert.deepEqual(found.args, args);
    assert.equal(realpathSync(found.cwd), realpathSync(f.dir));
    assert.equal(found.env, 'fictitious');
    assert.equal(found.input, 'stdin fixture');
    assert.equal(found.tty, false);
    assert.equal(result.stderr.trim(), 'child stderr');
  }
});
test('missing, incompatible and unexecutable packages fail clearly', () => {
  for (const f of [fixture(false), fixture(true, '999.0.0')]) {
    const result = spawnSync(process.execPath, [f.entry, '--version'], { encoding: 'utf8', timeout: 15000 });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /My-Agent:/);
    assert.match(result.stderr, /Reinstale/);
  }
});
test('unsupported architecture reports a useful error and never spawns', () => {
  const state = { platform: 'freebsd', arch: 'riscv64', argv: ['node', 'myagent'], exitCode: 0 };
  let error = '';
  runInNewContext(readFileSync(shim, 'utf8'), { process: state, console: { error: value => { error = value; } }, require: name => {
    if (name === '../package.json') return { version: validate() };
    if (name === 'node:child_process') return { spawn: () => assert.fail('must not spawn') };
    if (name === 'node:os') return { constants: { signals: {} } };
    if (name === 'node:path') return {};
    if (name === 'node:fs') return {};
    assert.fail(name);
  } });
  assert.equal(state.exitCode, 1);
  assert.match(error, /freebsd-riscv64/);
});
test('POSIX parent-directed signals wait for child cleanup and preserve 130', { skip: process.platform === 'win32' }, async () => {
  const f = fixture();
  const child = spawn(process.execPath, [f.entry, '-e', "process.on('SIGTERM',()=>{console.log('cleanup');process.exit(130)});console.log('ready');setInterval(()=>{},1000)"], { stdio: ['ignore', 'pipe', 'pipe'] });
  const timeout = setTimeout(() => child.kill('SIGKILL'), 10000);
  let output = '';
  child.stdout.on('data', chunk => { output += chunk; if (output === 'ready\n') child.kill('SIGTERM'); });
  try { const [code, signal] = await once(child, 'exit'); assert.equal(code, 130); assert.equal(signal, null); assert.match(output, /cleanup/); }
  finally { clearTimeout(timeout); }
});
test('POSIX child signal termination reaches the launcher caller', { skip: process.platform === 'win32' }, () => {
  const f = fixture();
  const result = spawnSync(process.execPath, [f.entry, '-e', "process.kill(process.pid,'SIGTERM')"], { timeout: 15000 });
  assert.equal(result.signal, 'SIGTERM');
});
test('binary validation rejects wrong architecture and dynamic Linux interpreters', () => {
  assert.throws(() => checkBinary('win32-x64', Buffer.alloc(64)));
  const elf = Buffer.alloc(120); Buffer.from([127, 69, 76, 70, 2, 1]).copy(elf);
  elf.writeUInt16LE(62, 18); elf.writeBigUInt64LE(64n, 32); elf.writeUInt16LE(56, 54); elf.writeUInt16LE(1, 56);
  checkBinary('linux-x64', elf);
  assert.throws(() => checkBinary('linux-arm64', elf));
  elf.writeUInt32LE(3, 64);
  assert.throws(() => checkBinary('linux-x64', elf), /PT_INTERP/);
});
const items = () => [...platforms.map(p => p.key), 'main'].map(key => ({ key, manifest: { name: key, version: '0.1.0' }, integrity: `sha512-${key}` }));
test('publisher confirms all platform versions before the main and resumes partial releases', async () => {
  const values = new Map([['linux-x64', 'sha512-linux-x64']]);
  const calls = [];
  await release(items(), { lookup: async item => values.get(item.key) ?? null, publish: async item => {
    if (item.key === 'main') for (const p of platforms) assert.equal(values.get(p.key), `sha512-${p.key}`);
    calls.push(item.key); values.set(item.key, item.integrity);
  } });
  assert.equal(calls.at(-1), 'main');
  assert.ok(!calls.includes('linux-x64'));
  await release(items(), { lookup: async item => values.get(item.key), publish: async () => assert.fail('must not overwrite') });
});
test('publisher rejects missing tarballs, integrity conflicts and invisible platform versions', async () => {
  let writes = 0;
  const publish = async () => { writes++; };
  await assert.rejects(release(items().slice(1), { lookup: async () => null, publish }));
  await assert.rejects(release(items(), { lookup: async () => 'sha512-other', publish }), /different integrity/);
  assert.equal(writes, 0);
  await assert.rejects(release(items(), { lookup: async () => null, publish, attempts: 1 }), /main package remains blocked/);
  assert.equal(writes, 1);
});
