"""Fixed technical ablations / annual purged A-share models and portfolio choices.

Research only. All later history is already observed exploratory evidence.
Original snapshot, models, execution source and product files are read-only.
"""
import argparse
import collections
import datetime as dt
import hashlib
import json
import math
import pickle
import sys
import time
from pathlib import Path

ROOT=Path(__file__).resolve().parent
OPEN=ROOT.parent/"ashare-open-2026-10-01"
sys.path.insert(0,str(OPEN/"learning"));sys.path.insert(0,str(OPEN/"rule-study"));sys.path.insert(0,str(OPEN))
import rolling_models as m
import absolute_models as a
import rules_execution as rules
import research_cli as cli
np,s,e=m.np,m.s,rules.e
DATA=OPEN/"stockdb-live/exports"
HOLDS=(5,10,15,20,30)
GROUPS={"stock14":14,"breadth22":22,"index26":26}
SEEDS=tuple(range(5))
PERIODS={"train":(20200101,20221231),"validation":(20230101,20241231),"test":(20250101,20260930)}
FEATURES=["r1","previous5","r5","r20","r60","distance_ma20","distance_prior60high","atr_pct","volume_ratio","close_location","body_atr","upper_wick_atr","lower_wick_atr","amount20_log",
    "market_breadth_centered","market_breadth_change5","market_breadth20_change20",
    "r1_x_breadth","previous5_x_breadth","r60_x_breadth","atr_pct_x_breadth","volume_ratio_x_breadth",
    "hs300_r5","hs300_r20","hs300_distance_ma60","hs300_volatility20"]


def sha(path):
    h=hashlib.sha256()
    with Path(path).open("rb") as f:
        for b in iter(lambda:f.read(4*1024*1024),b""):h.update(b)
    return h.hexdigest()


def save(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(value,ensure_ascii=False,indent=2,allow_nan=False,default=lambda x:x.item() if isinstance(x,np.generic) else (_ for _ in ()).throw(TypeError(type(x).__name__))),encoding="utf-8")


def xday(arrays,market,index,i,cols,group):
    # All feature groups are ordered prefixes of the original pure-technical26.
    return a.xday(arrays,market,index,i,cols)[:,:GROUPS[group]]


def target(i,cols,hold,raw,valid,factors,qclose):
    """Signal i, entry i+1, intended inclusive holding session hold at i+hold."""
    return a.target(i,cols,hold,raw,valid,factors,qclose)


def distribution(values):
    x=np.array([v for v in values if v is not None and math.isfinite(float(v))],float)
    if not len(x):return {"n":0,"mean":None,"q10":None,"median":None,"q90":None,"min":None,"max":None}
    return {"n":len(x),"mean":round(float(x.mean()),6),"q10":round(float(np.quantile(x,.1)),6),"median":round(float(np.median(x)),6),"q90":round(float(np.quantile(x,.9)),6),"min":round(float(x.min()),6),"max":round(float(x.max()),6)}


