import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { platforms, validate, npmCli } from './common.mjs';
import { verify } from './verify.mjs';

const exec = promisify(execFile);
const publicRegistry = 'https://registry.npmjs.org/';
export async function lookup(item) {
  try {
    const { stdout } = await exec(process.execPath, [npmCli(), 'view', `${item.manifest.name}@${item.manifest.version}`, 'dist.integrity', '--json', '--registry', publicRegistry, '--fetch-retries=0'], { timeout: 30000 });
    const value = JSON.parse(stdout);
    assert.match(value, /^sha512-/);
    return value;
  } catch (error) {
    // E401/E403, timeouts, malformed output and registry outages never mean absent.
    if (/"code"\s*:\s*"E404"/.test(error.stdout ?? '')) return null;
    throw error;
  }
}
export async function release(items, { lookup, publish, pause = () => new Promise(r => setTimeout(r, 5000)), attempts = 12 }) {
  const keys = [...platforms.map(p => p.key), 'main'];
  assert.deepEqual(items.map(p => p.key).sort(), [...keys].sort(), 'all six tested tarballs required');
  const ordered = keys.map(key => items.find(p => p.key === key));
  const existing = new Map();
  // Validate every existing version before publishing anything, including recovery.
  for (const item of ordered) {
    const found = await lookup(item);
    if (found !== null) assert.equal(found, item.integrity, `${item.manifest.name}: already published with different integrity; use a new version`);
    existing.set(item.key, found);
  }
  const confirm = async item => {
    for (let n = 0; n < attempts; n++) {
      const found = await lookup(item);
      if (found !== null) {
        assert.equal(found, item.integrity, `${item.manifest.name}: registry integrity mismatch`);
        return;
      }
      if (n + 1 < attempts) await pause();
    }
    throw new Error(`${item.manifest.name}: version not visible; main package remains blocked`);
  };
  for (const item of ordered.filter(p => p.key !== 'main')) {
    if (!existing.get(item.key)) await publish(item);
    await confirm(item);
  }
  // Reconfirm the entire platform set immediately before publishing the launcher.
  for (const item of ordered.filter(p => p.key !== 'main')) await confirm(item);
  const main = ordered.at(-1);
  if (!existing.get('main')) await publish(main);
  await confirm(main);
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, directory] = process.argv.slice(2);
  assert.ok(['--check', '--execute'].includes(mode) && directory, 'Usage: publish.mjs --check|--execute TARBALL_DIRECTORY');
  const version = validate();
  const items = verify(resolve(directory));
  if (mode === '--execute') {
    validate(process.env.GITHUB_REF_NAME);
    assert.equal(process.env.GITHUB_ACTIONS, 'true');
    assert.equal(process.env.GITHUB_EVENT_NAME, 'push');
    assert.equal(process.env.GITHUB_REPOSITORY, 'mrJoao28/My-Agent');
    assert.equal(process.env.GITHUB_REF, `refs/tags/v${version}`);
    assert.equal(process.env.GITHUB_WORKFLOW_REF, `mrJoao28/My-Agent/.github/workflows/release.yml@refs/tags/v${version}`);
    assert.ok(process.env.ACTIONS_ID_TOKEN_REQUEST_URL, 'OIDC permission required');
    const tool = (await exec(process.execPath, [npmCli(), '--version'])).stdout.trim().split('.').map(Number);
    assert.ok(tool[0] > 11 || tool[0] === 11 && (tool[1] > 5 || tool[1] === 5 && tool[2] >= 1), 'Trusted Publishers requires npm >= 11.5.1');
    await release(items, { lookup, publish: async item => {
      console.log(`Publishing ${item.manifest.name}@${version}`);
      await exec(process.execPath, [npmCli(), 'publish', item.path, '--provenance', '--access', 'public', '--registry', publicRegistry], { timeout: 180000 });
    } });
  } else {
    await release(items, { lookup, publish: async item => { throw new Error(`${item.manifest.name}@${version} is not published; check mode performs no writes`); }, attempts: 1 });
  }
}
