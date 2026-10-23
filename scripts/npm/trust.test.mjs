import test from 'node:test';
import assert from 'node:assert/strict';
import { checkTrust, assertTrustContext, trustReport, packages } from './trust.mjs';

const env = {
  GITHUB_ACTIONS: 'true', GITHUB_REPOSITORY: 'mrJoao28/My-Agent', GITHUB_EVENT_NAME: 'push',
  GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'mrJoao28/My-Agent/.github/workflows/release.yml@refs/heads/main',
  NPM_AUTO_RELEASE: 'true', ACTIONS_ID_TOKEN_REQUEST_URL: 'https://actions.example.invalid/token?api-version=2&audience=old',
  ACTIONS_ID_TOKEN_REQUEST_TOKEN: 'fixture-request-secret',
};
const json = (value, status = 200) => new Response(JSON.stringify(value), { status });
test('trust preflight validates every package with the npm audience and never returns credentials', async () => {
  const calls = [];
  const results = await checkTrust(env, async (url, options) => {
    calls.push({ url: new URL(url), options });
    assert.equal(options.redirect, 'error');
    assert.ok(options.signal instanceof AbortSignal);
    return calls.length === 1 ? json({ value: 'fixture-identity-secret' }) : json({ token: 'fixture-npm-secret' }, 201);
  });
  assert.equal(calls.length, packages.length + 1);
  assert.equal(calls[0].url.searchParams.get('api-version'), '2');
  assert.deepEqual(calls[0].url.searchParams.getAll('audience'), ['npm:registry.npmjs.org']);
  assert.equal(calls[0].options.headers.Authorization, 'Bearer fixture-request-secret');
  assert.deepEqual(calls.slice(1).map(c => decodeURIComponent(c.url.pathname.split('/').at(-1))), packages);
  for (const call of calls.slice(1)) {
    assert.equal(call.options.method, 'POST');
    assert.equal(call.url.origin, 'https://registry.npmjs.org');
    assert.equal(call.options.headers.Authorization, 'Bearer fixture-identity-secret');
  }
  assert.ok(results.every(r => r.ok));
  assert.doesNotMatch(JSON.stringify(results) + trustReport(results), /fixture-.*secret/);
});
test('all missing publishers are reported together without leaking server errors', async () => {
  let calls = 0;
  const results = await checkTrust(env, async () => {
    if (calls++ === 0) return json({ value: 'fixture-identity-secret' });
    if (calls === 2) return json({ token: 'fixture-npm-secret' }, 201);
    if (calls === 3) return json({ message: 'fixture-server-secret' }, 404);
    if (calls === 4) return json({ message: 'fixture-server-secret' }, 403);
    if (calls === 5) return json({ value: 'fixture-server-secret' });
    throw new Error('fixture-network-secret');
  });
  assert.equal(results.length, packages.length);
  assert.equal(results.filter(r => !r.ok).length, packages.length - 1);
  assert.ok(results.some(r => r.detail === 'HTTP 404'));
  assert.ok(results.some(r => r.detail === 'HTTP 403'));
  assert.doesNotMatch(JSON.stringify(results) + trustReport(results), /fixture-.*secret/);
});
test('forks, PRs, disabled automation, wrong workflows and missing OIDC stop before requests', async () => {
  const request = async () => assert.fail('must not request credentials');
  for (const patch of [{ GITHUB_REPOSITORY: 'fork/My-Agent' }, { GITHUB_EVENT_NAME: 'pull_request' },
    { NPM_AUTO_RELEASE: 'false' }, { ACTIONS_ID_TOKEN_REQUEST_TOKEN: '' },
    { GITHUB_REF: 'refs/heads/test' }, { GITHUB_WORKFLOW_REF: 'other' },
    { ACTIONS_ID_TOKEN_REQUEST_URL: 'http://actions.example.invalid/token' }]) {
    await assert.rejects(checkTrust({ ...env, ...patch }, request));
  }
  assertTrustContext({ ...env, GITHUB_EVENT_NAME: 'workflow_dispatch', REQUEST_PUBLISH: 'true' });
  assertTrustContext({ ...env, GITHUB_REF: 'refs/tags/v0.2.2', GITHUB_WORKFLOW_REF: 'mrJoao28/My-Agent/.github/workflows/release.yml@refs/tags/v0.2.2' });
});
test('identity service failure never reaches npm or exposes the response', async () => {
  for (const response of [json({ value: 'fixture-identity-secret' }, 403), json({ other: 'fixture-identity-secret' })]) {
    let calls = 0;
    await assert.rejects(checkTrust(env, async () => { calls++; return response; }), error => {
      assert.match(error.message, /identidade OIDC/);
      assert.doesNotMatch(error.message, /fixture-identity-secret/);
      return true;
    });
    assert.equal(calls, 1);
  }
});
