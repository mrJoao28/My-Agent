import assert from 'node:assert/strict';
import { execFile, spawnSync } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtempSync, mkdirSync, copyFileSync, chmodSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { npmCli, validate, tarballs, json } from './common.mjs';
import { inspect } from './archive.mjs';
import { registry } from './registry.mjs';

const execute = promisify(execFile);
export async function smoke(directory, report) {
  const version = validate();
  const key = `${process.platform}-${process.arch}`;
  const packages = tarballs(directory).map(inspect);
  assert.equal(new Set(packages.map(p => p.key)).size, packages.length, 'duplicate packages');
  const main = packages.find(p => p.key === 'main');
  const native = packages.find(p => p.key === key);
  assert.ok(main && native, `main and ${key} tarballs required`);
  const temp = mkdtempSync(join(tmpdir(), 'myagent npm smoke '));
  const prefix = join(temp, 'global prefix with spaces');
  const cwd = join(temp, 'workspace with spaces');
  const runtime = join(temp, 'runtime without rust');
  for (const path of [prefix, cwd, runtime]) mkdirSync(path);
  const node = join(runtime, process.platform === 'win32' ? 'node.exe' : 'node');
  copyFileSync(process.execPath, node);
  chmodSync(node, 0o755);
  const config = join(temp, 'empty npm user config');
  const globalConfig = join(temp, 'empty npm global config');
  writeFileSync(config, '');
  writeFileSync(globalConfig, '');
  const env = {};
  // A fresh npm config/cache and a minimal environment prevent user credentials,
  // custom registries, provider keys, Rust toolchains or project hooks leaking in.
  for (const name of ['SystemRoot', 'SYSTEMROOT', 'WINDIR', 'ComSpec', 'COMSPEC', 'TEMP', 'TMP', 'TMPDIR']) {
    if (process.env[name]) env[name] = process.env[name];
  }
  Object.assign(env, {
    PATH: runtime,
    HOME: temp, USERPROFILE: temp, APPDATA: join(temp, 'appdata'), LOCALAPPDATA: join(temp, 'localappdata'),
    MYAGENT_HOME: join(temp, 'isolated state'), MYAGENT_ENABLE_CHATGPT: '0',
    NPM_CONFIG_USERCONFIG: config, NPM_CONFIG_GLOBALCONFIG: globalConfig,
    NPM_CONFIG_CACHE: join(temp, 'empty npm cache'),
    NPM_CONFIG_AUDIT: 'false', NPM_CONFIG_FUND: 'false',
  });
  if (process.platform === 'win32') env.PATH += `;${join(process.env.SystemRoot, 'System32')}`;
  for (const name of ['rustc', 'cargo', 'bun']) {
    assert.equal(spawnSync(name, ['--version'], { env, shell: false }).error?.code, 'ENOENT', `${name} must be unavailable`);
  }
  const local = await registry(packages);
  const options = { cwd, env, timeout: 120000, maxBuffer: 4 * 1024 * 1024, windowsHide: true };
  try {
    // Install by name/version into an empty cache. The real main tarball keeps its
    // exact optionalDependencies; unpublished native packages resolve over HTTP.
    await execute(node, [npmCli(), 'install', '--global', '--prefix', prefix,
      '--registry', local.origin, '--ignore-scripts', '--include=optional',
      '--package-lock=false', '--no-audit', '--no-fund', '--fetch-retries=0', `@j2008/myagent@${version}`], options);
    assert.ok(local.hits.has('tar:main') && local.hits.has(`tar:${key}`), 'npm must fetch both actual tarballs');
    for (const platform of packages.filter(p => p.key !== 'main' && p.key !== key)) {
      assert.ok(!local.hits.has(`tar:${platform.key}`), 'npm must filter incompatible os/cpu');
    }
    const modules = process.platform === 'win32' ? join(prefix, 'node_modules') : join(prefix, 'lib/node_modules');
    const installed = join(modules, '@j2008/myagent');
    assert.deepEqual(json(join(installed, 'package.json')), main.manifest, 'installed manifest was not rewritten');
    const installedRequire = createRequire(join(installed, 'bin/myagent.js'));
    assert.equal(json(installedRequire.resolve(`@j2008/myagent-${key}/package.json`)).version, version);
    const command = process.platform === 'win32' ? join(prefix, 'myagent.cmd') : join(prefix, 'bin/myagent');
    const runInstalled = async args => {
      if (process.platform !== 'win32') return execute(command, args, options);
      // Only npm's generated .cmd requires cmd.exe. The distributed Node launcher
      // itself always spawns the native .exe directly with shell:false.
      assert.ok(!/["%\r\n]/.test(command));
      return execute(join(process.env.SystemRoot, 'System32/cmd.exe'), ['/d', '/s', '/c', `""${command}" ${args.join(' ')}"`], { ...options, windowsVerbatimArguments: true });
    };
    const found = await runInstalled(['--version']);
    assert.equal(found.stdout.trim(), `myagent ${version}`, 'native --version must match Rust/npm');
    const doctor = JSON.parse((await runInstalled(['doctor', '--json'])).stdout);
    assert.equal(doctor.exitCode, 0);
    assert.ok(doctor.checks.some(c => c.name === 'sqlite' && c.status === 'ok'));
    assert.ok(doctor.checks.some(c => c.name === 'state-write' && c.status === 'ok'));
    // Exercise launcher-only execution outside npm's command shim too.
    const entry = join(installed, 'bin/myagent.js');
    const demo = await execute(node, [entry, '--demo', 'exec', 'npm smoke', '--json'], options);
    const summary = JSON.parse(demo.stdout.trim().split(/\r?\n/).at(-1));
    assert.equal(summary.exitCode, 0);
    assert.equal(summary.mode, 'PLAN');
    const note = { platform: key, version, verifiedExecution: true, rustAvailable: false,
      packages: packages.map(p => ({ key: p.key, integrity: p.integrity })),
      resolvedTarballs: [...local.hits].sort(), prefix, state: env.MYAGENT_HOME, node: process.version };
    if (report) writeFileSync(report, `${JSON.stringify(note, null, 2)}\n`);
    console.log(JSON.stringify(note, null, 2));
    return note;
  } finally {
    await local.close();
    // Retain only this isolated fixture for inspection; no deletion of user state.
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error('Usage: smoke.mjs TARBALL_DIRECTORY');
  await smoke(resolve(process.argv[2]), process.argv[3]);
}
