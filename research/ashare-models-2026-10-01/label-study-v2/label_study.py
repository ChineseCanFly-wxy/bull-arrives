"""Fixed-label mechanism comparison; research only, existing source read-only."""
import argparse
import datetime as dt
import json
import math
import pickle
import sys
import time
from pathlib import Path

ROOT=Path(__file__).resolve().parent
BASE=ROOT.parent
sys.path.insert(0,str(BASE))
import model_study as r
np,s,m,a,e,rules=r.np,r.s,r.m,r.a,r.e,r.rules
PERIODS=r.PERIODS
HOLD=20
SEEDS=(0,1,2)
CONFIGS={
    "frozen22_absolute":{"label":"absolute", "threshold":0.0, "fit":False},
    "csi_excess22":{"label":"csi_excess", "threshold":0.0, "fit":True},
    "cross_section_rank22":{"label":"cross_section_rank", "threshold":0.6, "fit":True},
    "opening_downside22":{"label":"opening_downside", "threshold":0.0, "fit":True},
}


def save(path,value):r.save(path,value)


def sources():
    return {"runner":Path(__file__),"baseline_runner":BASE/"model_study.py","baseline_summary":BASE/"summary.json",
        "frozen22_scores":BASE/".cache/breadth22_h20_scores.npy","snapshot":r.DATA/"matrices.npz",
        "metadata":r.DATA/"matrices-metadata.json","features":Path(s.__file__),"learning":Path(m.__file__),
        "absolute_labels":Path(a.__file__),"execution":Path(e.__file__),"rules":Path(rules.__file__),
        "action_map":r.OPEN/"execution_study.py","action_overlay":r.OPEN/"online/action-overlay.json",
        "benchmark":r.OPEN/"online/index-sh-000300.ndjson","index_adapter":Path(r.cli.__file__),
        "sse_sources":r.OPEN/"online/rules/sources.json","szse_sources":r.OPEN/"online/rules/szse-sources.json"}


def register():
    if (ROOT/"preregistered.json").exists():raise ValueError("已有预登记，拒绝覆盖")
    hashes={k:r.sha(p) for k,p in sources().items()}
    save(ROOT/"preregistered.json",{"schema":"ashare-label-study-v2-prereg","created_at_utc":dt.datetime.now(dt.timezone.utc).isoformat(),
        "exploration":True,"production_admission":False,"configs":CONFIGS,"new_fits":21,"holding_sessions":20,
        "features":r.FEATURES[:22],"training":"each prediction year2020..2026, prior3calendar years limited2018+, 96code/datehash rows/day, exact same rows for all3labels; inclusive20session label endpoint strictly before predictionyear",
        "tree":{"max_iter":80,"learning_rate":.07,"max_leaf_nodes":15,"min_samples_leaf":80,"l2_regularization":1,"early_stopping":False,"random_state":20261001},
        "label_definitions":{
            "absolute_control":"unchanged frozen22 adjusted entryopen(i+1) to exitopen(i+20) return minus.005 clip[-.4,.4]; unavailable entry zero, missing exit retained last mark",
            "csi_excess":"stock unbounded net-return proxy minus explicit sh.000300 open(i+20)/open(i+1)-1, then clip[-.4,.4]; invalid stock entry zero. Benchmark is index return, not tradable benchmark fund return",
            "cross_section_rank":"midrank=(less+.5*equal)/N of stock unbounded net proxy across all signal-day eligible stocks, not hash96 only; retain rejected entry zero/missing exit mark. Prediction>0.6 only; rank is not profit probability/absolute return",
            "opening_downside":"stock unbounded net proxy minus1*max(0,1-min(adjusted opening marks i+1..i+20)/entryadjustedopen), clip[-.4,.4]. Missing day mark same current last-mark convention; invalid entry zero. Opening-only adverse excursion omits intraday lows and may understate risk"},
        "execution":"unchanged10names8%full,time20+3ATRfailure/STexit,T+1,openonlyfillableproxy,fees/slippage,knownactualpay/listing,SSE+SZhistoricalSTdates; score centered at fixed label-specific threshold; no newparam selection",
        "periods":PERIODS,"annual_independent_resets":list(range(2020,2027)),"annual_double_cost":True,
        "matched_signal_pool_neutral_seeds":list(SEEDS),"baseline":"frozen22 absolute exact periods metrics must reproduce, then compare same threshold-selected pool neutral rank",
        "rejection_conditions":["no positive train and validation net after base costs","weak negative-year stability or concentration/unclosed dependence","no broad descriptive gain versus same-pool neutral;3shared-market seeds cannot prove significance"],
        "already_seen_later_history":True,"no_retuning_or_promoting_test_winner":True,"input_sha256":hashes})
    print("registered3newlabels/21annualfits; frozen22control; thresholdsfixed",flush=True)


