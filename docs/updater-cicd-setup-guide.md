# 自动更新 CI/CD 配置指南

> Bull Arrives 仓库已设为 Public，更新检测和下载均直接使用 GitHub Releases，无需镜像仓库。

---

## 架构

```
bull-arrives (Public Repo)
┌──────────────────────────────────────────────┐
│ CI (v* tag)                                  │
│  1. build (Win/Mac/Linux 矩阵)               │
│  2. release: sign + latest.json + 创建 Release │
└──────────────────────┬───────────────────────┘
                       │ GitHub Releases API (公开，无需认证)
                       ▼
               Bull Arrives App
               (检查更新 → 下载 → 安装)
```

---

## ⚠️ 最容易踩的坑：CHANGELOG 必须先于 tag

> **v1.1.0 就栽在这里** —— 应用内点更新，弹窗「更新内容」显示的是"暂无更新说明"。

应用内更新说明的**唯一**来源是 `latest.json` 的 `notes` 字段，链路如下：

```
CHANGELOG.md（tag 指向的那个 commit）
  → scripts/extract-changelog.mjs
  → tauri-action releaseBody
  → latest.json 的 notes
  → 应用端 update.body → UpdateDialog.vue 渲染
```

**tag 打早了会怎样**：CI checkout 的是 tag 对应的 commit，那个 commit 里还没有新版本的 CHANGELOG 条目 → 提取失败 → `notes` 被写成占位文案 `No changelog entry for vX.Y.Z` → 用户端显示"暂无更新说明"。

**最坑的一点**：GitHub Release 页面上显示的正文（body）和 `latest.json` 的 `notes` 是两回事。事后手动编辑 Release 描述**不会**同步到 `latest.json`，只会让人误以为已经修好了。

### 正确顺序

```bash
# 1. 先写 CHANGELOG.md，提交
git add CHANGELOG.md
git commit -m "docs: CHANGELOG for v1.2.0"

# 2. 再打 tag
git tag v1.2.0
git push origin main
git push origin v1.2.0
```

### 条目结构要求

应用端 `UpdateDialog.vue` 的 `renderMarkdownLines` 解析很严格，三者缺一都会渲染成空：

| 元素 | 要求 | 缺失后果 |
|------|------|---------|
| 首行 | `## vX.Y.Z`（可带日期） | 整段被跳过 |
| 分组 | `### 分组名` | 条目被丢弃 |
| 条目 | `- 内容` | 该分组为空 |

标准写法：

```markdown
## v1.2.0 (2026-06-21)

### 新增

- 版本更新检测及自动更新机制
- 托盘菜单"检查更新"入口

### 修复

- 修复已知问题，优化性能
```

> CI 的 `Extract changelog for release body` 步骤已对上述三点做校验，任一不满足会直接中断发布并打印现有版本条目，不会再静默塞占位文案。

---

## 第一步：生成签名密钥对

在项目根目录执行：

```bash
npx tauri signer generate \
  -w ~/.tauri/bull-arrives.key \
  -p "<你的私钥密码>" \
  --ci \
  --force
```

### 密钥文件说明

| 文件 | 用途 | 存放位置 |
|------|------|---------|
| `~/.tauri/bull-arrives.key` | 私钥（加密） | 本地 + GitHub Secret |
| `~/.tauri/bull-arrives.key.pub` | 公钥 | 已写入 `src-tauri/tauri.conf.json` |

### 查看私钥内容

**Git Bash:**
```bash
cat ~/.tauri/bull-arrives.key
```

复制输出的全部内容，下一步要用。

---

## 第二步：添加 GitHub Secrets

仓库已 Public，只需 2 个 Secret：

打开 `https://github.com/ChineseCanFly-wxy/bull-arrives/settings/secrets/actions`，点 **New repository secret**：

### 2.1 添加 TAURI_SIGNING_PRIVATE_KEY

| 字段 | 值 |
|------|-----|
| **Name** | `TAURI_SIGNING_PRIVATE_KEY` |
| **Secret** | 粘贴第一步 `cat ~/.tauri/bull-arrives.key` 输出的全部内容 |

### 2.2 添加 TAURI_SIGNING_PRIVATE_KEY_PASSWORD

| 字段 | 值 |
|------|-----|
| **Name** | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| **Secret** | 上一步设置的私钥密码 |

### 验证 Secrets 列表

配置完成后页面应显示：

| Name | 状态 |
|------|------|
| `TAURI_SIGNING_PRIVATE_KEY` | ✅ |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | ✅ |

> 不需要 `PUBLIC_REPO_PAT`——仓库公开后，Release 创建使用内置 `GITHUB_TOKEN`，无需额外配置。

---

## 第三步：验证配置

### 3.1 更新 CHANGELOG.md

在项目根目录的 `CHANGELOG.md` 中添加新版本记录：

```markdown
## v1.2.0 (2026-06-21)
### Added
- 版本更新检测及自动更新机制
- 启动时自动检查更新，交易时段智能抑制弹窗
- 更新对话框展示完整 CHANGELOG
- 托盘菜单"检查更新"入口
### Fixed
- 修复已知问题，优化性能
```

