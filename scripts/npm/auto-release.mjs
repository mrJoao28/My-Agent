import assert from 'node:assert/strict';
import { appendFileSync } from 'node:fs';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { root, run, npmCli, platforms, validate, isMain } from './common.mjs';
import { setVersion } from './version.mjs';

const stable = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
function parts(version) {
  assert.match(version, stable, 'expected stable SemVer');
  const result = version.split('.').map(Number);
  assert.ok(result.every(Number.isSafeInteger), 'version component too large');
  return result;
}
export function compare(a, b) {
  const left = parts(a), right = parts(b);
  for (let i = 0; i < 3; i++) if (left[i] !== right[i]) return Math.sign(left[i] - right[i]);
  return 0;
}
export function releasePath(path) {
  return /^(Cargo\.(toml|lock)|rust-toolchain\.toml|LICENSE)$/.test(path)
    || /^(crates\/|npm\/|scripts\/npm\/)/.test(path)
    || /^\.github\/workflows\/(release|ci)\.yml$/.test(path);
}
export function planRelease({ current, previous, published = [], files, messages }) {
  parts(current);
  if (previous) parts(previous);
  for (const version of published) parts(version);
  const changed = files.some(releasePath);
  const floor = [current, previous, ...published].filter(Boolean).sort(compare).at(-1);
  if (!changed) {
    if (!previous) return null;
    const publishedCount = published.filter(v => v === previous).length;
    if (publishedCount === platforms.length + 1) return null;
    // A prepared tag is not a published release. Rebuild it only when none of
    // its packages exists; otherwise use a fresh patch to avoid byte conflicts.
    if (publishedCount === 0 && floor === previous) return { version: previous, level: 'retry' };
  }
  // Unprefixed code changes still get a patch. Docs outside the packages do not.
  const level = !changed ? 'patch' : messages.some(m => /^[a-z]+(?:\([^\n)]+\))?!:/m.test(m) || /^BREAKING[ -]CHANGE:/m.test(m))
    ? 'major' : messages.some(m => /^feat(?:\([^\n)]+\))?:/m.test(m)) ? 'minor' : 'patch';
  if (!previous && published.length === 0) return { version: current, level: 'initial' };
  // A manually prepared version above every tag/published package is kept.
  if ([previous, ...published].filter(Boolean).every(v => compare(current, v) > 0)) {
    return { version: current, level: 'prepared' };
  }
  const [major, minor, patch] = parts(floor);
  const next = level === 'major' ? [major + 1, 0, 0] : level === 'minor' ? [major, minor + 1, 0] : [major, minor, patch + 1];
  const version = next.join('.');
  parts(version);
  return { version, level };
}
export function assertAutoContext(env) {
  assert.equal(env.GITHUB_ACTIONS, 'true');
  assert.equal(env.GITHUB_REPOSITORY, 'mrJoao28/My-Agent');
  assert.equal(env.GITHUB_REF, 'refs/heads/main');
  assert.equal(env.GITHUB_WORKFLOW_REF, 'mrJoao28/My-Agent/.github/workflows/release.yml@refs/heads/main');
  assert.equal(env.NPM_AUTO_RELEASE, 'true', 'configure npm Trusted Publishers before enabling NPM_AUTO_RELEASE');
  assert.ok(env.GITHUB_EVENT_NAME === 'push' || env.GITHUB_EVENT_NAME === 'workflow_dispatch' && env.REQUEST_PUBLISH === 'true');
}
const exec = promisify(execFile);
export async function packageVersions(name, query = name => exec(process.execPath,
  [npmCli(), 'view', name, 'versions', '--json', '--registry=https://registry.npmjs.org/', '--fetch-retries=0'], { timeout: 30000 })) {
  try {
    const { stdout } = await query(name);
    const value = JSON.parse(stdout);
    assert.ok(typeof value === 'string' || Array.isArray(value) && value.every(v => typeof v === 'string'), 'malformed registry versions');
    return [...new Set((Array.isArray(value) ? value : [value]).filter(v => stable.test(v)))];
  } catch (error) {
    // Only a structured E404 means absent; authorization/network failures block.
    if (/"code"\s*:\s*"E404"/.test(error.stdout ?? '')) return [];
    throw error;
  }
}
export async function registryVersions(query) {
  // All platforms matter: a previous failed publication may have no main package.
  return (await Promise.all(['@j2008/myagent', ...platforms.map(p => `@j2008/myagent-${p.key}`)]
    .map(name => packageVersions(name, query)))).flat();
}
export function prepareRelease(base, published, { push = false } = {}) {
  const git = args => run('git', args, { cwd: base }).trim();
  assert.equal(git(['status', '--porcelain']), '', 'release checkout must be clean');
  const source = git(['rev-parse', 'HEAD']);
  assert.match(source, /^[a-f0-9]{40}$/);
  const tags = git(['tag', '--merged', 'HEAD', '--list', 'v*']).split('\n')
    .filter(t => t.startsWith('v') && stable.test(t.slice(1))).sort((a, b) => compare(a.slice(1), b.slice(1)));
  const previousTag = tags.at(-1);
  const files = git(previousTag ? ['diff', '--name-only', '-z', `${previousTag}..HEAD`] : ['ls-files', '-z']).split('\0');
  const messages = git(['log', '--format=%B%x00', ...(previousTag ? [`${previousTag}..HEAD`] : ['HEAD'])]).split('\0').map(m => m.trim());
  const plan = planRelease({ current: validate(undefined, base), previous: previousTag?.slice(1), published, files, messages });
  if (!plan) return { publish: false, ref: source };
  const tag = `v${plan.version}`;
  if (plan.level === 'retry') {
    const ref = git(['rev-parse', `refs/tags/${tag}^{commit}`]);
    return { publish: true, ref, tag, source };
  }
  assert.equal(git(['tag', '--list', tag]), '', 'release tag already exists; never overwrite it');
  setVersion(plan.version, base);
  git(['add', 'Cargo.toml', 'Cargo.lock', 'npm/myagent/package.json', ...platforms.map(p => `npm/platforms/${p.key}/package.json`)]);
  // Initial releases may not change the version; an empty marker still pins them.
  git(['-c', 'user.name=github-actions[bot]', '-c', 'user.email=41898282+github-actions[bot]@users.noreply.github.com',
    'commit', '--allow-empty', '-m', `chore(release): ${tag}`]);
  const ref = git(['rev-parse', 'HEAD']);
  git(['tag', tag, ref]);
  if (push) git(['push', '--atomic', 'origin', `${ref}:refs/heads/main`, `refs/tags/${tag}`]);
  return { publish: true, ref, tag, source };
}
if (isMain(import.meta.url)) {
  assertAutoContext(process.env);
  // Fetch the latest main after acquiring the workflow's shared concurrency lock.
  run('git', ['fetch', 'origin', 'main', '--tags'], { cwd: root });
  run('git', ['checkout', '--detach', 'refs/remotes/origin/main'], { cwd: root });
  const result = prepareRelease(root, await registryVersions(), { push: true });
  assert.ok(process.env.GITHUB_OUTPUT);
  for (const [key, value] of Object.entries(result)) appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${value}\n`);
  console.log(result.publish ? `Prepared ${result.tag}; publication still requires all quality/build/smoke gates.` : 'No package changes; all six packages of the last release are already published.');
}
