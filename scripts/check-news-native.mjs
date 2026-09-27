import assert from 'node:assert/strict';

const port = Number(process.argv[2]);
assert.ok(Number.isInteger(port) && port > 0, '用法：node scripts/check-news-native.mjs <WebView2-CDP-端口>');
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const main = pages.find(page => page.url === 'http://tauri.localhost/');
assert.ok(main, '主 WebView 不存在');
const ws = new WebSocket(main.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.addEventListener('open', resolve, { once: true });
  ws.addEventListener('error', reject, { once: true });
});
let nextId = 0;
const pending = new Map();
ws.addEventListener('message', event => {
  const response = JSON.parse(event.data);
  const callback = pending.get(response.id);
  if (!callback) return;
  pending.delete(response.id);
  response.error ? callback.reject(new Error(JSON.stringify(response.error))) : callback.resolve(response.result);
});
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const response = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (response.exceptionDetails) throw new Error(JSON.stringify(response.exceptionDetails));
  return response.result.value;
}
async function until(expression, timeoutMs = 10_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await evaluate(expression)) return;
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`等待超时：${expression}`);
}
const settings = () => evaluate(`window.__TAURI_INTERNALS__.invoke('get_settings')`);
try {
  await until(`!!document.querySelector('.alert-history-button') && !!window.__TAURI_INTERNALS__`);
  const initial = await settings();
  assert.ok(['0', '1'].includes(initial.news_notifications_enabled));
  assert.equal(initial.news_notification_mode, 'direct');
  console.log('原生隔离设置：资讯', initial.news_notifications_enabled, '，直接通知。');

  await evaluate(`document.querySelector('.alert-history-button').click()`);
  await until(`document.querySelectorAll('.news-mode button').length === 2`);
  assert.equal(await evaluate(`document.querySelectorAll('.news-mode button')[0].getAttribute('aria-pressed')`), 'true');
  await evaluate(`document.querySelectorAll('.news-mode button')[1].click()`);
  await until(`document.querySelectorAll('.news-mode button')[1].getAttribute('aria-pressed') === 'true'`);
  assert.equal((await settings()).news_notification_mode, 'ai');
  await evaluate(`document.querySelectorAll('.news-mode button')[0].click()`);
  await until(`document.querySelectorAll('.news-mode button')[0].getAttribute('aria-pressed') === 'true'`);
  assert.equal((await settings()).news_notification_mode, 'direct');
  console.log('模式按钮 → 真正 Tauri IPC → SQLite 持久化：AI、直接通知均通过。');

  if (initial.news_notifications_enabled === '0') await evaluate(`document.querySelector('.news-intro > button').click()`);
  await until(`document.querySelector('.news-intro > button')?.innerText.includes('暂停资讯采集')`);
  assert.equal((await settings()).news_notifications_enabled, '1');
  const started = Date.now();
  while (Date.now() - started < 95_000) {
    const current = await settings();
    if (current.news_scope_version === '2' && current.news_flash_initialized === '1' && current.news_announcement_initialized === '1') break;
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  const after = await settings();
  assert.equal(after.news_scope_version, '2');
  assert.equal(after.news_flash_initialized, '1');
  assert.equal(after.news_announcement_initialized, '1');
  const initialArchive = await evaluate(`window.__TAURI_INTERNALS__.invoke('get_news_archive')`);
  assert.equal(initialArchive.filter(row => row.signal_kind === 'news').length, 0, '首次采集不应补推旧消息');
  console.log('原生后台首次采集：全市场快讯与公告水位均建立，未补推历史资讯。');

  await evaluate(`window.__TAURI_INTERNALS__.invoke('set_setting', {key:'notification_desktop_always',value:'1'})`);
  const delivery = await evaluate(`window.__TAURI_INTERNALS__.invoke('test_notification')`);
  assert.ok(['accepted', 'failed'].includes(delivery.native));
  assert.equal(delivery.desktop, 'queued');
  console.log('原生通知调用：', JSON.stringify(delivery));
  const toastStart = Date.now();
  let toast;
  while (Date.now() - toastStart < 8000) {
    const currentPages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    toast = currentPages.find(page => page.url.endsWith('/toast.html'));
    if (toast) break;
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  assert.ok(toast, '桌面提醒 WebView 未创建');
  console.log('应用桌面提醒 WebView 已创建；Windows 通知结果见上。');
} finally {
  ws.close();
}
