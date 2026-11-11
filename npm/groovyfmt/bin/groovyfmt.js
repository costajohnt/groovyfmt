#!/usr/bin/env node
// Runs the groovyfmt binary from the @groovyfmt/<os>-<arch> package that npm
// installed for this platform as an optional dependency.
'use strict';
const { spawnSync } = require('node:child_process');

const pkg = `@groovyfmt/${process.platform}-${process.arch}`;
let bin;
try {
  bin = require.resolve(`${pkg}/bin/groovyfmt${process.platform === 'win32' ? '.exe' : ''}`);
} catch {
  console.error(
    `groovyfmt: ${pkg} is not installed. Prebuilt binaries exist for linux-x64, linux-arm64, ` +
      `darwin-x64, darwin-arm64 and win32-x64; if this is one of them, reinstall without --omit=optional ` +
      `or --no-optional. Otherwise see https://github.com/costajohnt/groovyfmt#install`,
  );
  process.exit(2);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' });
if (result.error) {
  console.error(`groovyfmt: failed to run ${bin}: ${result.error.message}`);
  process.exit(2);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 2);
