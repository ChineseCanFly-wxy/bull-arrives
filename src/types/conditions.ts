export type ConditionTree = {op:'and'|'or';children:ConditionTree[]}|{op:'intraday'|'mainline'}|{op:'change_above'|'change_below';value:number};
export function defaultConditionTree():ConditionTree{return {op:'and',children:[{op:'mainline'},{op:'change_above',value:1}]};}
export function usesRetiredMinutes(node:ConditionTree):boolean{return node.op==='intraday'||('children'in node&&node.children.some(usesRetiredMinutes));}
export function describeCondition(node:ConditionTree):string{switch(node.op){case'and':case'or':return '('+node.children.map(describeCondition).join(node.op==='and'?' 且 ':' 或 ')+')';case'intraday':return'已停用的分钟确认';case'mainline':return'强势主线领涨';case'change_above':return'涨跌幅 ≥ '+node.value+'%';case'change_below':return'涨跌幅 ≤ '+node.value+'%';}}