def proxy(i,cols,raw,valid,factors,qclose):
    entry,end=i+1,i+HOLD
    op=raw["open"][entry,cols]*factors[entry,cols]
    exit_mark=np.where(valid[end,cols],raw["open"][end,cols]*factors[end,cols],qclose[end,cols])
    available=valid[entry,cols]&(raw["is_st"][entry,cols]==0)&np.isfinite(op)&(op>0)
    net=np.divide(exit_mark,op,out=np.ones(len(cols)),where=available)-1-.005
    return np.where(available,net,0),available,op


def midrank(values):
    _,inverse,counts=np.unique(values,return_inverse=True,return_counts=True)
    before=np.cumsum(counts)-counts
    return ((before[inverse]+.5*counts[inverse])/len(values)).astype(np.float32)


def labels(i,cols,raw,valid,factors,qclose,index_open,eligible_cols):
    net,available,op=proxy(i,cols,raw,valid,factors,qclose)
    benchmark=index_open[i+HOLD]/index_open[i+1]-1
    excess=np.clip(np.where(available,net-benchmark,0),-.4,.4).astype(np.float32)
    all_net,_,_=proxy(i,eligible_cols,raw,valid,factors,qclose)
    rank=midrank(all_net)[np.searchsorted(eligible_cols,cols)]
    marks=np.where(valid[i+1:i+HOLD+1,cols],raw["open"][i+1:i+HOLD+1,cols]*factors[i+1:i+HOLD+1,cols],qclose[i+1:i+HOLD+1,cols])
    trough=np.min(marks,axis=0)
    adverse=np.maximum(0,1-np.divide(trough,op,out=np.ones(len(cols)),where=available))
    risk=np.clip(np.where(available,net-adverse,0),-.4,.4).astype(np.float32)
    return {"csi_excess":excess,"cross_section_rank":rank,"opening_downside":risk}, {
        "unavailable_entry":int((~available).sum()),"missing_exit":int((~valid[i+HOLD,cols]).sum()),
        "missing_open_path_marks":int((~valid[i+1:i+HOLD+1,cols]).sum())}


def self_check():
    np.testing.assert_allclose(midrank(np.array([1,1,2,3])),[.25,.25,.625,.875])
    raw={"open":np.full((24,3),10.),"is_st":np.zeros((24,3))}
    raw["open"][20]=[11,9,10];raw["open"][10,0]=8
    valid=np.ones((24,3),bool);factors=np.ones((24,3));idx=np.full(24,100.);idx[20]=105
    y,_=labels(0,np.arange(3),raw,valid,factors,raw["open"],idx,np.arange(3))
    np.testing.assert_allclose(y["csi_excess"],[.045,-.155,-.055],atol=1e-6)
    np.testing.assert_allclose(y["opening_downside"],[-.105,-.205,-.005],atol=1e-6)
    np.testing.assert_allclose(y["cross_section_rank"],[5/6,1/6,3/6],atol=1e-6)
    copied={k:v.copy() for k,v in raw.items()};copied["open"][21:]*=5
    alt,_=labels(0,np.arange(3),copied,valid,factors,copied["open"],idx,np.arange(3))
    for k in y:np.testing.assert_array_equal(y[k],alt[k])
    valid[1,1]=False;bad,_=labels(0,np.arange(3),raw,valid,factors,raw["open"],idx,np.arange(3))
    assert bad["csi_excess"][1]==0 and bad["opening_downside"][1]==0
    days=np.array([20191227,20191230,20191231,20200102,20200103]);train,boundary=m.train_days(days,2020,2)
    assert train.tolist()==[0] and boundary==3
    print("self-check passed: exactlabels/ties/labelendpointprefix/unavailable-retention/purgedyear",flush=True)


