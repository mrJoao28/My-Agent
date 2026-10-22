import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { root, platforms, run, validate } from './common.mjs';
import { setVersion } from './version.mjs';
import { planRelease, compare, assertAutoContext, prepareRelease, packageVersions, registryVersions } from './auto-release.mjs';
import { assertPublishContext } from './publish.mjs';

const owned = [];
after(() => { for (const dir of owned) rmSync(dir, { recursive: true, maxRetries: 5, retryDelay: 100 }); });
function temporary() {
  const dir = mkdtempSync(join(tmpdir(), 'myagent auto release test '));
  owned.push(dir);
  return dir;
}
const git = (base, ...args) => run('git', args, { cwd: base }).trim();
function config(base) {
  git(base, 'config', 'user.name', 'Release Fixture');
  git(base, 'config', 'user.email', 'fixture@example.invalid');
  git(base, 'config', 'commit.gpgsign', 'false');
  git(base, 'config', 'tag.gpgsign', 'false');
}
function commit(base, message, file = 'crates/fixture.rs') {
  mkdirSync(dirname(join(base, file)), { recursive: true });
  writeFileSync(join(base, file), message);
  git(base, 'add', file);
  git(base, 'commit', '-m', message);
  return git(base, 'rev-parse', 'HEAD');
}
function fixture() {
  const base = temporary(), remote = join(temporary(), 'remote.git');
  const files = ['Cargo.toml', 'Cargo.lock', 'npm/myagent/package.json', 'npm/myagent/bin/myagent.js',
    ...platforms.map(p => `npm/platforms/${p.key}/package.json`)];
  for (const file of files) {
    mkdirSync(dirname(join(base, file)), { recursive: true });
    copyFileSync(join(root, file), join(base, file));
  }
  mkdirSync(join(base, 'crates'));
  setVersion('0.1.0', base);
  git(base, 'init', '--initial-branch=main');
  config(base);
  git(base, 'add', '.');
  git(base, 'commit', '-m', 'feat: initial fixture');
  git(base, 'init', '--bare', '--initial-branch=main', remote);
  git(base, 'remote', 'add', 'origin', remote);
  git(base, 'push', '-u', 'origin', 'main');
  return { base, remote };
}
test('release planning handles initial, numeric SemVer, features, breaking changes and unprefixed code', () => {
  const source = { current: '0.1.0', previous: '0.1.0', files: ['crates/tui/src/app.rs'], messages: ['fix: login'] };
  assert.equal(compare('0.9.0', '0.10.0'), -1);
  assert.equal(planRelease({ ...source, previous: undefined }).version, '0.1.0');
  assert.equal(planRelease(source).version, '0.1.1');
  assert.equal(planRelease({ ...source, messages: ['feat(tui): account'] }).version, '0.2.0');
  for (const message of ['feat!: change protocol', 'fix(api)!: change protocol', 'feat: protocol\n\nBREAKING CHANGE: old format removed']) {
    assert.equal(planRelease({ ...source, messages: [message] }).version, '1.0.0');
  }
  assert.equal(planRelease({ ...source, messages: ['Update app.rs'] }).version, '0.1.1');
  assert.equal(planRelease({ ...source, files: ['README.md', 'docs/tui.md'] }), null);
  assert.equal(planRelease({ ...source, files: ['npm/myagent/README.md'] }).version, '0.1.1');
  assert.equal(planRelease({ ...source, published: ['0.1.3'] }).version, '0.1.4');
  assert.equal(planRelease({ ...source, current: '0.3.0' }).version, '0.3.0');
  assert.throws(() => planRelease({ ...source, current: 'bad' }));
});
test('registry discovery distinguishes absent packages from auth/network/malformed failures', async () => {
  const queried = [];
  const versions = await registryVersions(async name => {
    queried.push(name);
    return { stdout: name.endsWith('win32-x64') ? '["0.1.3"]' : '["0.1.0"]' };
  });
  assert.equal(queried.length, 6);
  assert.equal(new Set(queried).size, 6);
  assert.equal(planRelease({ current: '0.1.0', previous: '0.1.0', published: versions,
    files: ['crates/tui/src/app.rs'], messages: ['fix: login'] }).version, '0.1.4');
  assert.deepEqual(await packageVersions('fixture', async () => ({ stdout: '["0.1.0","0.2.0-beta.1","0.1.4"]' })), ['0.1.0', '0.1.4']);
  assert.deepEqual(await packageVersions('fixture', async () => { throw { stdout: '{"error":{"code":"E404"}}' }; }), []);
  for (const error of [{ stdout: '{"error":{"code":"E403"}}' }, { stdout: 'network E404' }, new Error('timeout')]) {
    await assert.rejects(packageVersions('fixture', async () => { throw error; }));
  }
  await assert.rejects(packageVersions('fixture', async () => ({ stdout: '{}' })));
});
test('atomic release pins all manifests, skips docs, then prepares one feature version in a local bare fixture', () => {
  const { base, remote } = fixture();
  const first = prepareRelease(base, [], { push: true });
  assert.equal(first.tag, 'v0.1.0');
  assert.equal(git(remote, 'rev-parse', 'refs/heads/main'), first.ref);
  assert.equal(git(remote, 'rev-parse', 'refs/tags/v0.1.0'), first.ref);
  commit(base, 'docs: explain fixture', 'README.md');
  const docs = prepareRelease(base, ['0.1.0'], { push: true });
  assert.equal(docs.publish, false);
  commit(base, 'feat: another command');
  const next = prepareRelease(base, ['0.1.0'], { push: true });
  assert.equal(next.tag, 'v0.2.0');
  assert.equal(validate('v0.2.0', base), '0.2.0');
  assert.equal(git(remote, 'rev-parse', 'refs/heads/main'), next.ref);
  assert.equal(prepareRelease(base, ['0.2.0']).publish, false);
});
test('dirty worktrees and unrelated tag collisions stop before release writes', () => {
  const { base } = fixture();
  writeFileSync(join(base, 'README.md'), 'untracked');
  assert.throws(() => prepareRelease(base, []), /clean/);
  git(base, 'add', 'README.md');
  git(base, 'commit', '-m', 'docs: fixture');
  // An unmerged tag still reserves its version globally.
  git(base, 'checkout', '--orphan', 'other');
  git(base, 'add', '.');
  git(base, 'commit', '-m', 'other history');
  git(base, 'tag', 'v0.1.0');
  git(base, 'checkout', 'main');
  assert.throws(() => prepareRelease(base, []), /tag already exists/);
  assert.equal(validate(undefined, base), '0.1.0');
});
test('a competing main update rejects the entire atomic push without publishing a tag', () => {
  const { base, remote } = fixture();
  prepareRelease(base, [], { push: true });
  const other = join(temporary(), 'other checkout');
  git(base, 'clone', remote, other);
  config(other);
  const advanced = commit(other, 'fix: competing change');
  git(other, 'push', 'origin', 'main');
  commit(base, 'fix: stale change');
  assert.throws(() => prepareRelease(base, ['0.1.0'], { push: true }));
  assert.equal(git(remote, 'rev-parse', 'refs/heads/main'), advanced);
  assert.equal(git(remote, 'tag', '--list', 'v0.1.1'), '');
});
const auto = {
  GITHUB_ACTIONS: 'true', GITHUB_REPOSITORY: 'mrJoao28/My-Agent',
  GITHUB_EVENT_NAME: 'push', GITHUB_REF: 'refs/heads/main',
  GITHUB_WORKFLOW_REF: 'mrJoao28/My-Agent/.github/workflows/release.yml@refs/heads/main',
  NPM_AUTO_RELEASE: 'true',
};
test('automatic preparation rejects forks, PRs, wrong workflows and disabled activation', () => {
  assertAutoContext(auto);
  assertAutoContext({ ...auto, GITHUB_EVENT_NAME: 'workflow_dispatch', REQUEST_PUBLISH: 'true' });
  for (const patch of [{ GITHUB_EVENT_NAME: 'pull_request' }, { GITHUB_EVENT_NAME: 'workflow_dispatch' },
    { GITHUB_REPOSITORY: 'fork/My-Agent' }, { GITHUB_REF: 'refs/heads/test' },
    { GITHUB_WORKFLOW_REF: 'mrJoao28/My-Agent/.github/workflows/other.yml@refs/heads/main' }, { NPM_AUTO_RELEASE: 'false' }]) {
    assert.throws(() => assertAutoContext({ ...auto, ...patch }));
  }
});
test('publication binds OIDC, tag and tested SHA for automatic and manual tag releases', () => {
  const sha = 'a'.repeat(40), other = 'b'.repeat(40);
  const env = { ...auto, AUTO_RELEASE: 'true', RELEASE_SHA: sha, RELEASE_TAG: 'v0.1.0', ACTIONS_ID_TOKEN_REQUEST_URL: 'https://example.invalid/oidc' };
  assertPublishContext(env, '0.1.0', sha, sha);
  assertPublishContext({ ...env, AUTO_RELEASE: 'false', GITHUB_REF: 'refs/tags/v0.1.0', GITHUB_WORKFLOW_REF: 'mrJoao28/My-Agent/.github/workflows/release.yml@refs/tags/v0.1.0' }, '0.1.0', sha, sha);
  assert.throws(() => assertPublishContext(env, '0.1.0', other, sha));
  assert.throws(() => assertPublishContext(env, '0.1.0', sha, other));
  for (const patch of [{ ACTIONS_ID_TOKEN_REQUEST_URL: '' }, { RELEASE_TAG: 'v0.2.0' },
    { GITHUB_EVENT_NAME: 'pull_request' }, { NPM_AUTO_RELEASE: 'false' }, { AUTO_RELEASE: 'false' }]) {
    assert.throws(() => assertPublishContext({ ...env, ...patch }, '0.1.0', sha, sha));
  }
});
