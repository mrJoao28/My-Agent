import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { root, platforms, validate, json } from './common.mjs';

const args = process.argv.slice(2);
if (args[0] === '--set' && args.length === 2) {
  const next = args[1];
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(next)) throw new Error('Expected a stable SemVer');
  validate();
  const cargo = join(root, 'Cargo.toml');
  writeFileSync(cargo, readFileSync(cargo, 'utf8').replace(/(\[workspace.package\][\s\S]*?^version = ")[^"]+("$)/m, `$1${next}$2`));
  const lock = join(root, 'Cargo.lock');
  writeFileSync(lock, readFileSync(lock, 'utf8').replace(/(\[\[package\]\]\r?\nname = "myagent-[^"]+"\r?\nversion = ")[^"]+(")/g, `$1${next}$2`));
  for (const key of ['main', ...platforms.map(p => p.key)]) {
    const path = join(root, key === 'main' ? 'npm/myagent/package.json' : `npm/platforms/${key}/package.json`);
    const manifest = json(path);
    manifest.version = next;
    if (key === 'main') for (const name of Object.keys(manifest.optionalDependencies)) manifest.optionalDependencies[name] = next;
    writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
  }
} else if (args.length > 1) {
  throw new Error('Usage: version.mjs [vVERSION] | --set VERSION');
}
console.log(`Validated Rust/npm version ${validate(args[0] === '--set' ? undefined : args[0])}`);
