import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';

const port = Number(process.argv[2]);
if (!Number.isInteger(port) || port < 1) {
  throw new Error('用法：node scripts/check-responsive.mjs <WebView2-CDP-端口>');
}
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const main = pages.find(page => page.url === 'http://tauri.localhost/');
assert.ok(main, '未找到主窗口的 WebView');
const ws = new WebSocket(main.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.addEventListener('open', resolve, { once: true });
  ws.addEventListener('error', reject, { once: true });
});
let nextId = 0;
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const onMessage = event => {
      const response = JSON.parse(event.data);
      if (response.id !== id) return;
      ws.removeEventListener('message', onMessage);
      if (response.error) reject(new Error(JSON.stringify(response.error)));
      else resolve(response.result);
    };
    ws.addEventListener('message', onMessage);
    ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
  return result.result.value;
}
const pause = () => new Promise(resolve => setTimeout(resolve, 240));
const evaluateLayout = `JSON.stringify((() => {
  const rect = element => element?.getBoundingClientRect().toJSON();
  const card = document.querySelector('.settings-modal');
  const content = document.querySelector('.settings-content');
  return {
    size: [innerWidth, innerHeight], style: document.documentElement.dataset.style,
    theme: document.documentElement.dataset.theme,
    rootDisplay: getComputedStyle(document.documentElement).display,
    card: rect(card),
    close: rect(card?.querySelector('button[aria-label="close"]')),
    footer: rect(card?.querySelector('.done-btn')),
    shell: rect(card?.querySelector('.settings-shell')),
    body: rect(content),
    scroll: [content?.scrollHeight, content?.clientHeight]
  };
})())`;
function verify(layout) {
  const [width, height] = layout.size;
  assert.notEqual(layout.rootDisplay, 'none');
  for (const [name, box] of Object.entries({ card: layout.card, close: layout.close, footer: layout.footer })) {
    assert.ok(box && box.width > 0 && box.height > 0, `${name} 未渲染`);
    assert.ok(box.left >= -1 && box.top >= -1 && box.right <= width + 1 && box.bottom <= height + 1,
      `${name} 超出窗口 ${width}×${height}: ${JSON.stringify(box)}`);
  }
  assert.ok(layout.body?.height > 30, '设置内容没有可用空间');
  assert.ok(layout.shell?.bottom <= layout.card.bottom + 1, '设置内容穿出卡片');
}
try {
  await evaluate(`document.querySelector('button[aria-label^="打开设置"]')?.click()`);
  await pause();
  for (const [width, height] of [[1000, 680], [800, 600], [640, 480]]) {
    await evaluate(`window.__TAURI_INTERNALS__.invoke('set_main_window_size', { config: { width: ${width}, height: ${height}, remember: false } })`);
    await pause();
    const layout = JSON.parse(await evaluate(evaluateLayout));
    verify(layout);
    console.log(`设置弹窗 ${layout.size.join('×')}: 卡片 ${Math.round(layout.card.width)}×${Math.round(layout.card.height)}，可滚动 ${layout.scroll[0] > layout.scroll[1]}`);
  }
  await evaluate(`[...document.querySelectorAll('.section-nav button')].find(button => button.innerText.includes('外观'))?.click()`);
  await pause();
  for (const style of ['classic', 'trading', 'modern']) {
    for (const theme of ['light', 'dark']) {
      await evaluate(`document.querySelector('.${style}-preview')?.click()`);
      await pause();
      await evaluate(`document.querySelector('.theme-card.${theme}')?.click()`);
      await pause();
      const layout = JSON.parse(await evaluate(evaluateLayout));
      verify(layout);
      assert.equal(layout.style, style);
      assert.equal(layout.theme, theme);
      console.log(`${style}/${theme}: 弹窗、关闭键、完成键均在窗口内`);
    }
  }
  const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  if (process.env.BULL_RESPONSIVE_SCREENSHOT) {
    writeFileSync(process.env.BULL_RESPONSIVE_SCREENSHOT, Buffer.from(png.data, 'base64'));
    console.log(`截图：${process.env.BULL_RESPONSIVE_SCREENSHOT}`);
  }
} finally {
  ws.close();
}