def cycles_audit(ledger):
    rows=ledger["cycles"];stock=collections.defaultdict(list);years=collections.defaultdict(list)
    for row in rows:stock[row["code"]].append(row);years[str(row["exit_date"]//10000)].append(row)
    def group_stats(items):
        return {"cycles":len(items),"net_pnl_cny":round(sum(r["net_pnl"] for r in items),3),"mean_cycle_net_pct":round(float(np.mean([r["net_pct"] for r in items])),6),
            "win_rate_pct":round(100*sum(r["net_pnl"]>0 for r in items)/len(items),3),"mean_holding_sessions":round(float(np.mean([r["holding_sessions"] for r in items])),3),"overdue_cycles":sum(r["overdue"]>0 for r in items)}
    stock_stats={code:group_stats(items) for code,items in sorted(stock.items())}
    positives=sorted([r["net_pnl"] for r in rows if r["net_pnl"]>0],reverse=True);gross=sum(positives)
    positive_by_stock=sorted([sum(max(0,r["net_pnl"]) for r in items) for items in stock.values()],reverse=True)
    unclosed=sum(r["unrealized_and_receivable_pnl"] for r in ledger["unclosed"])
    net=ledger["curve"][-1]["equity"]-100000
    return {"completed_cycles":len(rows),"distinct_completed_cycle_stocks":len(stock_stats),"cycle_net_pct":distribution([r["net_pct"] for r in rows]),"holding_sessions":distribution([r["holding_sessions"] for r in rows]),
        "stock_net_pnl_quantiles":distribution([r["net_pnl_cny"] for r in stock_stats.values()]),"stock_mean_cycle_net_pct_quantiles":distribution([r["mean_cycle_net_pct"] for r in stock_stats.values()]),
        "by_exit_year":{year:group_stats(items) for year,items in sorted(years.items())},"by_stock":stock_stats,
        "concentration":{"top5_win_cycles_share_gross_gain_pct":sum(positives[:5])/gross*100 if gross else None,"top10_stocks_share_gross_gain_pct":sum(positive_by_stock[:10])/gross*100 if gross else None,"gross_positive_completed_pnl_cny":round(gross,3)},
        "unclosed_pnl_cny":round(unclosed,3),"unclosed_pnl_divided_total_net_pnl_pct":unclosed/net*100 if net else None,
        "warning":"Stock quantiles are per-stock aggregates; cycles/year/stocks share the same market path and are not independent experimental replications"}


def model_selection(annual,year):
    prior=list(range(year-3,year));objectives={}
    for name,rows in annual.items():
        history=[rows[str(y)]["metrics"] for y in prior]
        count=sum(v["completed_holding_cycles"] for v in history)
        expectancy=sum((v["mean_cycle_net_pct"] or 0)*v["completed_holding_cycles"] for v in history)/count if count else -1e9
        objectives[name]={"positive_years":sum(v["net_return_pct"]>0 for v in history),"mean_cycle_net_pct_weighted":expectancy,
            "worst_annual_return_pct":min(v["net_return_pct"] for v in history),"max_annual_drawdown_pct":max(v["max_drawdown_pct"] for v in history),
            "mean_annual_return_pct":float(np.mean([v["net_return_pct"] for v in history])),"completed_cycles":count,"prior_years":prior}
    def key(name,style):
        x=objectives[name]
        if style=="expectancy":return (x["mean_cycle_net_pct_weighted"],x["positive_years"],-x["max_annual_drawdown_pct"],x["mean_annual_return_pct"])
        if style=="cross_year":return (x["positive_years"],x["worst_annual_return_pct"],x["mean_cycle_net_pct_weighted"],-x["max_annual_drawdown_pct"])
        return (-x["max_annual_drawdown_pct"],x["positive_years"],x["mean_annual_return_pct"],x["mean_cycle_net_pct_weighted"])
    return {style:{"config":max(sorted(objectives),key=lambda name:key(name,style)),"objectives":objectives} for style in ("expectancy","cross_year","drawdown")}


def neutral_rank(codes,dates,eligible,seed):
    out=np.full(eligible.shape,np.nan,dtype=np.float64)
    for i,day in enumerate(dates):
        if day<20200101:continue
        cols=np.flatnonzero(eligible[i]);prefix=f"ablation-neutral-v1|seed={seed}|signal_date={int(day)}|code=".encode("ascii")
        out[i,cols]=[int.from_bytes(hashlib.sha256(prefix+codes[c].encode("ascii")).digest()[:6],"big") for c in cols]
    return out


def self_check():
    dates=np.array([20191227,20191230,20191231,20200102,20200103,20200106])
    rows,boundary=m.train_days(dates,2020,2);assert rows.tolist()==[0] and boundary==3
    raw={"open":np.full((8,2),10.),"is_st":np.zeros((8,2))};raw["open"][4]=[11,9]
    y=target(1,np.array([0,1]),3,raw,np.ones((8,2),bool),np.ones((8,2)),raw["open"])
    np.testing.assert_allclose(y,[.095,-.105],atol=1e-6) # entry2, exit4, inclusive3sessions.
    arrays=[np.arange(15,dtype=np.float32).reshape(5,3)+n for n in range(16)]
    market=np.ones((5,3),np.float32);index=np.ones((5,4),np.float32)
    for group,width in GROUPS.items():assert xday(arrays,market,index,3,np.array([0,2]),group).shape==(2,width)
    eligible=np.ones((3,2),bool);d=np.array([20200102,20200103,20200106]);codes=["600001","000001"]
    h=neutral_rank(codes,d,eligible,0);np.testing.assert_array_equal(h[:2],neutral_rank(codes,d[:2],eligible[:2],0))
    np.testing.assert_array_equal(h,neutral_rank(codes[::-1],d,eligible,0)[:,::-1])
    annual={n:{str(y):{"metrics":{"completed_holding_cycles":10,"mean_cycle_net_pct":(1 if n=="a" else .2),"net_return_pct":(5 if n=="a" else 1),"max_drawdown_pct":(10 if n=="a" else 2)}} for y in range(2020,2024)} for n in ("a","b")}
    selected=model_selection(annual,2023)
    annual["b"]["2023"]["metrics"]["net_return_pct"]=999
    assert selected==model_selection(annual,2023) and selected["expectancy"]["config"]=="a" and selected["drawdown"]["config"]=="b"
    print("self-check passed: purged endpoint, inclusive holding label,14/22/26 dimensions,hash prefix/order invariance, annual choice ignores evaluation year",flush=True)


def report(summary):
    lines=["# 多年多股票的绝对收益模型机制消融","",
        "固定15个配置：股票自身价格/量额14输入、再加市场广度及技术交互22输入、再加显式沪深300指数26输入；持有5/10/15/20/30交易日。每预测年按此前可获得的三年窗口训练固定小树，标签端点严格在预测年之前。2020起点只可获得2018—2019两年，不伪称三个完整年。没有参数网格或后段冠军推荐。","",
        "所有组合只研究沪深普通股，按各日历史ST过滤、保留退市历史，十槽位各8%目标，整仓、计划持有标签同长度、3ATR失效保护、T+1及实际费用/滑点代理；沪深主板ST价格限制按官方生效日期，不买ST。基本面历史版本未认证，本轮不使用估值或当前行业回填历史。","",
        "训练2020—2022、验证2023—2024、后段2025—2026-09-30，每段独立10万元。后段已反复观察，本轮仍是回顾性探索。标签是调整后开盘收益减0.5%的代理，真实组合会受3ATR提前退出、限幅、现金、费用和未知公司行动锁定影响；标签不等于可执行账户损益。","",
        "|输入/持有|训练收益%|验证收益%|后段收益%|后段回撤%|双成本收益%|后段周期均值%|后段胜率%|完成周期/股票|",
        "|---|---:|---:|---:|---:|---:|---:|---:|---|"]
    for name,item in summary["experiments"].items():
        p=item["periods"];d=p["test"]["cycle_diagnostics"]
        lines.append(f"|{name}|{p['train']['metrics']['net_return_pct']}|{p['validation']['metrics']['net_return_pct']}|{p['test']['metrics']['net_return_pct']}|{p['test']['metrics']['max_drawdown_pct']}|{item['double_cost']['metrics']['net_return_pct']}|{p['test']['metrics']['mean_cycle_net_pct']}|{p['test']['metrics']['win_rate_pct']}|{d['completed_cycles']}/{d['distinct_completed_cycle_stocks']}|")
    lines += ["","## 每年开始前冻结选择","",
        "每年2023—2026只用此前三个年度独立重置组合回放：期望风格先比较完成周期加权净期望；跨年风格先比较正收益年度数和最差年度；回撤风格先比较最坏年度回撤。指标是预先固定词典序多目标，不以DD15硬筛。空周期期望记为不可比较的极低值；回撤风格仍可能选择低暴露或负期望配置，不能把低回撤当获利资格。各年选择随后冻结，与该年结果分开保存。年度净收益不能直接拼成连续复利账户。","",
        "|年度|风格|冻结配置|该年收益%|该年回撤%|双成本收益%|同正分池seed中位收益%|模型数值分位%|","|---|---|---|---:|---:|---:|---:|---:|"]
    for year,styles in summary["annual_frozen_selection"].items():
        for style,item in styles.items():
            model=item["evaluation"]["metrics"];base=item["matched_positive_baselines"]["base_cost"]["return_distribution"]
            lines.append(f"|{year}|{style}|{item['config']}|{model['net_return_pct']}|{model['max_drawdown_pct']}|{item['double_cost']['metrics']['net_return_pct']}|{base['median']}|{base['model_midrank_percentile']}|")
    lines += ["","## 同正分股票池中性排序与股票分散性","",
        "固定seed0—4、SHA256(股票代码、已完成信号日、seed)排序，只替换同一个模型正分股票池内部排名，执行、持有期、现金制度和成本相同。年度对照直接使用原冻结配置，不在基线里重新挑模型或seed。五个seed共用历史市场，不能当五份独立样本或显著性证据。",""]
    for name,item in summary["experiments"].items():
        d=item["periods"]["test"]["cycle_diagnostics"];base=item["matched_positive_baselines"]["test"]["return_distribution"]
        lines.append(f"- {name}：同池seed后段收益中位 {base['median']}%，模型分位 {base['model_midrank_percentile']}%；完成周期涉及 {d['distinct_completed_cycle_stocks']} 只股票，前5盈利周期占正盈利 {d['concentration']['top5_win_cycles_share_gross_gain_pct']}%，前10只股票占正盈利 {d['concentration']['top10_stocks_share_gross_gain_pct']}%。")
    lines += ["","## 输入重要性与限制","",
        "重要性只在2024验证年度的固定hash样本测一次变量置乱后MSE增量，使用该年之前训练的模型；未用2025以后样本。置乱影响是模型预测依赖的诊断，有相关变量和分布破坏问题，不是变量的因果收益或真实交易贡献。完整逐变量、逐输入组结果与负重要性保留。","",
        "逐年、逐股票完成周期分位、持有期、胜率、盈利集中度、未平仓含应收占比及双成本都在summary.json中。31处历史公司行动公式疑点、部分现金支付/新增股上市日期缺失、退市回收估值、历史ST标签开盘前可知时点与实际竞价深度仍未认证。未平仓及应收是净值的一部分，不是已实现提款收益。2026年仅截至9/30。","",
        f"固定模型训练 {len(summary['fits'])} 次；所有15配置均保留。源输入SHA未变={summary['inputs_unchanged']}，最小现金 {summary['minimum_cash_cny']} 元，最大已完成/未平仓损益与净值舍入差 {summary['max_reconcile_cny']} 元。没有自动实盘交易或生产准入。"]
    (ROOT/"RESULTS.md").write_text("\n".join(lines)+"\n",encoding="utf-8")


def main(resume=False):
    began=time.time();self_check();ROOT.mkdir(exist_ok=True)
    cache=ROOT/".cache";cache.mkdir(exist_ok=True);models=cache/"models";models.mkdir(exist_ok=True)
    sources={"runner":Path(__file__),"snapshot":DATA/"matrices.npz","metadata":DATA/"matrices-metadata.json","relative_features":Path(m.__file__),"absolute_features":Path(a.__file__),"study":Path(s.__file__),"execution":Path(e.__file__),"rules":Path(rules.__file__),"action_map":OPEN/"execution_study.py","action_overlay":OPEN/"online/action-overlay.json","index":OPEN/"online/index-sh-000300.ndjson","sse_sources":OPEN/"online/rules/sources.json","szse_sources":OPEN/"online/rules/szse-sources.json"}
    hashes={name:sha(path) for name,path in sources.items()}
    if not resume:
        save(ROOT/"preregistered.json",{"created_at_utc":dt.datetime.now(dt.timezone.utc).isoformat(),"status":"retrospective exploration, later history already seen","groups":GROUPS,"feature_names":FEATURES,"holds":HOLDS,"configs":15,"annual_fits":105,"training":"annual previous3calendar-years, no data before2018, strict labelendpoint before predictionyear;96fixedhashstocks/day","tree":{"max_iter":80,"learning_rate":.07,"max_leaf_nodes":15,"min_samples_leaf":80,"l2_regularization":1,"early_stopping":False,"random_state":20261001},"label":"next open i+1 to intended inclusive holding exit i+hold adjusted return minus.005, clip40%; invalidentryzero/missingexitlastmark, not actual execution PnL","execution":"10slots8%target,full,time+3ATR,T+1,fees/slippage,latestverifiedactions,SSE+SZhistoricalSTdates","selection":"2023..2026 choose using previous3annual independent-reset portfolios; lexicographic expectancy/positiveyears/worstDD styles; no DD15 cutoff; ties by configuration sorted name","seeds":SEEDS,"baseline":"matched finite-positive same-model pool; same hold/rules/config frozen yearly; no seed selection","importance":"2024validation only, fixed1/8of96hash/day sample, endpointwithin2024, one fixedseed column permutation/MSEdelta; not causal","input_sha256":hashes,"production_admission":False})
    with np.load(DATA/"matrices.npz",allow_pickle=False) as data:
        codes,dates=data["codes"].tolist(),data["dates"]
        raw={k.removeprefix("raw_"):data[k] for k in data.files if k.startswith("raw_")};seen,valid,factors=data["seen"],data["valid"],data["factors"]
    meta=json.loads((DATA/"matrices-metadata.json").read_text(encoding="utf-8"));events={(r["i"],r["c"]):r["event"] for r in meta["events"]}
    details,action_coverage=rules.action_map(codes,dates,events)
    rules.SZ_EFFECTIVE=20260706;rules.self_check()
    f=s.features(raw,valid,factors,seen);names,arrays,market=m.inputs(raw,valid,f);index,last_index=cli.index_inputs(dates,sources["index"],np,s)
    samples=m.sample_rows(codes,dates,f["eligible"])
    xs=[a.xday(arrays,market,index,i,cols) for i,cols in enumerate(samples)]
    summary={"schema":"ashare-technical-ablation15-v1","input_sha256_start":hashes,"quality":meta["quality"],"action_coverage":action_coverage,"feature_groups":GROUPS,"feature_names":FEATURES,"experiments":{},"annual":{},"fits":[],"failures":[],"importance_validation2024":{},"production_admission":False,"latest_index":last_index}
    all_accounts=[];old_open=e.tradable_open;e.tradable_open=rules.tradable_open
    if resume:
        summary=json.loads((ROOT/"progress.json").read_text(encoding="utf-8"))
        assert len(summary["experiments"])==15 and len(summary["fits"])==105
        old_hashes=summary["input_sha256_start"]
        assert all(old_hashes[k]==v for k,v in hashes.items() if k!="runner")
        for name,item in summary["experiments"].items():
            assert sha(cache/f"{name}_scores.npy")==item["scores_sha256"]
            all_accounts.extend(row["accounting"] for row in item["periods"].values());all_accounts.append(item["double_cost"]["accounting"])
            all_accounts.extend(row["accounting"] for row in summary["annual"][name].values())
        for fit in summary["fits"]:assert sha(models/f"{fit['config']}_y{fit['prediction_year']}.pkl")==fit["model_sha256"]
        save(ROOT/"resume-provenance.json",{"reason":"Completed105fits/165portfolio runs, then NumPy int64 selection-count JSON encoding failed; scalar conversion and verified checkpoint resume only","original_training_input_sha256":old_hashes,"resume_input_sha256":hashes,"score_and_model_hashes_verified":True,"models_refitted_on_resume":False,"original_prereg_unchanged":True})
        summary["input_sha256_start"]=hashes
    def run(name,score,hold,span,cost=1,path=None,diagnostics=True):
        f["score"]=score;f["signals"][name]=f["eligible"]&np.isfinite(score)&(score>0)
        result=e.backtest(name,*span,codes,dates,raw,valid,factors,events,f,multiplier=cost,full_entry=True,hold=hold,journal=True,exit_policy="time",max_positions=10,action_details=details)
        account=rules.accounting(result);all_accounts.append(account)
        assert account["minimum_cash_cny"]>=0 and abs(account["reconcile_cny"])<.1
        if path:save(path,result)
        rules.GATE_CHANGES.clear()
        row={"metrics":result["metrics"],"accounting":{k:v for k,v in account.items() if k!="unclosed"}}
        if diagnostics:row["cycle_diagnostics"]=cycles_audit(result)
        return row
    try:
        for group,width in GROUPS.items():
            if resume:continue
            for hold in HOLDS:
                name=f"{group}_h{hold}";score=np.full(valid.shape,np.nan,np.float32)
                labels=[target(i,cols,hold,raw,valid,factors,f["close"]) if len(cols) and i+hold<len(dates) else None for i,cols in enumerate(samples)]
                for year in range(2020,2027):
                    train,boundary=m.train_days(dates,year,hold);train=[int(i) for i in train if len(samples[i])]
                    x=np.concatenate([xs[i][:,:width] for i in train]);y=np.concatenate([labels[i] for i in train])
                    model=m.fit("hist_tree",x,y)
                    path=models/f"{name}_y{year}.pkl"
                    with path.open("wb") as handle:pickle.dump(model,handle)
                    predicted=np.flatnonzero((dates>=year*10000+101)&(dates<=year*10000+1231))
                    for i in predicted:
                        cols=np.flatnonzero(f["eligible"][i])
                        if len(cols):score[i,cols]=m.predict(model,xday(arrays,market,index,int(i),cols,group))
                    badexit=sum(int((~valid[i+hold,samples[i]]).sum()) for i in train)
                    summary["fits"].append({"config":name,"prediction_year":year,"rows":len(y),"first_training_signal":int(dates[train[0]]),"last_training_signal":int(dates[train[-1]]),"last_actual_label_endpoint":int(dates[train[-1]+hold]),"prediction_boundary":year*10000+101,"sampled_missing_exit_rows":badexit,"model_sha256":sha(path)})
                    print("fit",name,year,len(y),"elapsed",round(time.time()-began,1),flush=True)
                    if year==2024:
                        at=np.flatnonzero((dates>=20240101)&(np.arange(len(dates))+hold<int(np.searchsorted(dates,20250101))))
                        chosen=[(int(i),samples[i][::8]) for i in at if len(samples[i])]
                        ix=np.concatenate([xday(arrays,market,index,i,cols,group) for i,cols in chosen]);iy=np.concatenate([target(i,cols,hold,raw,valid,factors,f["close"]) for i,cols in chosen])
                        baseline=float(np.mean((m.predict(model,ix)-iy)**2));permutation=[]
                        rng=np.random.default_rng(20261001)
                        for c in range(width):
                            changed=ix.copy();changed[:,c]=changed[rng.permutation(len(changed)),c]
                            mse=float(np.mean((m.predict(model,changed)-iy)**2))
                            permutation.append({"feature":FEATURES[c],"mse_increase":mse-baseline,"permuted_mse":mse})
                        summary["importance_validation2024"][name]={"rows":len(iy),"baseline_mse":baseline,"permutation":permutation,"last_label_endpoint":int(max(dates[i+hold] for i,_ in chosen)),"sample_policy":"12of96fixedcode/datehashstocks per session","future_test_not_used":True}
                np.save(cache/f"{name}_scores.npy",score,allow_pickle=False)
                periods={p:run(name,score,hold,span,path=ROOT/"ledgers"/f"{name}-{p}.json") for p,span in PERIODS.items()}
                double=run(name,score,hold,PERIODS["test"],cost=2,path=ROOT/"ledgers"/f"{name}-test-double.json")
                summary["experiments"][name]={"group":group,"hold":hold,"scores_sha256":sha(cache/f"{name}_scores.npy"),"periods":periods,"double_cost":double}
                summary["annual"][name]={str(year):run(name,score,hold,(year*10000+101,min(year*10000+1231,20260930)),path=ROOT/"annual-ledgers"/f"{name}-{year}.json") for year in range(2020,2027)}
                save(ROOT/"progress.json",summary);print("model",name,{p:row["metrics"]["net_return_pct"] for p,row in periods.items()},flush=True)
        # Freeze choices before running evaluation-only baseline seeds.
        choices={str(year):model_selection(summary["annual"],year) for year in range(2023,2027)}
        save(ROOT/"annual-choice-frozen.json",{"rule":"previous3annual independent resets only; current/future year not inputs","choices":choices,"source_sha256":hashes})
        selected={str(year):{} for year in range(2023,2027)}
        for year,styles in choices.items():
            for style,item in styles.items():
                name=item["config"];spec=summary["experiments"][name];score=np.load(cache/f"{name}_scores.npy")
                selected[year][style]={"config":name,"choice_objective":item["objectives"][name],"evaluation":summary["annual"][name][year],"double_cost":run(name,score,spec["hold"],(int(year)*10000+101,min(int(year)*10000+1231,20260930)),cost=2,path=ROOT/"selected-ledgers"/f"{year}-{style}-double.json"),"baseline_seed_results":{}}
        # Each baseline keeps the exact model-positive pool; neutral scores only change rank.
        baseline_rows={name:{} for name in summary["experiments"]}
        for seed in SEEDS:
            ranks=neutral_rank(codes,dates,f["eligible"],seed)
            for name,spec in summary["experiments"].items():
                model_score=np.load(cache/f"{name}_scores.npy");pool=f["eligible"]&np.isfinite(model_score)&(model_score>0)
                neutral=np.where(pool,ranks,np.nan)
                result={p:run(name,neutral,spec["hold"],span,diagnostics=False,path=ROOT/"baseline-seed0-ledgers"/f"{name}-{p}.json" if seed==0 else None) for p,span in PERIODS.items()}
                result["test_double_cost"]=run(name,neutral,spec["hold"],PERIODS["test"],cost=2,diagnostics=False)
                baseline_rows[name][str(seed)]=result
                for year,styles in selected.items():
                    for style,item in styles.items():
                        if item["config"]!=name:continue
                        span=(int(year)*10000+101,min(int(year)*10000+1231,20260930))
                        item["baseline_seed_results"][str(seed)]={"base_cost":run(name,neutral,spec["hold"],span,diagnostics=False),"double_cost":run(name,neutral,spec["hold"],span,cost=2,diagnostics=False)}
            print("baseline seed",seed,"elapsed",round(time.time()-began,1),flush=True)
        def matched(rows,model):
            values=[row["metrics"]["net_return_pct"] for row in rows]
            d=distribution(values);v=model["metrics"]["net_return_pct"]
            d.update({"model_return":v,"model_minus_median":v-d["median"],"model_midrank_percentile":100*(sum(x<v for x in values)+.5*sum(x==v for x in values))/len(values)})
            return {"return_distribution":d,"drawdown_distribution":distribution([row["metrics"]["max_drawdown_pct"] for row in rows]),"seed_count":5,"warning":"matched positive pool; same market path; numerical midrank, not significance"}
        for name,rows in baseline_rows.items():
            item=summary["experiments"][name];item["matched_positive_baselines"]={p:matched([row[p] for row in rows.values()],item["periods"][p] if p in PERIODS else item["double_cost"]) for p in list(PERIODS)+["test_double_cost"]}
        for year,styles in selected.items():
            for style,item in styles.items():
                item["matched_positive_baselines"]={cost:matched([row[cost] for row in item["baseline_seed_results"].values()],item["evaluation"] if cost=="base_cost" else item["double_cost"]) for cost in ("base_cost","double_cost")}
        summary["annual_frozen_selection"]=selected
        save(ROOT/"baselines.json",{"seed_scores":"SHA256 fixedsignaldate/code/seed first48bits; no future prices","seeds":SEEDS,"all_config_periods":baseline_rows,"selected_annual":selected})
    finally:e.tradable_open=old_open
    after={name:sha(path) for name,path in sources.items()};assert hashes==after
    summary["input_sha256_end"]=after;summary["inputs_unchanged"]=True;summary["open_function_restored"]=e.tradable_open is old_open
    summary["minimum_cash_cny"]=min(x["minimum_cash_cny"] for x in all_accounts);summary["max_reconcile_cny"]=max(abs(x["reconcile_cny"]) for x in all_accounts);summary["portfolio_replays"]=len(all_accounts);summary["elapsed_seconds"]=round(time.time()-began,2)
    summary["failed_train_or_validation_configs"]=[name for name,item in summary["experiments"].items() if item["periods"]["train"]["metrics"]["net_return_pct"]<=0 or item["periods"]["validation"]["metrics"]["net_return_pct"]<=0]
    save(ROOT/"summary.json",summary);report(summary)
    save(ROOT/"artifact-check.json",{"source_sha256_start":hashes,"source_sha256_end":after,"inputs_unchanged":True,"files":{str(p.relative_to(ROOT)):{"sha256":sha(p),"bytes":p.stat().st_size} for p in ROOT.rglob("*") if p.is_file() and '.cache' not in p.parts and '__pycache__' not in p.parts and p.name!="artifact-check.json"}})
    print("study complete",len(summary["fits"]),"fits",len(all_accounts),"portfolio replays",round(time.time()-began,2),"seconds",flush=True)


if __name__=="__main__":
    parser=argparse.ArgumentParser();parser.add_argument("--self-check",action="store_true");parser.add_argument("--resume",action="store_true");args=parser.parse_args()
    self_check() if args.self_check else main(args.resume)
