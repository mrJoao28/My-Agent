import assert from 'node:assert/strict';
import { gunzipSync } from 'node:zlib';
import { readFileSync } from 'node:fs';
import { basename } from 'node:path';
import { validate, platforms, json, manifestPath, binaryName, sha, integrity, root } from './common.mjs';
import { join } from 'node:path';
import { checkBinary } from './binary.mjs';

// Inspect the actual immutable npm tarball, including executable mode and file allowlist.
export function inspect(path) {
  const bytes = readFileSync(path);
  assert.ok(bytes.length < 100 * 1024 * 1024, 'compressed package too large');
  const tar = gunzipSync(bytes, { maxOutputLength: 200 * 1024 * 1024 });
  const files = new Map();
  const text = buffer => buffer.toString('utf8').replace(/\0.*$/s, '');
  for (let at = 0; at + 512 <= tar.length; ) {
    const header = tar.subarray(at, at + 512);
    if (header.every(byte => byte === 0)) break;
    const sum = header.reduce((total, byte, i) => total + (i >= 148 && i < 156 ? 32 : byte), 0);
    assert.equal(sum, Number.parseInt(text(header.subarray(148, 156)).trim(), 8), 'tar header checksum');
    const size = Number.parseInt(text(header.subarray(124, 136)).trim(), 8);
    assert.ok(Number.isSafeInteger(size) && size >= 0 && at + 512 + size <= tar.length);
    const prefix = text(header.subarray(345, 500));
    const name = `${prefix ? `${prefix}/` : ''}${text(header.subarray(0, 100))}`;
    const type = text(header.subarray(156, 157));
    assert.ok(type === '0' || type === '', 'only regular package files are allowed');
    assert.ok(!files.has(name), 'duplicate tar entry');
    files.set(name, { bytes: tar.subarray(at + 512, at + 512 + size), mode: Number.parseInt(text(header.subarray(100, 108)), 8) });
    at += 512 + Math.ceil(size / 512) * 512;
  }
  const manifest = JSON.parse(files.get('package/package.json')?.bytes.toString('utf8') ?? '{}');
  const key = manifest.name === '@j2008/myagent' ? 'main' : platforms.find(p => manifest.name === `@j2008/myagent-${p.key}`)?.key;
  assert.ok(key, `unknown package in ${basename(path)}`);
  assert.deepEqual(manifest, json(manifestPath(key)), 'packed manifest differs from validated source');
  assert.equal(manifest.version, validate());
  const file = `package/bin/${key === 'main' ? 'myagent.js' : binaryName(key)}`;
  assert.deepEqual([...files.keys()].sort(), ['package/package.json', 'package/README.md', 'package/LICENSE', file].sort());
  // Windows tarballs use 0644 even for npm bin entries. npm creates its .cmd
  // launcher there; production main is packed on Linux and must retain 0755.
  if (!key.startsWith('win32-') && !(key === 'main' && process.platform === 'win32')) {
    assert.ok(files.get(file).mode & 0o111, 'executable mode must survive npm pack');
  }
  assert.deepEqual(files.get('package/LICENSE').bytes, readFileSync(join(root, 'LICENSE')));
  assert.deepEqual(files.get('package/README.md').bytes, readFileSync(join(root, 'npm/myagent/README.md')));
  if (key === 'main') assert.deepEqual(files.get(file).bytes, readFileSync(join(root, 'npm/myagent/bin/myagent.js')));
  if (key !== 'main') checkBinary(key, files.get(file).bytes);
  return { path, key, manifest, filename: basename(path), bytes, sha256: sha(bytes), integrity: integrity(bytes), shasum: sha(bytes, 'sha1'), binarySha256: sha(files.get(file).bytes) };
}
