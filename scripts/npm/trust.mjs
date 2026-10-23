import assert from 'node:assert/strict';
import { appendFileSync } from 'node:fs';
import { isMain, platforms, validate } from './common.mjs';
import { assertAutoContext } from './auto-release.mjs';

export const packages = ['@j2008/myagent', ...platforms.map(p => `@j2008/myagent-${p.key}`)];
export function assertTrustContext(env) {
  assert.equal(env.GITHUB_ACTIONS, 'true');
  assert.equal(env.GITHUB_REPOSITORY, 'mrJoao28/My-Agent');
  if (env.GITHUB_REF === 'refs/heads/main') assertAutoContext(env);
  else {
    assert.equal(env.GITHUB_EVENT_NAME, 'push');
    assert.match(env.GITHUB_REF ?? '', /^refs\/tags\/v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
    assert.equal(env.GITHUB_WORKFLOW_REF, `mrJoao28/My-Agent/.github/workflows/release.yml@${env.GITHUB_REF}`);
  }
  assert.ok(env.ACTIONS_ID_TOKEN_REQUEST_URL && env.ACTIONS_ID_TOKEN_REQUEST_TOKEN, 'id-token: write necessário');
}

// Exchange-only preflight. Never publishes, stores tokens, or logs response bodies.
export async function checkTrust(env, request = fetch) {
  assertTrustContext(env);
  let url;
  try { url = new URL(env.ACTIONS_ID_TOKEN_REQUEST_URL); }
  catch { throw new Error('Endpoint OIDC do GitHub inválido.'); }
  assert.ok(url.protocol === 'https:' && !url.username && !url.password, 'Endpoint OIDC deve usar HTTPS');
  url.searchParams.set('audience', 'npm:registry.npmjs.org');
  let identity;
  try {
    const response = await request(url, {
      headers: { Accept: 'application/json', Authorization: `Bearer ${env.ACTIONS_ID_TOKEN_REQUEST_TOKEN}` },
      redirect: 'error', signal: AbortSignal.timeout(20000),
    });
    if (!response.ok) throw new Error('identity refused');
    identity = (await response.json()).value;
    if (typeof identity !== 'string' || !identity) throw new Error('identity missing');
  } catch { throw new Error('Não foi possível obter a identidade OIDC do GitHub. Confira id-token: write e a conexão.'); }

  return Promise.all(packages.map(async name => {
    try {
      const response = await request(`https://registry.npmjs.org/-/npm/v1/oidc/token/exchange/package/${encodeURIComponent(name)}`, {
        method: 'POST', headers: { Accept: 'application/json', Authorization: `Bearer ${identity}` },
        redirect: 'error', signal: AbortSignal.timeout(20000),
      });
      if (!response.ok) {
        await response.body?.cancel();
        return { name, ok: false, detail: `HTTP ${response.status}` };
      }
      try {
        const data = await response.json();
        if (typeof data.token === 'string' && data.token) return { name, ok: true, detail: 'OIDC aceito' };
      } catch { /* Do not expose malformed response bodies or tokens. */ }
      return { name, ok: false, detail: 'Resposta OIDC inválida' };
    } catch { return { name, ok: false, detail: 'Falha de rede na troca OIDC' }; }
  }));
}

export function trustReport(results) {
  return ['## Cadastro npm por pacote', '', '| Pacote | Troca OIDC |', '| --- | --- |',
    ...results.map(r => `| ${r.name} | ${r.ok ? 'OK' : 'RECUSADO'}: ${r.detail} |`), '',
    'Confira em cada pacote: mrJoao28 / My-Agent / release.yml / npm-release e Allow npm publish.',
    'Esta checagem não publica. Token aceito comprova a troca OIDC; a permissão de publicação direta ainda é exigida no npm.', ''].join('\n');
}

if (isMain(import.meta.url)) {
  validate();
  try {
    const results = await checkTrust(process.env);
    const report = trustReport(results);
    console.log(report);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, report);
    if (results.some(r => !r.ok)) {
      console.error('::error::O npm recusou a troca OIDC de um ou mais pacotes. Corrija os cadastros indicados antes de criar uma release.');
      process.exitCode = 1;
    }
  } catch {
    console.error('::error::Não foi possível validar a identidade OIDC. Confira o contexto da release, id-token: write e a conexão.');
    process.exitCode = 1;
  }
}
