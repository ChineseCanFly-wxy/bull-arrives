export type ConditionTree = {op:'and'|'or';children:ConditionTree[]}|{op:'mainline'}|{op:'change_above'|'change_below';value:number};
export function defaultConditionTree():ConditionTree{return {op:'and',children:[{op:'mainline'},{op:'change_above',value:1}]};}
export function isConditionTree(value:unknown,depth=1):value is ConditionTree {
  if(!value||typeof value!=='object'||depth>4)return false;
  const node=value as Record<string,unknown>;
  switch(node.op){
    case'and':case'or':return Array.isArray(node.children)&&node.children.length>=2&&node.children.length<=4&&node.children.every(child=>isConditionTree(child,depth+1));
    case'mainline':return true;
    case'change_above':case'change_below':return typeof node.value==='number'&&Number.isFinite(node.value)&&node.value>=-30&&node.value<=30;
    default:return false;
  }
}
export function describeCondition(node:ConditionTree):string{switch(node.op){case'and':case'or':return '('+node.children.map(describeCondition).join(node.op==='and'?' 且 ':' 或 ')+')';case'mainline':return'强势主线领涨';case'change_above':return'涨跌幅 ≥ '+node.value+'%';case'change_below':return'涨跌幅 ≤ '+node.value+'%';}}