def report(summary):
    lines=["# 目标标签机制研究：超额、排序与途中下行","",
        "本轮固定三个新标签、二十二项价格量额及广度输入、二十交易日标签和同一年度小树。只改变训练目标，沿用此前三年窗口、每天固定哈希九十六股样本、年度端点隔离和完整模拟账户。当前数据后段已多次观察；这是回顾性机制检验，所有正负结果保留，不按本轮结果改阈值或换冠军。","",
        "沪深三百超额标签以真实指数同买卖日开盘收益为基准；它学习跑赢指数而非绝对赚钱。截面排序在当日完整信号可用股票池的未来净收益代理上计算并列中位分位，固定预测分位高于零点六才观察；排序分数不代表上涨概率。途中下行标签从净收益代理扣一倍二十日调整后开盘路径最大负偏离，未使用日内最低价，所以并不认证完整最大回撤。","",
        "标签都是调整后行情的理想代理，包含固定百分之零点五成本，并对收益类标签截断至正负百分之四十。无效次日开盘记零，缺失退出继续保留已有最后估值标记，避免用未来完整性挑存活股；这仍有实际可成交性、公司行动和退市回收局限。真实组合同时考虑十个槽位、每股百分之八目标、费用滑点、整手、T加一、涨跌停、已验证付款及上市日期、三倍ATR保护和ST退出。","",
        "|标签版本|前段净收益%|中段净收益%|已见后段净收益%|后段DD%|双成本后段%|周期/股票|同池三种子中位%|",
        "|---|---:|---:|---:|---:|---:|---|---:|"]
    for name,item in summary["experiments"].items():
        p=item["periods"];d=p["test"]["cycle_diagnostics"];b=summary["neutral_baselines"][name]["test"]
        lines.append(f"|{name}|{p['train']['metrics']['net_return_pct']}|{p['validation']['metrics']['net_return_pct']}|{p['test']['metrics']['net_return_pct']}|{p['test']['metrics']['max_drawdown_pct']}|{item['double_cost']['metrics']['net_return_pct']}|{d['completed_cycles']}/{d['distinct_completed_cycle_stocks']}|{b['median_return_pct']}|")
    lines.extend(["","每年独立账户结果、双成本、亏损年份、分股票周期损益/持有期分布、盈利集中度及未平仓含应收占比均存于 summary.json。不同标签的观察池会改变，不能把标签间收益差全部归于排序能力；同一标签的三种子只替换其观察池内部次序，不能当作独立市场或显著性证据。年度账户不能拼为真实连续复利。","",
        "此前冻结绝对收益二十二输入对照的三段绩效完全复现："+str(summary["checks"]["frozen22_exact_metrics_reproduced"])+"。本轮没有生产准入，没有在产品里替换策略。","",
        "## 固定否定条件与结论","",
        "若训练及验证不能同时在含成本组合取得正收益，否定其当前直接替换资格；若收益依赖少数股票或未兑现权益、年度稳定性不足、同池中性排序未表现出持续优势，继续保留为研究假说。即使描述性检查通过，已见后段也不能提供全新样本验证或准入证明。",""])
    for name,decision in summary["mechanism_decisions"].items():lines.append(f"- {name}：{decision['interpretation']}；前段和中段均正={decision['train_validation_positive']}；后段模型减同池种子中位={decision['test_model_minus_pool_median_pp']:.3f}个百分点。")
    lines.extend(["","完整证据：[summary.json](summary.json)、[preregistered.json](preregistered.json)、[manifest.json](manifest.json)及 ledgers/annual-ledgers/neutral-ledgers。模型和分值缓存仅为新研究可复核产物。"])
    (ROOT/"RESULTS.md").write_text("\n".join(lines)+"\n",encoding="utf-8")


