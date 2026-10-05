"""Single trusted frozen financial37 replay; no network/refit/forward extension."""
import argparse
import datetime as dt
import hashlib
import json
import math
import sys
import uuid
from pathlib import Path

HERE=Path(__file__).resolve().parent
REPO=HERE.parents[1]
SOURCES_SHA="a3e6127dcf40b1946b9975b17078a2ea1aedb57e0d45e80d002e927d39bc9b39"
MODEL_ID="fundamental37_h20"
MODEL_NAME="基本面37探索模型"
SOURCE_RUN_ID="ashare-fundamental-2026-10-01-conservative-ablation-v1"
SCORE_SEMANTIC="二十日绝对开盘收益代理；财务时点保守延后，历史修订链未认证；不是胜率"


def digest(path):
    value=hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda:stream.read(4*1024*1024),b""):value.update(block)
    return value.hexdigest()


def verify_sources():
    source=HERE/"replay-sources.json"
    if digest(source)!=SOURCES_SHA:raise ValueError("财务回放受信来源清单变化；拒绝执行")
    manifest=json.loads(source.read_text(encoding="utf-8"));hashes={};paths={}
    for key,row in manifest["files"].items():
        path=(REPO/row["path"]).resolve()
        if not path.is_relative_to(REPO) or digest(path)!=row["sha256"]:raise ValueError("财务回放冻结输入漂移："+key)
        hashes[key]=row["sha256"];paths[key]=path
    hashes["replay_sources"]=SOURCES_SHA
    return manifest,paths,hashes


def validate_choice(model,hold,comparison,mode):
    if (model,hold,comparison,mode)!=(MODEL_ID,20,"baseline","replay"):
        raise ValueError("财务37只支持冻结20日原执行历史回放；不能前向延续、训练、改阈值或改执行")


def scored_rows(np,codes,score,eligible):
    return [{"symbol":("sh" if codes[c].startswith("6") else "sz")+codes[c],"score":float(score[c]),"threshold":0.,"as_of":"2026-09-30"} for c in np.flatnonzero(eligible&np.isfinite(score))]


