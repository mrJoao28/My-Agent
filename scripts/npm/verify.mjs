import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { platforms, validate, tarballs, json } from './common.mjs';
import { inspect } from './archive.mjs';

function files(directory, suffix) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? files(path, suffix) : entry.name.endsWith(suffix) ? [path] : [];
  });
}
export function verify(directory, requireSmoke = true) {
  const version = validate();
  const items = tarballs(directory).map(inspect);
  const keys = [...platforms.map(p => p.key), 'main'];
  assert.deepEqual(items.map(p => p.key).sort(), [...keys].sort(), 'exactly six tarballs required');
  const receipts = files(directory, 'receipt.json').map(json);
  assert.equal(receipts.length, 6);
  for (const item of items) {
    const receipt = receipts.find(r => r.key === item.key);
    assert.ok(receipt);
    for (const field of ['version', 'filename', 'sha256', 'integrity', 'binarySha256']) {
      assert.equal(receipt[field], field === 'version' ? version : item[field], `receipt ${item.key}: ${field}`);
    }
    assert.equal(receipt.name, item.manifest.name);
  }
  const checksums = files(directory, 'SHA256SUMS');
  assert.equal(checksums.length, 6, 'six checksum sidecars required');
  for (const path of checksums) {
    const line = readFileSync(path, 'utf8').trim();
    assert.ok(items.some(p => line === `${p.sha256}  ${p.filename}`), 'checksum sidecar mismatch');
  }
  if (requireSmoke) {
    const reports = files(directory, '.smoke.json').map(json);
    for (const p of platforms) {
      const report = reports.find(r => r.platform === p.key);
      assert.ok(report, `native execution report missing: ${p.key}`);
      assert.equal(report.version, version);
      assert.equal(report.verifiedExecution, true);
      assert.equal(report.rustAvailable, false);
      for (const item of items) {
        assert.equal(report.packages.find(r => r.key === item.key)?.integrity, item.integrity, 'smoke used different tarballs');
      }
    }
  }
  console.log(`Validated ${items.length} tarballs${requireSmoke ? ' and five native executions' : ''} @ ${version}`);
  return items;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error('Usage: verify.mjs DIRECTORY [--before-smoke]');
  verify(resolve(process.argv[2]), process.argv[3] !== '--before-smoke');
}
