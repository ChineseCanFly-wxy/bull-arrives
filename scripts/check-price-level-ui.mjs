import assert from 'node:assert/strict';
import {uiHarness} from './ui-check-harness.mjs';
const h=await uiHarness({name:'price-level-readable',mock:'export async function invoke(){throw Error("Price chart must not invoke IPC")}',entry:`
import {createApp,h} from 'vue';
import PriceLevelChart from '/src/components/analysis/PriceLevelChart.vue';
import '/src/assets/styles/variables.css';
import '/src/assets/workspace.css';
const levels=[{kind:'support',price:9.4,label:'前二十日支撑',source:'price',strength:75},{kind:'resistance',price:11.2,label:'布林压力',source:'price',strength:80}];
createApp({render:()=>h('main',{style:'width:min(100%,880px);padding:16px;box-sizing:border-box'},h(PriceLevelChart,{close:10,levels}))}).mount('#app');
`});
const results=[];
try {
 await h.wait('document.querySelector(".plc-plot svg")');
 for(const style of ['classic','modern','elegant'])for(const theme of ['light','dark'])for(const width of [1280,430,360]){
  await h.evaluate('document.documentElement.dataset.style='+JSON.stringify(style==='elegant'?'modern':style)+';document.documentElement.dataset.appearance='+JSON.stringify(style)+';document.documentElement.dataset.theme='+JSON.stringify(theme));
  await h.call('Emulation.setDeviceMetricsOverride',{width,height:850,deviceScaleFactor:1,mobile:false});
  const measured=await h.evaluate('(()=>{const plot=document.querySelector(".plc-plot"),svg=plot.querySelector("svg");return {documentOverflow:document.documentElement.scrollWidth>innerWidth+1,plotClient:plot.clientWidth,plotScroll:plot.scrollWidth,fonts:[...svg.querySelectorAll("text")].map(t=>({text:t.textContent.trim(),pixels:parseFloat(getComputedStyle(t).fontSize)*Math.abs(t.getScreenCTM().a)})),labels:[...document.querySelectorAll(".plc-list li")].map(l=>l.textContent)}})()');
  assert.equal(measured.documentOverflow,false,'Document overflow '+style+'/'+theme+'/'+width);
  assert.ok(measured.fonts.every(row=>row.pixels>=12),'SVG text actually rendered below12px '+JSON.stringify(measured));
  assert.equal(measured.labels.length,2);
  if(width<680)assert.ok(measured.plotScroll>measured.plotClient,'Narrow chart should scroll, not shrink text');
  results.push({style,theme,width,...measured});
  if(style==='elegant'&&width===360)await h.screenshot('price-level-'+theme+'-360');
 }
 assert.deepEqual(h.errors,[]);h.save({passed:results.length,results,limitations:'Isolated real SVG component; synthetic prices, no sources, models, accounts or orders.'});
 console.log('Price-level readability passed: '+results.length+' actual rendered font/layout checks; '+h.output);
}finally{await h.close();}
