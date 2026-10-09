#!/usr/bin/env node
// 发布前校验草稿元数据与已下载的实际产物，生成SHA256清单；不发布Release。
import { readFileSync, writeFileSync, statSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, basename, dirname } from 'node:path';
import { tempDir } from './build-env.mjs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { root, checkRelease, json } from './check-release.mjs';

export function verifyRelease(release, folder, { tag, repository, notes, published = false }) {
  if (release.tag_name !== tag || release.prerelease) throw new Error('Release标签/正式版本状态不符');
  if (release.draft !== !published) throw new Error(published ? 'Release尚未正式发布' : '只允许核验草稿，拒绝覆盖已发布版本');
  if (release.body?.trim() !== notes.trim()) throw new Error('Release更新说明与源码CHANGELOG不一致');
  const version = tag.replace(/^v/, '');
  const assets = release.assets ?? [];
  const names = new Set(assets.map(a => a.name));
  if (names.size !== assets.length) throw new Error('Release有重复文件名');
  const required = ['BullArrives_' + version + '_x64-setup.exe', 'BullArrives_' + version + '_x64-portable.zip', 'latest.json'];
  for (const file of required) if (!names.has(file)) throw new Error('缺少产物：' + file);
  if (!assets.some(a => a.name.endsWith('.dmg')))
    throw new Error('macOS产物不完整');
  const lines = [];
  for (const asset of assets) {
    if (basename(asset.name) !== asset.name || asset.name.includes('\\') || !asset.size || asset.state !== 'uploaded') throw new Error('产物名称、大小或上传状态无效');
    const file = resolve(folder, asset.name);
    if (statSync(file).size !== asset.size) throw new Error('产物下载大小不符：' + asset.name);
    const digest = createHash('sha256').update(readFileSync(file)).digest('hex');
    if (asset.digest && asset.digest !== 'sha256:' + digest) throw new Error('GitHub产物摘要不符：' + asset.name);
    if (asset.name !== 'SHA256SUMS.txt') lines.push(digest + '  ' + asset.name);
  }
  const updater = JSON.parse(readFileSync(resolve(folder, 'latest.json'), 'utf8'));
  if (updater.version?.replace(/^v/, '') !== version || updater.notes?.trim() !== notes.trim()) throw new Error('latest.json版本或更新说明不符');
  if (!Number.isFinite(Date.parse(updater.pub_date))) throw new Error('latest.json缺少有效发布日期');
  for (const key of ['windows-x86_64', 'darwin-x86_64', 'darwin-aarch64']) {
    const item = updater.platforms?.[key];
    if (!item || typeof item.signature !== 'string' || item.signature.trim().length < 40) throw new Error('更新平台或签名内容缺失：' + key);
    const url = new URL(item.url);
    const prefix = '/' + repository + '/releases/download/' + tag + '/';
    if (url.origin !== 'https://github.com' || !decodeURIComponent(url.pathname).startsWith(prefix) || url.search || url.hash)
      throw new Error('更新地址不属于此仓库/版本：' + key);
    const name = decodeURIComponent(url.pathname).slice(prefix.length);
    if (!names.has(name) || !names.has(name + '.sig')) throw new Error('更新文件或签名附件缺失：' + key);
    if (readFileSync(resolve(folder, name + '.sig'), 'utf8').trim() !== item.signature.trim()) throw new Error('更新签名与附件不一致：' + key);
    if (key === 'windows-x86_64' && name !== required[0]) throw new Error('Windows更新未指向本版NSIS安装包');
  }
  return lines.sort().join('\n') + '\n';
}

// 复用原生minisign校验器，不自行实现加密算法；CI发布job安装该工具。
export function verifyUpdaterSignatures(folder, encodedPublicKey) {
  const updater = JSON.parse(readFileSync(resolve(folder, 'latest.json'), 'utf8'));
  const work = mkdtempSync(resolve(tempDir, 'bull-arrives-signature-'));
  try {
    const publicFile = resolve(work, 'updater.pub');
    writeFileSync(publicFile, Buffer.from(encodedPublicKey, 'base64'));
    const verified = new Set();
    for (const item of Object.values(updater.platforms)) {
      const name = decodeURIComponent(new URL(item.url).pathname.split('/').pop());
      if (basename(name) !== name || name.includes('\\')) throw new Error('签名产物名称不安全');
      if (verified.has(name)) continue;
      const signatureFile = resolve(work, 'artifact.minisig');
      writeFileSync(signatureFile, Buffer.from(item.signature, 'base64'));
      // minisign 默认兼容 Tauri 非预哈希签名，同时校验 Ed25519 与可信注释。
      execFileSync('minisign', ['-Vm', resolve(folder, name), '-p', publicFile, '-x', signatureFile, '-q'], { stdio: 'pipe', windowsHide: true });
      verified.add(name);
    }
    return verified.size;
  } finally {
    if (dirname(work) === tempDir) rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [metadata, folder, flag] = process.argv.slice(2);
    if (!metadata || !folder || (flag && flag !== '--published')) throw new Error('Usage: node scripts/verify-release.mjs <release.json> <asset-dir> [--published]');
    const current = checkRelease(root, { version: process.env.RELEASE_TAG, tracked: true });
    if (!process.env.GITHUB_REPOSITORY) throw new Error('缺少GITHUB_REPOSITORY');
    const content = verifyRelease(JSON.parse(readFileSync(metadata, 'utf8')), folder, {
      tag: current.tag, repository: process.env.GITHUB_REPOSITORY, notes: current.notes, published: flag === '--published',
    });
    verifyUpdaterSignatures(folder, json(root, 'src-tauri/tauri.conf.json').plugins.updater.pubkey);
    if (flag && readFileSync(resolve(folder, 'SHA256SUMS.txt'), 'utf8') !== content) throw new Error('SHA256SUMS清单与实际产物不符');
    if (!flag) writeFileSync(resolve(folder, 'SHA256SUMS.txt'), content);
    console.log('Release实际产物、版本、更新说明和签名附件一致；' + (flag ? '已正式发布。' : '可进入最终发布步骤。'));
  } catch (error) { console.error('Release核验失败：' + error.message); process.exitCode = 1; }
}
