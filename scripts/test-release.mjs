#!/usr/bin/env node
// 一份无依赖、无网络的可运行检查；只修改统一构建临时目录中的假仓库。
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tempDir } from './build-env.mjs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { checkRelease, checkNotes, versionFiles } from './check-release.mjs';
import { extractChangelog } from './extract-changelog.mjs';
import { prepareRelease } from './prepare-release.mjs';
import { verifyRelease } from './verify-release.mjs';

const work = mkdtempSync(resolve(tempDir, 'bull-arrives-release-check-'));
const put = (name, value) => { const file = resolve(work, name); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, value); };
const git = (...args) => execFileSync('git', ['-C', work, ...args], { encoding: 'utf8', stdio: 'pipe' });
const snapshot = () => versionFiles.concat('CHANGELOG.md').map(name => readFileSync(resolve(work, name), 'utf8'));
try {
  put('package.json', JSON.stringify({ name: 'bull-arrives', version: '2.2.6' }));
  put('package-lock.json', JSON.stringify({ version: '2.2.6', packages: { '': { version: '2.2.6' } } }));
  put('src-tauri/tauri.conf.json', JSON.stringify({ version: '2.2.6' }));
  put('src-tauri/Cargo.toml', '[package]\nname = "bull-arrives"\nversion = "2.2.6"\n\n[dependencies]\n');
  put('src-tauri/Cargo.lock', '[[package]]\nname = "bull-arrives"\nversion = "2.2.6"\n\n[[package]]\nname = "other"\nversion = "8.0.0"\n');
  put('src-tauri/src/lib.rs', 'const F: &str = include_str!("../../research/fixture.json");');
  put('research/fixture.json', '{}');
  put('CHANGELOG.md', '# Changelog\n\n## Unreleased\n\n### 新增\n- 本次测试功能\n\n## v2.2.6 (2026-09-27)\n\n### 修复\n- 历史条目\n\n## v2.2.5\n\n### 修复\n- 更早条目\n');
  git('init', '--quiet');
  assert.equal(checkRelease(work).version, '2.2.6');
  assert.throws(() => checkRelease(work, { version: 'v2.2.7' }), /标签版本/);
  assert.throws(() => checkRelease(work, { tracked: true }), /未纳入Git/);
  git('add', '--', ...versionFiles, 'CHANGELOG.md', 'src-tauri/src/lib.rs', 'research/fixture.json');
  assert.equal(checkRelease(work, { tracked: true }).embeddedFiles.length, 1);
  put('obsolete.ts', '// retired source'); git('add', '--', 'obsolete.ts');
  rmSync(resolve(work, 'obsolete.ts'));
  assert.doesNotThrow(() => checkRelease(work));
  assert.throws(() => checkRelease(work, { tracked: true }), /删除的文件尚未纳入Git/);
  git('add', '-u', '--', 'obsolete.ts');
  assert.doesNotThrow(() => checkRelease(work, { tracked: true }));
  put('research/fixture.json', ''); rmSync(resolve(work, 'research/fixture.json'));
  assert.throws(() => checkRelease(work)); put('research/fixture.json', '{}');
  assert.throws(() => checkNotes('### 修复\n- TODO'), /占位/);
  assert.throws(() => checkNotes('- 无分组'), /分组/);
  assert.throws(() => extractChangelog('## v2.2.6-rc\n### 修复\n- x', '2.2.6'), /找不到/);
  const extractor = resolve(dirname(fileURLToPath(import.meta.url)), 'extract-changelog.mjs');
  // GitHub output实际使用本仓库公告，但只往临时文件写，不修改源码。
  const outputFile = resolve(work, 'github-output');
  execFileSync(process.execPath, [extractor, '2.2.6', '--github-output'], { env: { ...process.env, GITHUB_OUTPUT: outputFile }, stdio: 'pipe' });
  assert.match(readFileSync(outputFile, 'utf8'), /^body<<notes_/);
  assert.ok(readFileSync(outputFile, 'utf8').includes('## v2.2.6'));
  const originals = snapshot();
  assert.throws(() => prepareRelease(work, '2.2.5'), /高于/); assert.deepEqual(snapshot(), originals);
  assert.throws(() => prepareRelease(work, '2.02.7'), /格式|稳定版/); assert.deepEqual(snapshot(), originals);
  const prepared = prepareRelease(work, 'v2.2.7');
  assert.equal(prepared.tag, 'v2.2.7');
  assert.equal(checkRelease(work, { version: 'v2.2.7', tracked: true }).version, '2.2.7');
  assert.match(readFileSync(resolve(work, 'src-tauri/Cargo.lock'), 'utf8'), /name = "other"\nversion = "8.0.0"/);
  assert.match(readFileSync(resolve(work, 'CHANGELOG.md'), 'utf8'), /## v2\.2\.7 \(\d{4}-\d{2}-\d{2}\)/);
  assert.ok(extractChangelog(readFileSync(resolve(work, 'CHANGELOG.md'), 'utf8'), '2.2.7').includes('本次测试功能'));
  assert.ok(extractChangelog(readFileSync(resolve(work, 'CHANGELOG.md'), 'utf8'), '2.2.6').includes('历史条目'));
  const preparedSnapshot = snapshot();
  assert.throws(() => prepareRelease(work, '2.2.8'), /Unreleased/); assert.deepEqual(snapshot(), preparedSnapshot);
  put('.env', 'synthetic-secret'); git('add', '--', '.env');
  assert.throws(() => checkRelease(work), /密钥/); git('reset', '--quiet', '--', '.env');
  put('oversized.json', Buffer.alloc(21 * 1024 * 1024)); git('add', '--', 'oversized.json');
  assert.throws(() => checkRelease(work), /20MiB/); git('reset', '--quiet', '--', 'oversized.json');

  const folder = resolve(work, 'assets'); mkdirSync(folder);
  const notes = '## v2.2.7\n\n### 修复\n- 本次修复';
  const repository = 'example/bull-arrives', tag = 'v2.2.7';
  const files = ['BullArrives_2.2.7_x64-setup.exe', 'BullArrives_2.2.7_x64-portable.zip', 'BullArrives.dmg', 'BullArrives.deb', 'BullArrives.AppImage', 'BullArrives.app.tar.gz'];
  const signature = Buffer.from('synthetic-minisign-placeholder-for-boundary-test-only').toString('base64');
  const updater = { version: '2.2.7', notes, pub_date: '2026-10-02T00:00:00Z', platforms: {} };
  for (const [key, name] of [['windows-x86_64', files[0]], ['darwin-x86_64', files[5]], ['darwin-aarch64', files[5]], ['linux-x86_64', files[4]]])
    updater.platforms[key] = { signature, url: 'https://github.com/' + repository + '/releases/download/' + tag + '/' + name };
  for (const file of files) writeFileSync(resolve(folder, file), 'synthetic-' + file);
  for (const file of [files[0], files[4], files[5]]) { writeFileSync(resolve(folder, file + '.sig'), signature); files.push(file + '.sig'); }
  writeFileSync(resolve(folder, 'latest.json'), JSON.stringify(updater)); files.push('latest.json');
  const release = { tag_name: tag, prerelease: false, draft: true, body: notes, assets: files.map(name => {
    const content = readFileSync(resolve(folder, name)); return { name, size: content.length, state: 'uploaded', digest: 'sha256:' + createHash('sha256').update(content).digest('hex') };
  }) };
  const options = { tag, repository, notes };
  assert.match(verifyRelease(release, folder, options), /latest\.json/);
  assert.throws(() => verifyRelease({ ...release, draft: false }, folder, options), /草稿/);
  assert.doesNotThrow(() => verifyRelease({ ...release, draft: false }, folder, { ...options, published: true }));
  assert.throws(() => verifyRelease({ ...release, body: 'wrong notes' }, folder, options), /说明/);
  assert.throws(() => verifyRelease({ ...release, assets: release.assets.filter(a => !a.name.endsWith('portable.zip')) }, folder, options), /缺少/);
  assert.throws(() => verifyRelease({ ...release, assets: release.assets.concat(release.assets[0]) }, folder, options), /重复/);
  updater.platforms['windows-x86_64'].url = 'https://other.example/installer.exe';
  writeFileSync(resolve(folder, 'latest.json'), JSON.stringify(updater));
  const changedAsset = release.assets.find(a => a.name === 'latest.json');
  const changedBytes = readFileSync(resolve(folder, 'latest.json')); changedAsset.size = changedBytes.length; changedAsset.digest = 'sha256:' + createHash('sha256').update(changedBytes).digest('hex');
  assert.throws(() => verifyRelease(release, folder, options), /地址/);
  console.log('发版自检通过：版本同步/拒绝降版、失败不改文件、Git输入、缺文件、私密/大文件、公告边界和Release产物防错。合成签名只测附件边界，真实密码学校验由CI原生minisign执行。');
} finally {
  if (dirname(work) === tempDir) rmSync(work, { recursive: true, force: true });
}
