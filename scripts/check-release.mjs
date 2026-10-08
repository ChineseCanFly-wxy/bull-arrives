#!/usr/bin/env node
// 本地和CI共用的只读发版检查；不暂存、不提交、不联网。
import { readFileSync, statSync, lstatSync, readdirSync, existsSync } from 'node:fs';
import { resolve, dirname, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { extractChangelog } from './extract-changelog.mjs';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const versionFiles = ['package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json'];
export const read = (base, name) => readFileSync(resolve(base, name), 'utf8');
export const json = (base, name) => JSON.parse(read(base, name));
export function cargoVersion(content, lock = false) {
  const section = content.split(lock ? /^\[\[package\]\]\s*$/m : /^\[package\]\s*$/m)
    .find(part => /^name\s*=\s*"bull-arrives"\s*$/m.test(part));
  const version = section?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version) throw new Error('Cargo 中缺少 bull-arrives 的版本');
  return version;
}
export function checkNotes(entry) {
  if (!/^###\s+\S/m.test(entry) || !/^[-*]\s+\S/m.test(entry))
    throw new Error('更新说明必须包含 ### 分组和 - 条目，否则应用更新窗口无法展示');
  if (/(?:TODO|待填写|No changelog entry)/i.test(entry)) throw new Error('更新说明仍有占位内容');
}
export function gitFiles(base, args) {
  return execFileSync('git', ['-C', base, ...args, '-z'], { encoding: 'utf8' }).split('\0').filter(Boolean);
}
function sourceIncludes(base) {
  const needed = new Set();
  function walk(folder) {
    for (const row of readdirSync(folder, { withFileTypes: true })) {
      const file = resolve(folder, row.name);
      if (row.isDirectory()) walk(file);
      else if (row.name.endsWith('.rs')) {
        for (const match of readFileSync(file, 'utf8').matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
          const resolved = resolve(dirname(file), match[1]);
          const name = relative(base, resolved).split(sep).join('/');
          if (name.startsWith('../') || name === '..') throw new Error('内嵌文件离开项目目录：' + name);
          needed.add(name);
        }
      }
    }
  }
  walk(resolve(base, 'src-tauri/src'));
  if (existsSync(resolve(base, 'research-assets.json'))) {
    needed.add('research-assets.json');
    for (const file of ['scripts/research-bundle.py','scripts/check-research-runtime.py','scripts/research-requirements.txt']) needed.add(file);
    for (const manifest of ['research/research-center-runner/registry.json','research/ashare-fundamental-2026-10-01/replay-sources.json']) {
      for (const row of Object.values(json(base, manifest).files)) {
        if (row.path.endsWith('.py')) needed.add(row.path);
      }
    }
  }
  return [...needed].sort();
}
export function checkRelease(base = root, { version, tracked = false } = {}) {
  const pkg = json(base, versionFiles[0]), lock = json(base, versionFiles[1]);
  const versions = [pkg.version, lock.version, lock.packages?.['']?.version,
    cargoVersion(read(base, versionFiles[2])), cargoVersion(read(base, versionFiles[3]), true),
    json(base, versionFiles[4]).version];
  if (!/^\d+\.\d+\.\d+$/.test(pkg.version) || versions.some(v => v !== pkg.version))
    throw new Error('五个版本文件不一致：' + versions.join(', '));
  if (version && version.replace(/^v/, '') !== pkg.version) throw new Error('标签版本与源码版本不一致');
  const notes = extractChangelog(read(base, 'CHANGELOG.md'), pkg.version);
  checkNotes(notes);
  const includes = sourceIncludes(base);
  for (const file of includes) {
    if (!lstatSync(resolve(base, file)).isFile()) throw new Error('内嵌文件不是普通文件：' + file);
  }
  const paths = gitFiles(base, ['ls-files', '--cached']);
  const cached = new Set(paths);
  const pendingDeletes = new Set(gitFiles(base, ['diff', '--name-only', '--diff-filter=D']));
  if (tracked) {
    for (const file of [...includes, ...versionFiles, 'CHANGELOG.md']) {
      if (!cached.has(file)) throw new Error('编译/发版必需文件未纳入Git：' + file);
    }
    for (const file of includes) {
      const input = execFileSync('git', ['-C', base, 'show', ':' + file], { maxBuffer: 20 * 1024 * 1024 });
      if (!input.equals(readFileSync(resolve(base, file))))
        throw new Error('内嵌文件与Git暂存字节不同，换行转换会破坏冻结指纹：' + file);
    }
  }
  for (const file of new Set([...paths, ...gitFiles(base, ['diff', '--cached', '--name-only', '--diff-filter=ACMR'])])) {
    // A local source cleanup may still be an unstaged deletion. CI/tracked
    // verification requires that deletion to be included in the Git input.
    if (pendingDeletes.has(file)) {
      if (tracked) throw new Error('删除的文件尚未纳入Git提交：' + file);
      continue;
    }
    if (/(^|\/)(?:\.env(?:\..*)?|data\/)|\.(?:pem|key|p12|pfx|db(?:-wal|-shm|-journal)?|sqlite(?:3)?|npz|npy|pkl|exe)$/i.test(file) && !file.endsWith('.env.example'))
      throw new Error('不能发布本地数据、模型二进制或密钥：' + file);
    if (statSync(resolve(base, file)).size > 20 * 1024 * 1024) throw new Error('提交文件超过20MiB，请审核并拆出源码：' + file);
  }
  return { version: pkg.version, tag: 'v' + pkg.version, embeddedFiles: includes, notes };
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    if (args.some(a => a !== '--tracked' && !/^v?\d+\.\d+\.\d+$/.test(a))) throw new Error('Usage: npm run release:check -- [vX.Y.Z] [--tracked]');
    const result = checkRelease(root, { version: args.find(a => a !== '--tracked'), tracked: args.includes('--tracked') });
    console.log('发版检查通过：' + result.tag + '，' + result.embeddedFiles.length + ' 个内嵌文件存在。' + (args.includes('--tracked') ? '必需文件已纳入Git。' : '发布前还须运行 --tracked 确认Git提交范围。'));
  } catch (error) { console.error('发版检查失败：' + error.message); process.exitCode = 1; }
}