def emit(args):
    validate_choice(args.model_id,args.holding_days,args.comparison,args.mode)
    output=args.output.resolve()
    if output.exists():raise ValueError("拒绝覆盖已有账本")
    manifest,paths,hashes=verify_sources()
    snapshot=args.snapshot.resolve() if args.snapshot else paths["snapshot"].parent
    index=args.index.resolve() if args.index else paths["index"]
    supplied={"snapshot":snapshot/"matrices.npz","metadata":snapshot/"matrices-metadata.json","index":index}
    if any(digest(path)!=hashes[key] for key,path in supplied.items()):raise ValueError("财务回放只允许完全相同的冻结矩阵/元数据/指数；动态快照没有财务更新证据")
    # All executable imports occur only after all transitive source hashes pass.
    sys.path.insert(0,str(HERE))
    import fundamental_study as fs
    r,np,s,e,rules=fs.r,fs.np,fs.s,fs.e,fs.rules
    with np.load(supplied["snapshot"],allow_pickle=False) as data:
        codes,dates=data["codes"].tolist(),data["dates"]
        raw={key.removeprefix("raw_"):data[key] for key in data.files if key.startswith("raw_")};valid,seen,factors=data["valid"],data["seen"],data["factors"]
    if len(codes)!=5427 or len(dates)!=2123 or int(dates[-1])!=20260930 or len(set(codes))!=len(codes) or any(not s.STOCK.fullmatch(code) for code in codes):raise ValueError("冻结沪深股票/日期轴不符")
    if np.any(np.diff(dates)<=0) or valid.shape!=(len(dates),len(codes)) or not np.all(np.isfinite(factors)&(factors>0)):raise ValueError("市场轴/复权因子无效")
    index_rows=[json.loads(line) for line in index.read_text(encoding="utf-8").splitlines() if line.strip()]
    if any(row.get("code")!="sh.000300" for row in index_rows) or [int(row["date"].replace("-","")) for row in index_rows]!=dates.tolist():raise ValueError("CSI300显式身份/完整交易日不符")
    metadata=json.loads(supplied["metadata"].read_text(encoding="utf-8"));events={(row["i"],row["c"]):row["event"] for row in metadata["events"]}
    if len(events)!=len(metadata["events"]) or any(not(0<=i<len(dates) and 0<=c<len(codes)) for i,c in events):raise ValueError("公司行动重复或坐标越界")
    score=np.load(paths[MODEL_ID+"_score"],allow_pickle=False)
    if score.shape!=valid.shape:raise ValueError("财务分数和市场轴不同")
    f=s.features(raw,valid,factors,seen);f["score"]=score;f["signals"][MODEL_ID]=f["eligible"]&np.isfinite(score)&(score>0)
    details,action_coverage=rules.action_map(codes,dates,events);rules.SZ_EFFECTIVE=20260706;rules.self_check();old_open=e.tradable_open
    try:
        e.tradable_open=rules.tradable_open
        ledger=e.backtest(MODEL_ID,20200101,20260930,codes,dates,raw,valid,factors,events,f,multiplier=1,full_entry=True,hold=20,journal=True,exit_policy="time",max_positions=10,action_details=details)
    finally:e.tradable_open=old_open
    accounting=rules.accounting(ledger)
    if accounting["minimum_cash_cny"]<0 or abs(accounting["reconcile_cny"])>.1:raise ValueError("财务账户资金不守恒")
    ci={code:c for c,code in enumerate(codes)};di={int(day):i for i,day in enumerate(dates)}
    for row in ledger["orders"]:
        if row["side"]=="buy":
            i,c=di[row["date"]],ci[row["code"]]
            if raw["is_st"][i,c]!=0 or i<=0 or not f["signals"][MODEL_ID][i-1,c]:raise ValueError("财务买单ST排除/前日冻结信号失败")
    current=scored_rows(np,codes,score[-1],f["eligible"][-1]);unknown=[{"symbol":("sh" if codes[c].startswith("6") else "sz")+codes[c],"as_of":"2026-09-30","score":None,"reason":"主要财务字段不足共同研究池条件；未知不填0"} for c in np.flatnonzero(f["eligible"][-1]&~np.isfinite(score[-1]))]
    if (len(current),len(unknown),sum(row["score"]>0 for row in current))!=(4454,1,4021):raise ValueError("冻结财务评分数量不符")
    cash=ledger["curve"][-1]["cash"];equity=ledger["curve"][-1]["equity"];watch=[]
    for c in np.flatnonzero(f["signals"][MODEL_ID][-1]):
        close=float(raw["close"][-1,c]);budget=min(max(0,cash-10),equity*.08,float(raw["amount"][-1,c])*.01);price=close*1.001;qty=s.lot_buy(codes[c],budget,price)
        while qty>0 and qty*price+s.fee(qty*price,False,20260930)>budget:
            qty-=1 if codes[c].startswith(("688","689")) else 100
            if codes[c].startswith(("688","689")) and qty<200:qty=0
        watch.append({"symbol":("sh" if codes[c].startswith("6") else "sz")+codes[c],"score":float(score[-1,c]),"threshold":0.,"as_of":"2026-09-30","close":close,"signal_eligible":True,"cash_reference_quantity":qty,"cash_reference_eligible":qty>0,"cash_reference_stage":"full_target_8pct","technical_state":"冻结技术适用池及财务共同覆盖；修订链未认证","reason":"未准入广候选池，4021正分不是4021买入指令；下一开盘/限幅/现金未知"})
    watch.sort(key=lambda row:(-row["score"],row["symbol"]))
    if {row["symbol"]:row["score"] for row in watch}!={row["symbol"]:row["score"] for row in current if row["score"]>0}:raise ValueError("财务全评分与观察名单不同")
    if hashes!=verify_sources()[2] or any(digest(path)!=hashes[key] for key,path in supplied.items()):raise ValueError("计算期间来源改变")
    content={"schema":"ashare-model-run-v1","run_id":str(uuid.uuid4()),"generated_at":dt.datetime.now(dt.timezone.utc).isoformat(),"model_id":MODEL_ID,"model_name":MODEL_NAME,"score_semantic":SCORE_SEMANTIC,"signal_threshold":0.,"label_holding_days":20,"holding_days":20,"comparison":"baseline","position_policy":"full8pct target","mode":"replay","forward_start":None,"as_of":"2026-09-30","production_admission":False,"exploration":True,"initial_cash_cny":100000,"source_run_id":SOURCE_RUN_ID,"input_sha256":hashes,"data_sha256":{key:hashes[key] for key in supplied},"model_sha256":hashes[MODEL_ID+"_model2026"],"score_cache_sha256":hashes[MODEL_ID+"_score"],"runner_sha256":digest(__file__),"frozen_prefix_preserved":True,"stocks":len(codes),"sessions":len(dates),"training_refitted":False,"state":"historical_replay","ledger":ledger,"accounting":{key:value for key,value in accounting.items() if key!="unclosed"},"signal_watch":watch,"current_scores":current,"unknown_current_scores":unknown,"current_financial_universe":len(current)+len(unknown),"action_coverage":action_coverage,"financial_history_version_certified":False,"forward_supported":False,"limitations":["只读冻结历史回放，财务更新未验证，不能开启自动前向账户。","财务公告/修订/入库取最晚日后才允许信号；资产负债表只有最新公告日，income可用日不认证balance修订链。","三段研究收益不能替代本次连续账户；后段多次已见，不是全新OOS。","15/25日期限早段转负，延迟1日后段增量消失；财务模型仍为探索候选。","财务证据4455只，其中1只研究字段不足导致分数未知；未知不填0。4021正分是广候选排序，分数不是胜率。","公司行动支付/新股上市日期、ST时点/竞价深度与退市回收未全认证，锁定权益及应收保守保留。"],"checks":{"trusted_inputs_sha_verified":True,"ST_BJ_buys":0,"model_score_prefix_preserved":True,"whole_ledger_pnl_reconciled":True,"current_scores_match_signal_watch":True,"no_AI_script_or_refit":True,"complete_current_universe_including_unknown":True}}
    text=json.dumps(content,ensure_ascii=False,allow_nan=False,separators=(",",":"));envelope={"schema":"model-run-bundle-v1","content":text,"content_sha256":hashlib.sha256(text.encode("utf-8")).hexdigest()}
    output.parent.mkdir(parents=True,exist_ok=True)
    with output.open("x",encoding="utf-8",newline="\n") as stream:json.dump(envelope,stream,ensure_ascii=False,separators=(",",":"))
    print(json.dumps({"output":str(output),"runner_sha256":content["runner_sha256"],"input_files":len(hashes),"current_scores":len(current),"unknown_scores":len(unknown),"positive_scores":len(watch),"orders":len(ledger["orders"]),"metrics":ledger["metrics"],"accounting":content["accounting"]},ensure_ascii=False))


def self_check():
    validate_choice(MODEL_ID,20,"baseline","replay")
    for choice in [(MODEL_ID,20,"baseline","forward"),(MODEL_ID,15,"holding15","replay"),("breadth22_h20",20,"baseline","replay")]:
        try:validate_choice(*choice)
        except ValueError:pass
        else:raise AssertionError("untrusted choice accepted")
    _,_,hashes=verify_sources();assert len(hashes)==108 and "fundamental_study" in hashes and MODEL_ID+"_model2026" in hashes
    print("self-check passed: single frozen baseline replay only; all107inputs+source manifest hashes verified; no executable imports/refit/network",flush=True)


if __name__=="__main__":
    parser=argparse.ArgumentParser();parser.add_argument("--model-id",default=MODEL_ID);parser.add_argument("--holding-days",type=int,default=20);parser.add_argument("--comparison",default="baseline");parser.add_argument("--mode",default="replay");parser.add_argument("--snapshot",type=Path);parser.add_argument("--index",type=Path);parser.add_argument("--output",type=Path);parser.add_argument("--self-check",action="store_true");args=parser.parse_args()
    if args.self_check:self_check()
    elif args.output:emit(args)
    else:parser.error("需要 --output 或 --self-check")
