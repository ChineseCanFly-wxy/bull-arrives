import assert from 'node:assert/strict';

const port = Number(process.argv[2]);
assert.ok(Number.isInteger(port) && port > 0, '用法：node scripts/check-stock-detail.mjs <WebView2-CDP-端口>');
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const main = pages.find(page => page.url === 'http://tauri.localhost/');
assert.ok(main, '未找到主窗口');
const ws = new WebSocket(main.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.addEventListener('open', resolve, { once: true });
  ws.addEventListener('error', reject, { once: true });
});
let nextId = 0;
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const receive = event => {
      const result = JSON.parse(event.data);
      if (result.id !== id) return;
      ws.removeEventListener('message', receive);
      result.error ? reject(new Error(JSON.stringify(result.error))) : resolve(result.result);
    };
    ws.addEventListener('message', receive);
    ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
  return result.result.value;
}
const original = await evaluate('({width: innerWidth, height: innerHeight})');
try {
  await evaluate(`window.__TAURI_INTERNALS__.invoke('set_main_window_size', { config: { width: 640, height: 480, remember: false } })`);
  await new Promise(resolve => setTimeout(resolve, 300));
  await evaluate(`document.querySelector('tr[aria-label*="单击查看详情"]')?.click()`);
  await new Promise(resolve => setTimeout(resolve, 300));
  const layout = await evaluate(`(() => {
    const detail = document.querySelector('.stock-detail');
    const chart = detail?.querySelector('.chart-container');
    if (!detail || !chart) return null;
    detail.scrollTop = detail.scrollHeight;
    return {
      overflow: getComputedStyle(detail).overflowY,
      scrollHeight: detail.scrollHeight,
      clientHeight: detail.clientHeight,
      scrollTop: detail.scrollTop,
      chartBottom: chart.getBoundingClientRect().bottom,
      detailBottom: detail.getBoundingClientRect().bottom,
    };
  })()`);
  assert.ok(layout, '未打开自选股行情');
  assert.equal(layout.overflow, 'auto');
  assert.ok(layout.scrollHeight > layout.clientHeight, '窄窗口应允许滚动详情');
  assert.ok(layout.scrollTop > 0, '详情无法滚动到底部');
  assert.ok(layout.chartBottom <= layout.detailBottom + 1, '滚动到底后图表仍被裁切');
  console.log(`详情可滚动 ${layout.scrollTop}px，图表底部 ${Math.round(layout.chartBottom)}px`);
} finally {
  await evaluate(`window.__TAURI_INTERNALS__.invoke('set_main_window_size', { config: { width: ${original.width}, height: ${original.height}, remember: false } })`);
  ws.close();
}
