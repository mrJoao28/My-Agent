import assert from 'node:assert/strict';
import { mkdirSync, copyFileSync, chmodSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkBinary } from './binary.mjs';
import { root, platforms, manifestPath, binaryName, json, sha, integrity, validate, npm } from './common.mjs';

export function pack(key, binary, output) {
  const version = validate();
  assert.ok(key === 'main' || platforms.some(p => p.key === key), 'unknown platform');
  output = resolve(output);
  mkdirSync(output, { recursive: true });
  const stage = join(output, 'package');
  mkdirSync(stage); // Refuse to reuse stale staging data.
  mkdirSync(join(stage, 'bin'));
  copyFileSync(manifestPath(key), join(stage, 'package.json'));
  copyFileSync(join(root, 'LICENSE'), join(stage, 'LICENSE'));
  copyFileSync(join(root, 'npm/myagent/README.md'), join(stage, 'README.md'));
  const file = key === 'main' ? 'myagent.js' : binaryName(key);
  copyFileSync(key === 'main' ? join(root, 'npm/myagent/bin/myagent.js') : resolve(binary), join(stage, 'bin', file));
  chmodSync(join(stage, 'bin', file), 0o755);
  if (key !== 'main') checkBinary(key, readFileSync(join(stage, 'bin', file)));
  const packed = JSON.parse(npm(['pack', '--json', '--ignore-scripts', '--pack-destination', output], { cwd: stage }));
  assert.equal(packed.length, 1);
  const item = packed[0];
  const manifest = json(join(stage, 'package.json'));
  assert.equal(item.name, manifest.name);
  assert.equal(item.version, version);
  assert.deepEqual(item.files.map(f => f.path).sort(), ['LICENSE', 'README.md', 'package.json', `bin/${file}`].sort());
  if (key === 'main') assert.ok(item.unpackedSize < 20000, 'launcher must remain small');
  const bytes = readFileSync(join(output, item.filename));
  assert.equal(item.integrity, integrity(bytes));
  const receipt = { key, name: item.name, version, filename: item.filename, sha256: sha(bytes), integrity: item.integrity, binarySha256: sha(readFileSync(join(stage, 'bin', file))) };
  writeFileSync(join(output, 'receipt.json'), `${JSON.stringify(receipt, null, 2)}\n`);
  writeFileSync(join(output, 'SHA256SUMS'), `${receipt.sha256}  ${item.filename}\n`);
  return receipt;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [key, binary, output] = process.argv.slice(2);
  if (!key || !binary || !output) throw new Error('Usage: pack.mjs KEY BINARY_OR_DASH OUTPUT');
  console.log(JSON.stringify(pack(key, binary, output)));
}
