import { mkdirSync, realpathSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
mkdirSync(resolve(root, 'src-tauri/target'), { recursive: true });
export const targetDir = realpathSync(resolve(root, 'src-tauri/target'));
export const tempDir = resolve(targetDir, 'tmp');
mkdirSync(tempDir, { recursive: true });
export const buildEnv = { ...process.env, CARGO_TARGET_DIR: targetDir, TMP: tempDir, TEMP: tempDir, TMPDIR: tempDir };

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [tool, ...args] = process.argv.slice(2);
  if (tool !== 'tauri' && tool !== 'cargo') throw new Error('Usage: node scripts/build-env.mjs <tauri|cargo> [...args]');
  const child = spawn(tool === 'tauri' ? process.execPath : 'cargo',
    tool === 'tauri' ? [resolve(root, 'node_modules/@tauri-apps/cli/tauri.js'), ...args] : args,
    { cwd: root, env: buildEnv, stdio: 'inherit', windowsHide: true });
  child.on('error', error => { console.error(error.message); process.exitCode = 1; });
  child.on('exit', code => { process.exitCode = code ?? 1; });
}
