import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

// Exercise the real store with IPC responses; no Claude call or user-data write.
const fingerprint = 'snapshot-A';
const task = { id: 'team-task', symbol: 'sz300308', context_fingerprint: fingerprint, team: true };
const result = { status: 'ready', context_fingerprint: fingerprint, summary: '谨慎观察' };
let pendingImport;
let failProgress = false;
let reads = 0;
async function invoke(command) {
  if (command === 'import_interactive_analysis') return pendingImport ?? result;
  if (command === 'list_interactive_analyses') return [task];
  if (command === 'inspect_interactive_analysis') {
    reads++;
    if (failProgress) throw new Error('讨论记录暂时不可读');
    return { task, progress: '多方承认反转未确认；空方承认修复未失效。' };
  }
  if (command === 'analyze_stock_agent') return result;
  throw new Error(`Unexpected IPC: ${command}`);
}
const source = ts.transpileModule(readFileSync(new URL('../src/stores/analysis.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const module = { exports: {} };
vm.runInThisContext(`(function(require,module,exports){${source}\n})`)(name => {
  if (name === '@tauri-apps/api/core') return { invoke };
  if (name === '@tauri-apps/api/event') return { listen: async () => () => {} };
  if (name === 'vue') return { ref: value => ({ value, __testRef: true }) };
  if (name === 'pinia') return { defineStore: (_name, setup) => () => {
    const store = setup();
    return new Proxy(store, {
      get(target, key) { const value = target[key]; return value?.__testRef ? value.value : value; },
      set(target, key, value) { if (target[key]?.__testRef) target[key].value = value; else target[key] = value; return true; },
    });
  } };
  throw new Error(`Unexpected import: ${name}`);
}, module, module.exports);
const store = module.exports.useAnalysisStore();
function openSnapshot() {
  store.symbol = task.symbol;
  store.analysis = { agent_context_fingerprint: fingerprint };
  store.agentStatus = { installed: true };
  store.interactiveTasks = [task];
}

openSnapshot();
await store.importInteractiveAnalysis(task.id);
assert.equal(store.importedTeamTaskId, task.id);
assert.equal(reads, 1, 'Team import must load its discussion without another click');
assert.match(store.interactiveActivities[task.id].progress, /空方承认/);
await store.analyzeWithAgent();
assert.equal(store.importedTeamTaskId, null, 'Single-agent results must not inherit Team discussion');

store.reset();
openSnapshot();
failProgress = true;
await store.importInteractiveAnalysis(task.id);
assert.equal(store.agentAnalysis.status, 'ready', 'A missing discussion must not discard the validated result');
assert.match(store.interactiveError, /讨论记录暂时不可读/);

store.reset();
openSnapshot();
let complete;
pendingImport = new Promise(resolve => { complete = resolve; });
const importing = store.importInteractiveAnalysis(task.id);
store.reset();
complete(result);
await importing;
assert.equal(store.importedTeamTaskId, null, 'A late import must not attach discussion after switching stock');
assert.equal(store.agentAnalysis, null);
console.log('Team import loads discussion, preserves valid results on read failure, and isolates later single-agent/stock results.');
