#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const { constants } = require('node:os');
const { join, dirname } = require('node:path');
const { readFileSync } = require('node:fs');

const platforms = new Set(['win32-x64', 'linux-x64', 'linux-arm64', 'darwin-arm64', 'darwin-x64']);
const platform = `${process.platform}-${process.arch}`;
const version = require('../package.json').version;
let binary;
try {
  if (!platforms.has(platform)) throw new Error('unsupported platform');
  const name = `@j2008/myagent-${platform}`;
  const manifest = require.resolve(`${name}/package.json`);
  const installed = JSON.parse(readFileSync(manifest, 'utf8'));
  if (installed.name !== name || installed.version !== version) throw new Error('incompatible binary package');
  binary = join(dirname(manifest), 'bin', process.platform === 'win32' ? 'myagent.exe' : 'myagent');
} catch {
  console.error(`My-Agent: binário ${platform}@${version} indisponível. Reinstale @j2008/myagent com optionalDependencies habilitadas (sem --omit=optional). Plataformas: ${[...platforms].join(', ')}.`);
  process.exitCode = 1;
}

if (binary) {
  // Inherit the console/TTY, cwd and environment; never interpolate a shell command.
  const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit', shell: false });
  const handlers = new Map();
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    const handler = () => {
      // Windows console events reach the attached native child directly. kill()
      // there would forcibly terminate it, bypassing Rust's orderly cleanup.
      if (process.platform !== 'win32' && child.exitCode === null && child.signalCode === null) {
        child.kill(signal);
      }
    };
    handlers.set(signal, handler);
    process.on(signal, handler);
  }
  const cleanup = () => {
    for (const [signal, handler] of handlers) process.removeListener(signal, handler);
  };
  child.once('error', () => {
    cleanup();
    console.error(`My-Agent: não foi possível executar o binário ${platform}@${version}. Reinstale o pacote e confira as permissões do executável.`);
    process.exitCode = 1;
  });
  child.once('exit', (code, signal) => {
    cleanup();
    if (signal && process.platform !== 'win32') {
      // Preserve signal termination (and the shell's conventional exit status).
      process.kill(process.pid, signal);
    } else {
      process.exitCode = code ?? 128 + (constants.signals[signal] ?? 1);
    }
  });
}
