import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { parse, compileScript, compileTemplate } from '@vue/compiler-sfc';
import ts from 'typescript';
import * as Vue from 'vue';

// Real setup/watchers, memory-only IPC and native-menu boundary. Never launches
// the desktop application, changes a user DB, or calls a market data provider.
const file='src/components/ticker/TickerBar.vue';
const {descriptor,errors}=parse(fs.readFileSync(file,'utf8'),{filename:file});
assert.deepEqual(errors,[]);
assert.deepEqual(compileTemplate({source:descriptor.template.content,filename:file,id:'ticker-check'}).errors,[]);
const calls=[],mounted=[],disposed=[],listeners=new Map(),intervals=new Map();
let serial=0,menuItems,menuCreates=0,menuClosed=0;
const settings=Vue.reactive({tickerDisplayMode:'fixed',tickerPageSize:2,tickerSingleColor:false,tickerTextColor:'#888',theme:'light',
  async fetchSettings(){return true;},applyTheme(){},applyRemoteSetting(key,value){if(key==='ticker_display_mode')this.tickerDisplayMode=value;}});
const watchlist=Vue.reactive({items:[],groups:[{id:0,name:'全部'}],activeGroupId:0,async fetchWatchlist(){}});
const quotes={getQuote(){return null;},async startListening(){},stopListening(){calls.push({stopped:true});}};
const owner={label:'ticker',async startDragging(){calls.push({drag:true});}};
const invoke=async(command,args)=>{calls.push({command,args});};
const Menu={async new(options){menuCreates++;menuItems=options.items;return{
  async popup(at,window){calls.push({popup:true,window:window.label});},async close(){menuClosed++;}
};}};
const vue={...Vue,onMounted:fn=>mounted.push(fn),onUnmounted:fn=>disposed.push(fn),
  watch(...args){const stop=Vue.watch(...args);disposed.push(stop);return stop;}};
const requireStub=id=>{
  if(id==='vue')return vue;
  if(id==='@tauri-apps/api/core')return{invoke};
  if(id==='@tauri-apps/api/event')return{async listen(name,fn){listeners.set(name,fn);return()=>listeners.delete(name);}};
  if(id==='@tauri-apps/api/window')return{getCurrentWindow:()=>owner};
  if(id==='@tauri-apps/api/menu')return{Menu};
  if(id==='@/stores/quote')return{useQuoteStore:()=>quotes};
  if(id==='@/stores/watchlist')return{useWatchlistStore:()=>watchlist};
  if(id==='@/stores/settings')return{useSettingsStore:()=>settings,SETTING_CHANGED_EVENT:'setting-changed'};
  return{};
};
const module={exports:{}};
const compiled=compileScript(descriptor,{id:'ticker-check'});
const code=ts.transpileModule(compiled.content,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.CommonJS}}).outputText;
vm.runInNewContext(code,{require:requireStub,module,exports:module.exports,console,
  setInterval:fn=>{const id=++serial;intervals.set(id,fn);return id;},clearInterval:id=>intervals.delete(id),
  setTimeout:()=>++serial,clearTimeout(){},document:{addEventListener(){calls.push({mouseListener:true});},removeEventListener(){}}});
const state=module.exports.default.setup({}, {expose(){}});
await mounted[0]();await Vue.nextTick();
assert.equal(calls.filter(v=>v.command==='resize_ticker_window').at(-1).args.visibleRows,1);
const stock=n=>({code:'sh'+String(n).padStart(6,'0'),market:'CN',name:'测试'+n});
watchlist.items.push(stock(1),stock(2),stock(3));await Vue.nextTick();
assert.equal(state.visibleItems.value.length,3);
assert.equal(calls.filter(v=>v.command==='resize_ticker_window').at(-1).args.visibleRows,3);
watchlist.items.push(stock(4));await Vue.nextTick();
assert.equal(state.visibleItems.value.length,4);
watchlist.items.pop();await Vue.nextTick();
assert.equal(calls.filter(v=>v.command==='resize_ticker_window').at(-1).args.visibleRows,3);
settings.tickerDisplayMode='carousel';await Vue.nextTick();
assert.equal(state.visibleItems.value.length,2);assert.equal(intervals.size,1);
[...intervals.values()][0]();assert.equal(state.page.value,2);
state.paused.value=true;[...intervals.values()][0]();assert.equal(state.page.value,2);
listeners.get('setting-changed')({payload:{key:'ticker_display_mode',value:'fixed'}});await Vue.nextTick();
assert.equal(state.visibleItems.value.length,3);assert.equal(intervals.size,0);
state.onMouseDown({button:2});assert.equal(calls.some(v=>v.mouseListener),false,'Right click must not start a drag');
await state.showContextMenu();await state.showContextMenu();
assert.equal(menuCreates,1,'Reuse the native context menu');
assert.deepEqual(Array.from(menuItems,x=>x.text),['快速自选','展示设置']);
assert.equal(calls.filter(v=>v.popup).at(-1).window,'ticker');
menuItems[0].action();await Vue.nextTick();
assert.equal(calls.filter(v=>v.command==='open_ticker_quick_add').length,1,'Same entry as the plus button');
menuItems[1].action();await Vue.nextTick();
assert.equal(calls.filter(v=>v.command==='open_navigation').at(-1).args.destination,'settings:ticker');
for(const stop of disposed)stop();await Vue.nextTick();await Promise.resolve();
assert.equal(menuClosed,1);assert.equal(listeners.size,0);
console.log('Ticker UI check passed: adaptive add/remove, empty state, carousel pause, cross-window setting, two native menu entries, quick-add reuse, direct display settings and cleanup. Native Windows menu still requires desktop verification.');