### 3.2 推送 tag 触发构建

```bash
git add CHANGELOG.md
git commit -m "docs: update CHANGELOG for v1.2.0"
git tag v1.2.0
git push origin main
git push origin v1.2.0
```

### 3.3 监控 CI 运行

打开 `https://github.com/ChineseCanFly-wxy/bull-arrives/actions`，查看 Release workflow：

构建流程（单个 job `publish`，Windows / macOS / Linux 三平台矩阵并行，各 12 个 step）：

1. **环境准备** — checkout、Node 22、Rust stable、依赖缓存（Linux 额外装系统依赖，macOS 额外补两个 target）
2. **提取 CHANGELOG** — 见上方 ⚠️ 章节，校验不通过会中断发布
3. **tauri-action** — 构建 → 签名 → 生成 `latest.json` → 创建/更新 Release。三平台各跑一次，`latest.json` 会自动合并各平台条目
4. **Windows 专属** — 打便携版 `BullArrives_<版本>_x64-portable.zip` 并上传

### 3.4 验证 Release

打开 `https://github.com/ChineseCanFly-wxy/bull-arrives/releases`，确认：
- 新 Release `v1.2.0` 已创建
- 安装包文件（`.exe`/`.msi`/`.dmg` 等）已上传
- `latest.json` 已上传

### 3.5 验证 latest.json

下载 `latest.json`，内容格式应为：

```json
{
  "version": "1.2.0",
  "notes": "### Added\n- 版本更新检测...",
  "pub_date": "2026-06-21T...",
  "platforms": {
    "windows-x86_64": {
      "signature": "...",
      "url": "https://github.com/ChineseCanFly-wxy/bull-arrives/releases/download/v1.2.0/..."
    }
  }
}
```

---

## 故障排查

### CI sign 步骤失败

| 错误信息 | 可能原因 | 解决 |
|---------|---------|------|
| `A public key has been found, but no private key` | `TAURI_SIGNING_PRIVATE_KEY` 未设置或名称不对 | 检查 Secret Name 是否完全一致 |
| `Bad password` | 密钥密码错误 | 确认 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 值与生成时一致 |

### 应用端检查不到更新

| 现象 | 可能原因 | 解决 |
|------|---------|------|
| 始终显示"已是最新版本" | `latest.json` 未上传到 Release | 检查 Release Assets 是否包含 `latest.json` |
| `endpoint did not respond with a successful status code` | Release 不存在或仓库非 Public | 确认仓库为 Public，Release 已创建 |
| 下载失败 | 签名验证不通过 | 确认 `tauri.conf.json` 中 pubkey 与 CI 私钥匹配 |
| 能检测到更新，但弹窗「更新内容」显示"暂无更新说明" | `latest.json` 的 `notes` 是占位文案 —— tag 早于 CHANGELOG 提交 | 见上方 ⚠️ 章节。**已经发出去的版本不用重发**：下载 `latest.json` 改好 `notes` 后用 `gh release upload <tag> latest.json --clobber` 覆盖即可（`notes` 不参与签名，签名在 `platforms.*.signature`，只改 `notes` 不影响校验） |

### 验证更新说明能否正常渲染

不装应用也能自查 —— 把 `notes` 喂给应用端同款解析逻辑：

```bash
node -e "
const notes = require('fs').readFileSync('latest.json','utf8');
const raw = JSON.parse(notes).notes;
let passed = false, sections = 0, items = 0;
for (const line of raw.split('\n')) {
  if (!passed) { if (/^##\s+v?\d+\.\d+\.\d+/.test(line)) passed = true; continue; }
  if (/^###\s+/.test(line)) sections++;
  else if (/^[-*]\s+/.test(line)) items++;
}
console.log('首行命中:', passed, '| 分组:', sections, '| 条目:', items);
console.log(sections && items ? '✅ 用户端可正常展示' : '❌ 用户端显示：暂无更新说明');
"
```

---

## 后续日常发布流程

```bash
# 1. 更新版本号
# 编辑 src-tauri/Cargo.toml 和 src-tauri/tauri.conf.json 的 version

# 2. 更新 CHANGELOG.md

# 3. 提交 + 打 tag
git add .
git commit -m "chore: bump version to v1.x.0"
git tag v1.x.0
git push origin main
git push origin v1.x.0
```

推送 tag 后 CI 自动构建，构建完成后应用即可检测到更新。

---

## 配置清单

| 序号 | 操作 | 位置 | 状态 |
|------|------|------|------|
| 1 | 生成签名密钥对 | 本地 `npx tauri signer generate` | ☐ |
| 2 | 添加 Secret `TAURI_SIGNING_PRIVATE_KEY` | Settings → Secrets → Actions | ☐ |
| 3 | 添加 Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Settings → Secrets → Actions | ☐ |
| 4 | 更新 `CHANGELOG.md` | 项目根目录 | ☐ |
| 5 | 推送 `v*` tag 触发构建 | `git tag v1.2.0 && git push origin v1.2.0` | ☐ |
| 6 | 验证 Release 和 `latest.json` | `https://github.com/ChineseCanFly-wxy/bull-arrives/releases` | ☐ |
