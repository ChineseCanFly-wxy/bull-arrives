#!/usr/bin/env node
// 只准备工作区版本和说明；提交、推送、标签由发版执行者审核后执行。
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { root, read, json, checkRelease, checkNotes, versionFiles } from './check-release.mjs';

export function prepareRelease(base = root, requested) {
  const current = checkRelease(base).version;
  const parts = current.split('.').map(Number);
  const version = requested?.replace(/^v/, '') ?? parts.slice(0, 2).concat(parts[2] + 1).join('.');
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error('版本必须为稳定版 X.Y.Z');
  const next = version.split('.').map(Number);
  const difference = next.map((v, i) => v - parts[i]).find(v => v !== 0) ?? 0;
  if (difference <= 0) throw new Error('新版本必须高于当前版本');
  const tag = 'v' + version;
  if (execFileSync('git', ['-C', base, 'tag', '--list', tag], { encoding: 'utf8' }).trim()) throw new Error('本地已存在标签 ' + tag);
  const old = new Map([...versionFiles, 'CHANGELOG.md'].map(name => [name, read(base, name)]));
  const changelog = old.get('CHANGELOG.md');
  if (new RegExp('^##\\s+v?' + version.replaceAll('.', '\\.') + '(?=\\s|$)', 'm').test(changelog)) throw new Error('CHANGELOG已存在新版本，拒绝重复准备');
  const match = /^##\s+Unreleased\s*\r?$/m.exec(changelog);
  if (!match) throw new Error('请先在 CHANGELOG 的 ## Unreleased 写好本次更新');
  const rest = changelog.slice(match.index + match[0].length);
  const stop = rest.search(/^##\s+/m);
  const body = rest.slice(0, stop < 0 ? undefined : stop).trim();
  checkNotes(body);
  const date = new Intl.DateTimeFormat('sv-SE', { timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit' }).format(new Date());
  const changed = new Map(old);
  const pkg = json(base, 'package.json'); pkg.version = version;
  const lock = json(base, 'package-lock.json'); lock.version = version; lock.packages[''].version = version;
  const config = json(base, 'src-tauri/tauri.conf.json'); config.version = version;
  for (const [name, value] of [['package.json', pkg], ['package-lock.json', lock], ['src-tauri/tauri.conf.json', config]])
    changed.set(name, JSON.stringify(value, null, 2) + '\n');
  for (const [name, expression] of [
    ['src-tauri/Cargo.toml', /(\[package\][\s\S]*?^version\s*=\s*")[^"]+("\s*$)/m],
    ['src-tauri/Cargo.lock', /(\[\[package\]\]\s*\r?\nname\s*=\s*"bull-arrives"\s*\r?\nversion\s*=\s*")[^"]+("\s*$)/m],
  ]) {
    if (!expression.test(old.get(name))) throw new Error('无法精确定位应用版本：' + name);
    changed.set(name, old.get(name).replace(expression, (_, head, tail) => head + version + tail));
  }
  changed.set('CHANGELOG.md', changelog.slice(0, match.index) + '## v' + version + ' (' + date + ')\n\n' + body + '\n\n' + (stop < 0 ? '' : rest.slice(stop)));
  try {
    for (const [name, content] of changed) writeFileSync(resolve(base, name), content);
    checkRelease(base, { version });
  } catch (error) {
    for (const [name, content] of old) writeFileSync(resolve(base, name), content);
    throw error;
  }
  return { version, tag, files: [...changed.keys()] };
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length > 3) throw new Error('Usage: npm run release:prepare -- [X.Y.Z]');
    const result = prepareRelease(root, process.argv[2]);
    console.log('已准备 ' + result.tag + '；仅修改版本与CHANGELOG，尚未暂存/提交/推送。审核后运行 release:check -- ' + result.tag + ' --tracked。');
  } catch (error) { console.error('准备发版失败：' + error.message); process.exitCode = 1; }
}
