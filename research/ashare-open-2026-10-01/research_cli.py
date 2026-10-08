"""Reproducible research entry point. Numeric observations, never broker orders."""
import argparse
import datetime as dt
import hashlib
import json
import pickle
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OLD = ROOT.parent / "ashare-2026-10-01"
SNAPSHOT = OLD / "exports/matrices.npz"
INDEX = ROOT / "online/index-sh-000300.ndjson"
LOCK = ROOT / "research-lock.json"
BATCHES = {
    "technical": "technical/technical_open.py", "peers": "peers/peer_study.py",
    "relative": "learning/rolling_models.py", "absolute": "learning/absolute_models.py",
    "execution": "execution_study.py", "refine": "candidate_refine.py",
    "stress": "candidate-stress/stress_candidates.py", "actions": "action-study/action_study.py",
    "ranking": "ranking-study/ranking_study.py",
}


def sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False)+"\n", encoding="utf-8")


def model_files(kind, year=None):
    base = "absolute-" if kind == "absolute" else ""
    return ROOT / f"learning/{base}results/hist_tree_h20_scores.npy", (
        ROOT / f"learning/{base}models/hist_tree_h20_y{year}.pkl" if year else None)


def freeze(refresh=False):
    if LOCK.exists() and not refresh:
        raise ValueError("已有冻结清单；研究重跑后明确使用 freeze --refresh 保留新的版本。")
    paths = [SNAPSHOT, OLD/"exports/matrices-metadata.json", OLD/"study.py", INDEX,
        ROOT/"learning/rolling_models.py", ROOT/"learning/absolute_models.py", ROOT/"execution.py",
        ROOT/"online/action-overlay.json"]
    for kind in ("absolute", "relative"):
        paths.append(model_files(kind)[0])
        paths.extend(model_files(kind, year)[1] for year in range(2020, 2027))
    entries = {str(p.relative_to(ROOT.parent)): sha(p) for p in paths}
    if LOCK.exists():
        # ponytail: append-only JSON snapshots suffice for this local research; no database.
        archived = ROOT/"locks"/f"{dt.datetime.now(dt.timezone.utc):%Y%m%dT%H%M%S%fZ}.json"
        archived.parent.mkdir(exist_ok=True); archived.write_bytes(LOCK.read_bytes())
    result = {"schema": 1, "created_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "files": entries, "production_admission": False}
    write(LOCK, result); return result


def verify(lock, paths):
    for path in paths:
        key = str(path.relative_to(ROOT.parent))
        if key not in lock["files"] or sha(path) != lock["files"][key]:
            raise ValueError(f"冻结文件不匹配：{path}；不得混用旧模型、特征或评分。")


def compare():
    files = ("technical/results/summary.json", "peers/results/summary.json",
        "learning/results/summary.json", "execution-results/summary.json",
        "learning/absolute-results/summary.json", "candidate-refine/summary.json",
        "candidate-stress/summary.json", "action-study/summary.json", "ranking-study/summary.json")
    rows = []
    for name in files:
        path = ROOT/name
        if not path.exists(): continue
        data = read(path); experiments = data.get("experiments", data.get("configs", {}))
        if "seeds" in data and "anchors" in data:
            count=sum(sum(len(p["seeds"]) for p in v["pools"].values()) for v in data["anchors"].values())
        else:
            count = sum(len(v) for v in data["anchors"].values()) if "anchors" in data else len(experiments)
        rows.append({"path": name, "sha256": sha(path), "configurations": count,
            "styles": data.get("styles", data.get("style_comparators", {})),
            "action_coverage": data.get("action_coverage"), "production_admission": False})
    candidate = read(ROOT/"candidate-refine/summary.json")
    anchors = {k: v for k,v in candidate["experiments"].items() if "double_cost" in v}
    return {"schema": 1, "status": "retrospective_exploration", "batches": rows,
        "configuration_count_not_independent_strategies": sum(r["configurations"] for r in rows),
        "latest_action_replay_anchors": anchors, "production_admission": False,
        "audit_paths": ["absolute-audit/RESULTS.md", "candidate-audit/RESULTS.md"],
        "warning": "各批次执行和公司行动版本不同；仅最新复跑锚可直接比较。后段已见，分数不是胜率。"}


def validate_snapshot(codes, dates, raw, seen, valid, factors, np, stock_pattern):
    shape = (len(dates), len(codes))
    if not codes or len(codes) > 10000 or any(not isinstance(c,str) for c in codes) or len(set(codes)) != len(codes):
        raise ValueError("股票代码为空、重复或数量异常")
    if any(not stock_pattern.fullmatch(c) for c in codes):
        raise ValueError("仅接受沪深普通股票代码；排除北交所和非股票")
    if dates.ndim!=1 or dates.dtype.kind not in "iu" or len(dates) < 120 or len(dates) > 10000 or np.any(np.diff(dates) <= 0):
        raise ValueError("日期必须递增且至少有120个历史市场交易日")
    for day in dates:
        dt.datetime.strptime(str(int(day)), "%Y%m%d")
    for key in ("open", "high", "low", "close", "volume", "amount", "is_st", "damaged"):
        if key not in raw or raw[key].shape != shape: raise ValueError(f"缺字段/维度异常：{key}")
    if any(v.shape != shape or v.dtype.kind not in "bifu" for v in raw.values()):
        raise ValueError("行情字段必须全部为同维度数值/布尔数组")
    if any(a.shape != shape for a in (seen, valid, factors)):
        raise ValueError("seen/valid/factors维度不一致")
    if seen.dtype != bool or valid.dtype != bool or raw["damaged"].dtype != bool:
        raise ValueError("质量和隔离标记必须为布尔值")
    if not np.all(np.isfinite(factors) & (factors > 0)):
        raise ValueError("复权因子须为有限正值")
    price_ok = np.isfinite(raw["open"]) & np.isfinite(raw["high"]) & np.isfinite(raw["low"]) & np.isfinite(raw["close"])
    price_ok &= (raw["open"] > 0) & (raw["low"] > 0) & (raw["close"] > 0)
    price_ok &= (raw["high"]+.011 >= np.maximum(raw["open"],raw["close"]))
    price_ok &= (raw["low"]-.011 <= np.minimum(raw["open"],raw["close"]))
    price_ok &= np.isfinite(raw["volume"]) & np.isfinite(raw["amount"]) & (raw["volume"]>0) & (raw["amount"]>0)
    if np.any(valid & (~seen | ~price_ok)):
        raise ValueError("valid将未见或坏OHLC/量额记录标为有效")


def index_inputs(dates, path, np, s):
    values = {}
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        if not line.strip(): continue
        row = json.loads(line); day = int(row["date"].replace("-", ""))
        if day in values: raise ValueError("沪深300指数日期重复")
        if row.get("code") != "sh.000300": raise ValueError("指数必须显式为sh.000300")
        dt.datetime.strptime(str(day),"%Y%m%d")
        price=float(row["close"])
        if not np.isfinite(price) or price<=0:raise ValueError("指数价格无效")
        values[day] = price
    if list(values)!=sorted(values):raise ValueError("指数日期必须按市场日递增")
    if any(int(d) not in values for d in dates): raise ValueError("指数未覆盖股票快照交易日；不能填充未来/缺失指数")
    expected=[day for day in values if int(dates[0])<=day<=int(dates[-1])]
    if dates.tolist()!=expected:raise ValueError("股票日期表缺少真实指数交易日；不能压缩市场时间")
    prices = np.array([values[int(d)] for d in dates], dtype=np.float32)
    if not np.all(np.isfinite(prices) & (prices>0)): raise ValueError("指数价格无效")
    r1 = prices/s.lag(prices)-1
    x = np.column_stack((prices/s.lag(prices,5)-1, prices/s.lag(prices,20)-1,
        prices/s.roll(prices[:,None],60)[:,0]-1, s.roll(r1[:,None],20,"std")[:,0]))
    return np.nan_to_num(x).astype(np.float32), max(values)


def screen(args):
    sys.path.insert(0, str(ROOT/"learning"))
    import rolling_models as m
    import absolute_models as a
    np, s = m.np, m.s
    lock = read(LOCK)
    verify(lock, [OLD/"study.py", ROOT/"learning/rolling_models.py", ROOT/"learning/absolute_models.py", ROOT/"execution.py"])
    rules=read(ROOT/"rule-study/sse-sz-20260706/summary.json")
    if rules["sse_effective_date"]!=20260706 or rules["sz_effective_date"]!=20260706:
        raise ValueError("已核验规则日期与观察卡不一致")
    for key,digest in rules["input_sha256_start"].items():
        if key.startswith("official_rules_"):
            path=ROOT/"online/rules"/key.removeprefix("official_rules_")
            if sha(path)!=digest:raise ValueError(f"规则证据已变化：{path}")
    snapshot = (args.snapshot or SNAPSHOT).resolve()
    external = args.snapshot is not None
    verify(lock, [SNAPSHOT])
    with np.load(snapshot, allow_pickle=False) as data:
        codes = data["codes"].tolist(); dates = data["dates"]
        raw = {k.removeprefix("raw_"):data[k] for k in data.files if k.startswith("raw_")}
        seen,valid,factors = data["seen"],data["valid"],data["factors"]
    validate_snapshot(codes, dates, raw, seen, valid, factors, np, s.STOCK)
    if external:
        with np.load(SNAPSHOT, allow_pickle=False) as reference:
            old_codes = set(reference["codes"].tolist()); first_reference_day = int(reference["dates"][0])
            reference_dates=reference["dates"]
        if not old_codes.issubset(codes) or int(dates[0]) != first_reference_day:
            raise ValueError("外部快照须保留冻结全股票池和历史起点；小候选篮子不能替代模型横截面")
        if len(dates)<len(reference_dates) or not np.array_equal(dates[:len(reference_dates)],reference_dates):
            raise ValueError("外部快照须保留完整冻结日期前缀，不能删除中间市场交易日")
    requested = args.as_of or int(dates[-1])
    at = int(np.searchsorted(dates, requested))
    if at == len(dates) or int(dates[at]) != requested: raise ValueError("as-of必须是快照中的精确交易日，不能将旧行情叫作今天")
    if at < 119: raise ValueError("可用历史不足120交易日")
    original_shape = valid.shape; snapshot_end = int(dates[-1])
    dates = dates[:at+1]; raw = {k:v[:at+1] for k,v in raw.items()}
    seen,valid,factors = seen[:at+1],valid[:at+1],factors[:at+1]
    if args.model == "relative" and any(k not in raw for k in ("pe_ttm","pb")):
        raise ValueError("相对模型需要原估值代理；不得把缺失基本面虚构为已知")
    for key in ("pe_ttm", "pb"):
        raw.setdefault(key, np.zeros_like(raw["close"]))
    f = s.features(raw,valid,factors,seen)
    if external and f["eligible"][-1].sum()<2000:
        raise ValueError("外部截面有效合格股票不足2000；缺失行情不能生成全市场排名")
    names,arrays,market = m.inputs(raw,valid,f)
    index_path = (args.index or INDEX).resolve()
    if args.index is None: verify(lock, [INDEX])
    idx,latest_index = index_inputs(dates,index_path,np,s)
    if external:
        year = requested//10000; model_path = model_files(args.model,year)[1]
        verify(lock,[model_path])
        # Only our explicitly frozen local pickle can execute; never a supplied model file.
        with model_path.open("rb") as handle: model = pickle.load(handle)
        cols = np.flatnonzero(f["eligible"][-1]); score = np.full(len(codes),np.nan)
        x = a.xday(arrays,market,idx,len(dates)-1,cols) if args.model=="absolute" else m.day_x(arrays,market,len(dates)-1,cols)
        if len(cols): score[cols]=m.predict(model,x)
        score_sha = sha(model_path); score_origin = "frozen_year_model_new_snapshot_inference"
    else:
        score_path = model_files(args.model)[0]; verify(lock,[score_path])
        cached = np.load(score_path,allow_pickle=False,mmap_mode="r")
        if cached.shape != original_shape: raise ValueError("评分缓存与快照不匹配")
        score = cached[at]; score_sha = sha(score_path); score_origin = "audited_historical_score_cache"
    selected = f["eligible"][-1] & np.isfinite(score) & (score>0)
    positive_before_market_gate=int(selected.sum())
    breadth = float(f["breadth"][-1]); previous_breadth = float(f["breadth"][-6])
    market_gate_passed=args.model=="absolute" or (breadth>=.45 and breadth-previous_breadth>=0)
    if args.model == "relative": selected &= market_gate_passed
    cols = np.flatnonzero(selected); cols = cols[np.argsort(-score[cols],kind="stable")[:args.limit]]
    cards = []
    for c in cols:
        close = float(raw["close"][-1,c]); atr = float(f["atr"][-1,c]/factors[-1,c])
        weights = [.08] if args.mode=="full" else [.032,.024,.024]
        budget = min(args.capital*weights[0], float(raw["amount"][-1,c])*.01)
        price = close*1.001; qty = s.lot_buy(codes[c],max(0,budget-5),price)
        while qty and qty*price+s.fee(qty*price,False,requested)>budget:
            qty -= 1 if codes[c].startswith(("688","689")) else 100
            if codes[c].startswith(("688","689")) and qty<200:qty=0
        cards.append({"code":codes[c], "model_score_proxy":float(score[c]), "score_is_probability":False,
            "raw_close":close,"raw_atr14":atr,"r20":float(f["r20"][-1,c]),
            "distance_ma20_pct":float((f["close"][-1,c]/f["ma20"][-1,c]-1)*100),
            "action_intent":{"max_names":10,"target_capital_weights":weights,"initial_budget_cny":round(budget,2),
                "quantity_estimate_at_reference_close":int(qty),"quantity_is_executable_order":False,
                "next_open_price_cap_before_slippage":round(close*1.04,3),
                "entry":"next tradable open; reject ST, invalid reference, price cap or limit boundary; T+1 by lot",
                "add":"layers only: within first7 sessions, completed close above MA10 and entry+stage*0.5*currentATR; next open; cap total8%",
                "exit":"20-session intent, prior completed close below adjusted entry-3*entryATR, or becameST/unknown; next executable open",
                "st_exit_limit_rule":{"reference":"official SSE/SZSE2026 rules, verified date map; intent only, no order execution",
                    "mainboard":[{"through":20260705,"fraction":.05},{"from":20260706,"fraction":.10}],
                    "growth_star_fraction":.20,"date_key":"actual attempted execution market date"},
                "deferred_exit":"blocked sells retained with actual overdue sessions; dividends and bonus shares need verified dates"}})
    return {"schema":1,"as_of":requested,"snapshot_end":snapshot_end,"latest_online_index_date":latest_index,
        "stale_to_online_calendar":requested<latest_index,"model":args.model,"prediction_year":requested//10000,
        "mode":args.mode,"score_origin":score_origin,"snapshot_sha256":sha(snapshot),"score_or_model_sha256":score_sha,
        "index_sha256":sha(index_path),"stock_universe_size":len(codes),"eligible_pool":int(f["eligible"][-1].sum()),
        "positive_score_pool":positive_before_market_gate,"after_market_gate_pool":int(selected.sum()),
        "market_gate_passed":bool(market_gate_passed),
        "empty_reason":"no_positive_model_scores" if positive_before_market_gate==0 else "market_gate_failed" if not market_gate_passed else None,
        "breadth_above_ma20":breadth,"breadth_change5":breadth-previous_breadth,
        "rule_evidence_sha256":{str(p.relative_to(ROOT)):sha(p) for p in (ROOT/"online/rules/sources.json",ROOT/"online/rules/szse-sources.json",ROOT/"rule-study/sse-sz-20260706/summary.json")},
        "data_status":"external_snapshot_structural_checks_only" if external else "frozen_offline_snapshot",
        "evidence_status":"research_observation","production_admission":False,"candidates":cards,
        "warnings":["回顾性研究候选，分数是收益代理而非上涨概率；没有已证实的排序增益。",
            "日线开盘为成交代理，缺队列/深度；公司行动、ST知时与退市清算覆盖仍不完整。",
            "外部快照需全股票截面及历史隔离标记；结构校验不能证明历史信息版本真实。" ]}


def self_check():
    sys.path.insert(0,str(ROOT/"learning")); import rolling_models as m
    np=m.np; shape=(120,2); dates=np.array([int((dt.date(2020,1,1)+dt.timedelta(days=i)).strftime("%Y%m%d")) for i in range(120)])
    raw={k:np.ones(shape) for k in ("open","high","low","close","volume","amount","is_st")}
    raw["is_st"]*=0;raw["damaged"]=np.zeros(shape,bool); seen=np.ones(shape,bool)
    validate_snapshot(["600000","300001"],dates,raw,seen,seen,np.ones(shape),np,m.s.STOCK)
    for codes,bad_dates in ((["600000","830001"],dates),(["600000","600000"],dates),(["600000","300001"],dates[::-1])):
        try:validate_snapshot(codes,bad_dates,raw,seen,seen,np.ones(shape),np,m.s.STOCK)
        except ValueError:pass
        else:raise AssertionError("bad universe/date accepted")
    raw["pe_ttm"]=np.ones((1,2))
    try:validate_snapshot(["600000","300001"],dates,raw,seen,seen,np.ones(shape),np,m.s.STOCK)
    except ValueError:pass
    else:raise AssertionError("malformed optional valuation accepted")
    del raw["pe_ttm"];raw["high"][0,0]=.5
    try:validate_snapshot(["600000","300001"],dates,raw,seen,seen,np.ones(shape),np,m.s.STOCK)
    except ValueError:pass
    else:raise AssertionError("bad OHLC accepted")
    path=ROOT/"research_cli.py"
    try:verify({"files":{str(path.relative_to(ROOT.parent)):"invalid"}},[path])
    except ValueError:pass
    else:raise AssertionError("tampered fingerprint accepted")
    with tempfile.TemporaryDirectory() as folder:
        index=Path(folder)/"index.ndjson"
        rows=[{"date":dt.datetime.strptime(str(day),"%Y%m%d").strftime("%Y-%m-%d"),"code":"sh.000300","close":10} for day in dates]
        index.write_text("\n".join(json.dumps(r) for r in rows),encoding="utf-8")
        index_inputs(dates,index,np,m.s)
        try:index_inputs(np.delete(dates,60),index,np,m.s)
        except ValueError:pass
        else:raise AssertionError("compressed market calendar accepted")
        del rows[0]["code"];index.write_text("\n".join(json.dumps(r) for r in rows),encoding="utf-8")
        try:index_inputs(dates,index,np,m.s)
        except ValueError:pass
        else:raise AssertionError("index without explicit exchange accepted")
    print("CLI self-check passed: valid snapshot; reject Beijing, duplicates, reversed dates, invalid OHLC and mismatched fingerprint")


def main():
    parser=argparse.ArgumentParser(description=__doc__); sub=parser.add_subparsers(dest="command",required=True)
    p=sub.add_parser("freeze");p.add_argument("--refresh",action="store_true")
    p=sub.add_parser("compare");p.add_argument("--output",type=Path,default=ROOT/"comparison.json")
    p=sub.add_parser("screen");p.add_argument("--model",choices=("absolute","relative"),default="absolute")
    p.add_argument("--mode",choices=("full","layers"),default="full");p.add_argument("--as-of",type=int)
    p.add_argument("--snapshot",type=Path);p.add_argument("--index",type=Path);p.add_argument("--limit",type=int,default=10)
    p.add_argument("--capital",type=float,default=100000);p.add_argument("--output",type=Path)
    p=sub.add_parser("run");p.add_argument("batch",choices=tuple(BATCHES));sub.add_parser("check")
    args=parser.parse_args()
    try:
        if args.command=="check":self_check();return
        if args.command=="run":subprocess.run([sys.executable,str(ROOT/BATCHES[args.batch])],cwd=ROOT,check=True);return
        if args.command=="freeze":result=freeze(args.refresh)
        elif args.command=="compare":result=compare();write(args.output,result)
        else:
            if not 1<=args.limit<=100 or not 0<args.capital<1e10:raise ValueError("limit须1—100，capital须有限正数")
            result=screen(args);output=args.output or ROOT/"screens"/f"{result['as_of']}-{args.model}-{args.mode}.json"
            write(output,result)
        print(json.dumps(result,ensure_ascii=False,indent=2,allow_nan=False))
    except (ValueError,KeyError,FileNotFoundError) as error:
        parser.exit(2,f"研究入口拒绝执行：{error}\n")


if __name__=="__main__":main()
