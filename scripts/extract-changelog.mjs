#!/usr/bin/env node
import { readFileSync, appendFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export function extractChangelog(content, version) {
  const normalized = version.replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+$/.test(normalized)) throw new Error('版本格式必须为 vX.Y.Z 或 X.Y.Z');
  const escaped = normalized.replaceAll('.', '\\.');
  const match = content.match(new RegExp('^##\\s+v?' + escaped + '(?=\\s|$)', 'm'));
  if (!match) throw new Error('CHANGELOG.md 找不到 v' + normalized);
  const rest = content.slice(match.index + match[0].length);
  const next = rest.search(/^##\s+/m);
  return content.slice(match.index, next < 0 ? undefined : match.index + match[0].length + next).trim();
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (!process.argv[2]) throw new Error('Usage: node scripts/extract-changelog.mjs <version>');
    const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
    const entry = extractChangelog(readFileSync(resolve(root, 'CHANGELOG.md'), 'utf8'), process.argv[2]);
    if (process.argv[3]) {
      if (process.argv[3] !== '--github-output' || process.argv[4] || !process.env.GITHUB_OUTPUT) throw new Error('GitHub输出参数或环境无效');
      const delimiter = 'notes_' + randomUUID();
      appendFileSync(process.env.GITHUB_OUTPUT, 'body<<' + delimiter + '\n' + entry + '\n' + delimiter + '\n');
    }
    process.stdout.write(entry + '\n');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
