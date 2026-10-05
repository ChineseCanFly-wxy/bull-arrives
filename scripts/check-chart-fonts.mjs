import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
import * as Vue from 'vue';
// Actual shared chart styles against a memory-only chart; no quotes, database or native window.
const settings=Vue.reactive({theme:'light',visualStyle:'modern'});
const tokens=new Map([['--font-sans','"Microsoft YaHei", sans-serif'],['--font-mono','Consolas, monospace']]);
const calls=[];let resized=0;
const fakeChart={setStyles(value){calls.push(value);},resize(){resized++;}};
const module={exports:{}};
const compiled=ts.transpileModule(readFileSync('src/composables/useChartCore.ts','utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.CommonJS}}).outputText;
vm.runInNewContext(compiled,{module,exports:module.exports,document:{documentElement:{}},getComputedStyle:()=>({getPropertyValue:key=>tokens.get(key)||''}),requestAnimationFrame:fn=>fn(),require:id=>{
 if(id==='vue')return {...Vue,onUnmounted(){}};
 if(id==='klinecharts')return {init:()=>fakeChart,dispose(){}};
 if(id==='@/stores/settings')return {useSettingsStore:()=>settings};
 if(id==='@/utils/format')return {getPricePrecision:()=>2};
 if(id==='./minutePeriod')return {isMinuteK:()=>false,minuteKSpan:()=>null};
 throw Error('Unexpected import '+id);
}});
const scope=Vue.effectScope();
try{
 const core=scope.run(()=>module.exports.useChartCore({chartRef:Vue.ref(null),code:'600519',market:'sh'}));
 core.chart.value=fakeChart;core.applyChartStyles();
 assert.equal(calls.at(-1).xAxis.tickText.family,'Consolas, monospace');
 assert.equal(calls.at(-1).candle.tooltip.legend.family,'Consolas, monospace');
 tokens.set('--font-sans','"Microsoft YaHei UI", "Microsoft YaHei", sans-serif');
 tokens.set('--font-mono','Arial, "Microsoft YaHei", sans-serif');
 settings.visualStyle='elegant';await Vue.nextTick();
 const elegant=calls.at(-1);
 assert.equal(elegant.xAxis.tickText.family,tokens.get('--font-mono'));
 assert.equal(elegant.yAxis.tickText.size,12);
 assert.equal(elegant.candle.tooltip.legend.family,tokens.get('--font-mono'));
 assert.equal(elegant.candle.tooltip.legend.size,13);
 assert.equal(elegant.indicator.tooltip.title.family,tokens.get('--font-sans'));
 assert.equal(elegant.crosshair.horizontal.text.family,tokens.get('--font-mono'));
 core.currentPeriod.value='daily';core.reapplyStyles();
 assert.equal(calls.at(-1).candle.type,'candle_solid');
 assert.equal(calls.at(-1).candle.tooltip.legend.family,tokens.get('--font-mono'));
 settings.visualStyle='classic';tokens.set('--font-mono','Consolas, monospace');await Vue.nextTick();
 assert.equal(calls.at(-2).xAxis.tickText.size,10);
 assert.equal(calls.at(-1).candle.tooltip.legend.family,'Consolas, monospace');
 assert.ok(resized>=2,'Theme changes resize the actual chart');
 assert.ok(!JSON.stringify(calls).includes('var(--font-'),'Canvas receives resolved font families, never CSS variable strings');
 console.log('Shared chart font check passed: resolved families, minute/daily/indicator/crosshair styles and theme switching.');
}finally{scope.stop();}