def run():
    started=time.time();self_check()
    if (ROOT/"summary.json").exists():raise ValueError("已有结果，拒绝覆盖")
    prereg=json.loads((ROOT/"preregistered.json").read_text(encoding="utf-8"));paths=sources();hashes={k:r.sha(p) for k,p in paths.items()}
    if hashes!=prereg["input_sha256"]:raise ValueError("预登记后输入SHA变化")
    baseline=json.loads(paths["baseline_summary"].read_text(encoding="utf-8"))
    assert hashes["snapshot"]==baseline["input_sha256_start"]["snapshot"]
    assert hashes["frozen22_scores"]==baseline["experiments"]["breadth22_h20"]["scores_sha256"]
    with np.load(paths["snapshot"],allow_pickle=False) as data:
        codes=data["codes"].tolist();dates=data["dates"];raw={k.removeprefix("raw_"):data[k] for k in data.files if k.startswith("raw_")}
        valid,seen,factors=data["valid"],data["seen"],data["factors"]
    assert int(dates[-1])==20260930 and all(s.STOCK.fullmatch(c) for c in codes)
    meta=json.loads(paths["metadata"].read_text(encoding="utf-8"));events={(v["i"],v["c"]):v["event"] for v in meta["events"]}
    details,action_coverage=rules.action_map(codes,dates,events);rules.SZ_EFFECTIVE=20260706;rules.self_check()
    f=s.features(raw,valid,factors,seen);_,arrays,market=m.inputs(raw,valid,f)
    index,_=r.cli.index_inputs(dates,paths["benchmark"],np,s)
    benchmark_rows=[json.loads(v) for v in paths["benchmark"].read_text(encoding="utf-8").splitlines() if v]
    assert [int(v["date"].replace("-","")) for v in benchmark_rows]==dates.tolist()
    assert all(v["code"]=="sh.000300" for v in benchmark_rows)
    index_open=np.array([v["open"] for v in benchmark_rows],float);assert np.all(np.isfinite(index_open)&(index_open>0))
    samples=m.sample_rows(codes,dates,f["eligible"]);xs=[r.xday(arrays,market,index,i,cols,"breadth22") for i,cols in enumerate(samples)]
    targets={spec["label"]:[] for spec in CONFIGS.values() if spec["fit"]};label_audit=[]
    for i,cols in enumerate(samples):
        if len(cols) and i+HOLD<len(dates):
            ys,audit=labels(i,cols,raw,valid,factors,f["close"],index_open,np.flatnonzero(f["eligible"][i]));label_audit.append({"signal_date":int(dates[i]),**audit})
            for label in targets:targets[label].append(ys[label]);assert np.isfinite(ys[label]).all()
        else:
            for label in targets:targets[label].append(None)
    summary={"schema":"ashare-label-study-v2-results","as_of":"2026-09-30","exploration":True,"production_admission":False,
        "input_sha256_start":hashes,"quality":meta["quality"],"action_coverage":action_coverage,"label_audit":label_audit,
        "configs":CONFIGS,"fits":[],"experiments":{},"annual":{},"annual_double_cost":{},"neutral_baselines":{},"failures":[],
        "limitations":["laterhistoryalreadyseen","alllabelsidealizednotexecutablePnL","currentST/openavailabilitytimeuncertified","incompleteactualactions/rights/retirementcashrecovery","openingrisklabelnotintradaydrawdown","rankandexcesspositivitynotabsoluteprofit","3neutralsharedmarketseedsnotindependent","annualresetsnotcontinuousaccount"]}
    cache=ROOT/".cache";models=cache/"models";models.mkdir(parents=True,exist_ok=True)
    scores={"frozen22_absolute":np.load(paths["frozen22_scores"],mmap_mode="r")}
    for name,spec in CONFIGS.items():
        if not spec["fit"]:continue
        score=np.full(valid.shape,np.nan,np.float32)
        for year in range(2020,2027):
            train,boundary=m.train_days(dates,year,HOLD);train=[int(i) for i in train if len(samples[i])]
            x=np.concatenate([xs[i] for i in train]);y=np.concatenate([targets[spec["label"]][i] for i in train])
            model=m.fit("hist_tree",x,y);path=models/f"{name}_y{year}.pkl"
            with path.open("wb") as stream:pickle.dump(model,stream)
            for i in np.flatnonzero(dates//10000==year):
                cols=np.flatnonzero(f["eligible"][i])
                if len(cols):score[i,cols]=m.predict(model,r.xday(arrays,market,index,int(i),cols,"breadth22"))
            summary["fits"].append({"config":name,"prediction_year":year,"rows":len(y),"features":x.shape[1],
                "first_training_signal":int(dates[train[0]]),"last_training_signal":int(dates[train[-1]]),"last_actual_label_endpoint":int(dates[train[-1]+HOLD]),
                "training_day_indices_sha256":r.hashlib.sha256(np.array(train,np.int32).tobytes()).hexdigest(),
                "training_targets_sha256":r.hashlib.sha256(y.tobytes()).hexdigest(),"model_sha256":r.sha(path)})
            print("fit",name,year,len(y),"elapsed",round(time.time()-started,1),flush=True)
        np.save(cache/f"{name}-scores.npy",score,allow_pickle=False);scores[name]=score
        save(ROOT/"fit-progress.json",summary["fits"])
    accounts=[];buy_count=0;ledger_count=0;old_open=e.tradable_open;e.tradable_open=rules.tradable_open
    def replay(name,execution_score,span,cost=1,path=None):
        nonlocal buy_count,ledger_count
        f["score"]=execution_score;f["signals"][name]=f["eligible"]&np.isfinite(execution_score)&(execution_score>0)
        ledger=e.backtest(name,*span,codes,dates,raw,valid,factors,events,f,multiplier=cost,full_entry=True,hold=HOLD,journal=True,exit_policy="time",max_positions=10,action_details=details)
        account=rules.accounting(ledger);assert account["minimum_cash_cny"]>=0 and abs(account["reconcile_cny"])<.1
        accounts.append(account);rules.GATE_CHANGES.clear()
        for order in ledger["orders"]:
            c=codes.index(order["code"]);at=int(np.searchsorted(dates,order["date"]));assert s.STOCK.fullmatch(order["code"])
            if order["side"]=="buy":assert raw["is_st"][at,c]==0;buy_count+=1
        if path:save(path,ledger);ledger_count+=1
        return {"metrics":ledger["metrics"],"accounting":{k:v for k,v in account.items() if k!="unclosed"},"cycle_diagnostics":r.cycles_audit(ledger)}
    try:
        for name,spec in CONFIGS.items():
            signal_score=scores[name]-spec["threshold"]
            periods={p:replay(name,signal_score,span,path=ROOT/"ledgers"/f"{name}-{p}.json") for p,span in PERIODS.items()}
            double=replay(name,signal_score,PERIODS["test"],2,ROOT/"ledgers"/f"{name}-test-double.json")
            summary["experiments"][name]={"spec":spec,"periods":periods,"double_cost":double,"scores_sha256":hashes["frozen22_scores"] if not spec["fit"] else r.sha(cache/f"{name}-scores.npy")}
            summary["annual"][name]={str(y):replay(name,signal_score,(y*10000+101,min(y*10000+1231,20260930)),path=ROOT/"annual-ledgers"/f"{name}-{y}.json") for y in range(2020,2027)}
            summary["annual_double_cost"][name]={str(y):replay(name,signal_score,(y*10000+101,min(y*10000+1231,20260930)),2,ROOT/"annual-ledgers"/f"{name}-{y}-double.json") for y in range(2020,2027)}
            save(ROOT/"progress.json",summary);print("replay",name,{p:v["metrics"]["net_return_pct"] for p,v in periods.items()},flush=True)
            pool=f["eligible"]&np.isfinite(signal_score)&(signal_score>0)
            seed_results=[]
            for seed in SEEDS:
                neutral=r.neutral_rank(codes,dates,pool,seed)
                seeds={p:replay(f"neutral_{name}_{seed}",neutral,span,path=ROOT/"neutral-ledgers"/f"{name}-seed{seed}-{p}.json") for p,span in PERIODS.items()}
                seeds["test_double_cost"]=replay(f"neutral_{name}_{seed}",neutral,PERIODS["test"],2,ROOT/"neutral-ledgers"/f"{name}-seed{seed}-test-double.json")
                seed_results.append({"seed":seed,"periods":seeds})
            save(ROOT/"neutral-results"/f"{name}.json",seed_results)
            summary["neutral_baselines"][name]={p:{"seed_returns":[row["periods"][p]["metrics"]["net_return_pct"] for row in seed_results],
                "median_return_pct":float(np.median([row["periods"][p]["metrics"]["net_return_pct"] for row in seed_results])),
                "model_return_pct":periods[p]["metrics"]["net_return_pct"] if p in PERIODS else double["metrics"]["net_return_pct"]} for p in list(PERIODS)+["test_double_cost"]}
    finally:e.tradable_open=old_open
    reproduced=[]
    for p in PERIODS:
        assert summary["experiments"]["frozen22_absolute"]["periods"][p]["metrics"]==baseline["experiments"]["breadth22_h20"]["periods"][p]["metrics"],p
        reproduced.append(p)
    assert summary["experiments"]["frozen22_absolute"]["double_cost"]["metrics"]==baseline["experiments"]["breadth22_h20"]["double_cost"]["metrics"]
    fitgroups=[[v for v in summary["fits"] if v["config"]==name] for name,spec in CONFIGS.items() if spec["fit"]]
    assert all(len({rows[k]["training_day_indices_sha256"] for rows in fitgroups})==1 and len({rows[k]["rows"] for rows in fitgroups})==1 for k in range(7))
    after={k:r.sha(p) for k,p in paths.items()};assert after==hashes
    summary["input_sha256_end"]=after
    summary["checks"]={"new_model_fits":21,"saved_ledgers":ledger_count,"portfolio_replays":len(accounts),"buy_orders_checked":buy_count,
        "no_ST_or_BJ_buy_orders":True,"nonnegative_cash_all":True,"max_PnL_reconcile_cny":max(abs(v["reconcile_cny"]) for v in accounts),
        "same_training_samples_all3labels":True,"all_label_endpoints_before_prediction_year":all(v["last_actual_label_endpoint"]<v["prediction_year"]*10000+101 for v in summary["fits"]),
        "CSI_explicit_code_exact_market_dates":True,"frozen22_exact_metrics_reproduced":reproduced+["test_double_cost"],"all_source_SHA_unchanged":True,"elapsed_seconds":round(time.time()-started,3)}
    summary["mechanism_decisions"]={name:{"train_validation_positive":all(item["periods"][p]["metrics"]["net_return_pct"]>0 for p in ("train","validation")),
        "negative_annual_years":[y for y,v in summary["annual"][name].items() if v["metrics"]["net_return_pct"]<=0],
        "test_model_minus_pool_median_pp":item["periods"]["test"]["metrics"]["net_return_pct"]-summary["neutral_baselines"][name]["test"]["median_return_pct"],
        "interpretation":"保留回顾性机制结果，不据后段挑选冠军或准入；对照用于分离观察池与内部排序"} for name,item in summary["experiments"].items()}
    save(ROOT/"summary.json",summary);report(summary)
    rows=[]
    for name,spec in CONFIGS.items():
        for c in np.flatnonzero(f["eligible"][-1]&np.isfinite(scores[name][-1])):
            rows.append({"model":name,"symbol":("sh" if codes[c].startswith(("60","68")) else "sz")+codes[c],
                "as_of":"2026-09-30","prediction":float(scores[name][-1,c]),"threshold":spec["threshold"],"above_fixed_threshold":bool(scores[name][-1,c]>spec["threshold"]),
                "label":spec["label"],"production_admission":False,"score_cache_sha256":summary["experiments"][name]["scores_sha256"]})
    (ROOT/"current-scores.ndjson").write_text("".join(json.dumps(v,ensure_ascii=False,allow_nan=False)+"\n" for v in rows),encoding="utf-8")
    save(ROOT/"manifest.json",{"schema":"ashare-label-study-v2-artifacts","completed_at_utc":dt.datetime.now(dt.timezone.utc).isoformat(),
        "as_of":"2026-09-30","input_sha256_start":hashes,"input_sha256_end":after,"all_inputs_unchanged":True,
        "files":{str(p.relative_to(ROOT)):{"sha256":r.sha(p),"bytes":p.stat().st_size} for p in ROOT.rglob("*") if p.is_file() and '__pycache__' not in p.parts and p.name!="manifest.json"}})
    print("done",summary["checks"],flush=True)


if __name__=="__main__":
    parser=argparse.ArgumentParser();parser.add_argument("--register",action="store_true");parser.add_argument("--self-check",action="store_true");args=parser.parse_args()
    if args.self_check:self_check()
    elif args.register:register()
    else:run()
