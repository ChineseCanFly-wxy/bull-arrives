// Lightweight path check; never compiles Rust or changes account data.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execFileSync, spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { resolveConfig } from 'vite';
import { root, targetDir, tempDir, buildEnv } from './build-env.mjs';

const runtime = JSON.parse(execFileSync(process.execPath,
  ['-e', 'console.log(JSON.stringify({tmp:require("node:os").tmpdir(),target:process.env.CARGO_TARGET_DIR}))'],
  { env: buildEnv, encoding: 'utf8', windowsHide: true }));
assert.equal(runtime.tmp, tempDir);
assert.equal(runtime.target, targetDir);
const args = ['metadata', '--no-deps', '--offline', '--format-version', '1'];
const wrapped = JSON.parse(execFileSync(process.execPath,
  [resolve(root, 'scripts/build-env.mjs'), 'cargo', ...args, '--manifest-path', 'src-tauri/Cargo.toml'],
  { cwd: root, encoding: 'utf8', windowsHide: true }));
const direct = JSON.parse(execFileSync('cargo', args,
  { cwd: resolve(root, 'src-tauri'), encoding: 'utf8', windowsHide: true }));
assert.equal(resolve(wrapped.target_directory), targetDir);
assert.equal(resolve(direct.target_directory), targetDir);
const vite = await resolveConfig({ root }, 'build');
const tauri = JSON.parse(readFileSync(resolve(root, 'src-tauri/tauri.conf.json'), 'utf8'));
assert.equal(resolve(vite.build.outDir), resolve(targetDir, 'frontend'));
assert.equal(resolve(root, 'src-tauri', tauri.build.frontendDist), resolve(vite.build.outDir));
assert.equal(resolve(vite.cacheDir), resolve(targetDir, 'vite-cache'));
const invalid = spawnSync(process.execPath, [resolve(root, 'scripts/build-env.mjs'), 'invalid-tool'],
  { encoding: 'utf8', windowsHide: true });
assert.notEqual(invalid.status, 0);
assert.match(invalid.stderr, /Usage:/);
console.log('构建目录自检通过：Cargo/Vite/Tauri 输出一致，子进程临时文件统一，错误命令不会执行。');
