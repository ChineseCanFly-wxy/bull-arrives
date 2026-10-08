"""Conservative timestamp financial ablation, not certified historical PIT."""
import argparse
import bisect
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
BASE=ROOT.parent/"ashare-models-2026-10-01"
sys.path.insert(0,str(BASE))
import model_study as r
np,m,s,e,rules=r.np,r.m,r.s,r.e,r.rules
PERIODS=r.PERIODS
HOLD=20
MAX_AGE=550
FIN_NAMES=["revenue_yoy","profit_yoy","roe","gross_margin","net_margin","cash_per_eps","deducted_per_eps","revenue_quarter_change","profit_quarter_change","income_report_age","debt_assets","cash_assets","working_assets","current_ratio","balance_report_age"]
CONFIGS={"frozen22_full":{"fit":False,"common":False},"frozen22_financial_pool":{"fit":False,"common":True},"refit22_financial_pool":{"fit":True,"common":True,"financial":False},"refit37_conservative_financial":{"fit":True,"common":True,"financial":True}}


def save(path,value):r.save(path,value)


def day(value):
    try:return int(str(value)[:10].replace("-","")) if value else None
    except ValueError:return None


def calendar(value):return dt.date(value//10000,value//100%100,value%100)


def number(value):
    try:return float(value) if value is not None and math.isfinite(float(value)) else np.nan
    except (ValueError,TypeError):return np.nan


def timestamp(row,kind):
    keys=("NOTICE_DATE","UPDATE_DATE","EITIME") if kind=="income" else ("NOTICE_DATE",)
    values=[day(row.get(key)) for key in keys]
    report=day(row.get("REPORTDATE" if kind=="income" else "REPORT_DATE"))
    if report is None or any(value is None or value<report or value>20261001 for value in values):return None
    return max(values)


def load_rows(kind,codes):
    folder=ROOT/("quarters" if kind=="income" else "balance-quarters")
    bystock=collections.defaultdict(list);audit=collections.Counter();duplicates=set()
    allowed=set(codes)
    for path in sorted(folder.glob("*.ndjson")):
        for text in path.read_text(encoding="utf-8").splitlines():
            row=json.loads(text);audit["raw_rows"]+=1;code=row.get("SECURITY_CODE")
            if code not in allowed or not s.STOCK.fullmatch(code):audit["outside_snapshot_or_non_AShare"]+=1;continue
            market="SH" if code.startswith(("60","68")) else "SZ"
            if row.get("SECUCODE")!=code+"."+market:audit["explicit_market_mismatch"]+=1;continue
            at=timestamp(row,kind)
            if at is None:audit["invalid_or_missing_dates"]+=1;continue
            key=(code,json.dumps(row,sort_keys=True,ensure_ascii=False))
            if key in duplicates:audit["identical_duplicates"]+=1;continue
            duplicates.add(key);row["available_calendar_day"]=at;row["report_day"]=day(row.get("REPORTDATE" if kind=="income" else "REPORT_DATE"));bystock[code].append(row)
            audit["retained_rows"]+=1
            if kind=="income" and at>day(row["NOTICE_DATE"]):audit["deferred_after_first_notice"]+=1
    for code,rows in bystock.items():
        grouped=collections.defaultdict(list)
        for row in rows:grouped[(row["report_day"],row["available_calendar_day"])].append(row)
        ambiguous={key for key,values in grouped.items() if len(values)>1}
        audit["ambiguous_same_report_availability_groups"]+=len(ambiguous)
        bystock[code]=sorted([row for row in rows if (row["report_day"],row["available_calendar_day"]) not in ambiguous],key=lambda row:(row["available_calendar_day"],row["report_day"]))
    return bystock,dict(audit)


def select_rows(rows,dates):
    """Only strictly later signal dates; late older revisions never replace newer reports."""
    selected=np.full(len(dates),-1,np.int32);at=0;best=-1
    for i,date in enumerate(dates):
        while at<len(rows) and rows[at]["available_calendar_day"]<int(date):
            if best<0 or rows[at]["report_day"]>=rows[best]["report_day"]:best=at
            at+=1
        if best>=0 and (calendar(int(date))-calendar(rows[best]["report_day"])).days<=MAX_AGE:selected[i]=best
    return selected


def ratio(a,b,positive=False):
    a,b=number(a),number(b)
    return a/b if np.isfinite(a) and np.isfinite(b) and (b>.001 if positive else abs(b)>.001) else np.nan


def clipped(value,lo,hi):return float(max(lo,min(value,hi))) if math.isfinite(value) else np.nan


def financial_values(income,balance,date):
    eps=income.get("BASIC_EPS") if income else None
    iv=income or {};bv=balance or {}
    out=[clipped(number(iv.get("YSTZ"))/100,-3,3),clipped(number(iv.get("SJLTZ"))/100,-3,3),clipped(number(iv.get("WEIGHTAVG_ROE"))/100,-1,1),clipped(number(iv.get("XSMLL"))/100,-1,1),
        clipped(ratio(iv.get("PARENT_NETPROFIT"),iv.get("TOTAL_OPERATE_INCOME"),True),-1,1),clipped(ratio(iv.get("MGJYXJJE"),eps,True),-5,5),clipped(ratio(iv.get("DEDUCT_BASIC_EPS"),eps),-5,5),
        clipped(number(iv.get("YSHZ"))/100,-5,5),clipped(number(iv.get("SJLHZ"))/100,-5,5),
        (calendar(date)-calendar(iv["report_day"])).days/550 if income else np.nan,
        clipped(number(bv.get("DEBT_ASSET_RATIO"))/100,0,2),clipped(ratio(bv.get("MONETARYFUNDS"),bv.get("TOTAL_ASSETS"),True),0,1),
        clipped(ratio(number(bv.get("ACCOUNTS_RECE"))+number(bv.get("INVENTORY")),bv.get("TOTAL_ASSETS"),True),0,1),clipped(number(bv.get("CURRENT_RATIO")),0,10),
        (calendar(date)-calendar(bv["report_day"])).days/550 if balance else np.nan]
    return out


def construct(codes,dates,eligible):
    income,ia=load_rows("income",codes);balance,ba=load_rows("balance",codes)
    features=np.full((*eligible.shape,len(FIN_NAMES)),np.nan,np.float32);cover=np.zeros(eligible.shape,bool);current={};coverage={}
    ordinals=np.array([calendar(int(date)).toordinal() for date in dates])
    for c,code in enumerate(codes):
        ir,br=income.get(code,[]),balance.get(code,[]);ii=select_rows(ir,dates);bi=select_rows(br,dates)
        for at in np.unique(ii[ii>=0]):
            mask=ii==at;iv=ir[at];features[mask,c,:10]=financial_values(iv,None,int(dates[np.flatnonzero(mask)[0]]))[:10]
            features[mask,c,9]=(ordinals[mask]-calendar(iv["report_day"]).toordinal())/550
        for at in np.unique(bi[bi>=0]):
            mask=bi==at;bv=br[at];features[mask,c,10:]=financial_values(None,bv,int(dates[np.flatnonzero(mask)[0]]))[10:]
            features[mask,c,14]=(ordinals[mask]-calendar(bv["report_day"]).toordinal())/550
        finite=np.isfinite(features[:,c])
        # A genuinely observed income report with >=5 of 9 financial measurements;
        # missing debt/gross margin/cash quality stays NaN, never benign zero.
        cover[:,c]=(ii>=0)&(finite[:,:9].sum(axis=1)>=5)
        if eligible[-1,c] and ii[-1]>=0:
            iv=ir[ii[-1]];bv=br[bi[-1]] if bi[-1]>=0 else None
            av=iv["available_calendar_day"];next_i=int(np.searchsorted(dates,av,side="right"))
            current[("sh" if code.startswith(("60","68")) else "sz")+code]=[iv["report_day"],day(iv["NOTICE_DATE"]),day(iv["UPDATE_DATE"]),day(iv["EITIME"]),int(dates[next_i]) if next_i<len(dates) else None,
                number(iv.get("YSTZ")),number(iv.get("SJLTZ")),number(iv.get("WEIGHTAVG_ROE")),number(iv.get("MGJYXJJE")),number(iv.get("BASIC_EPS")),number(iv.get("DEDUCT_BASIC_EPS")),number(iv.get("XSMLL")),
                bv["report_day"] if bv else None,day(bv.get("NOTICE_DATE")) if bv else None,number(bv.get("DEBT_ASSET_RATIO")) if bv else None,bool(cover[-1,c])]
    common=cover&eligible
    for year in range(2018,2027):
        mask=dates//10000==year;count=int(common[mask].sum());denom=int(eligible[mask].sum())
        coverage[str(year)]={"eligible_stock_days":denom,"financial_known_eligible_stock_days":count,"known_share_pct":round(100*count/denom,3) if denom else None,"distinct_known_stocks":int(np.any(common[mask],axis=0).sum()),"days_with_any":int(np.any(common[mask],axis=1).sum()),"market_days":int(mask.sum())}
    current={code:[float(value) if isinstance(value,np.generic) and not isinstance(value,np.bool_) else value if not isinstance(value,float) or math.isfinite(value) else None for value in row] for code,row in current.items()}
    evidence={"schema":"conservative-financial-stock-evidence-v1","as_of":"2026-09-30","acquired_on":"2026-10-01","source":"Eastmoney RPT_LICO_FN_CPD + RPT_DMSK_FN_BALANCE","history_version_certified":False,
        "date_rule":"signal date strictly after max(first notice,current revision date,provider EITIME); balance uses latest notice only and has no separate revision timestamp; signal enters following open",
        "columns":["report_date","first_notice_date","revision_date","provider_ingestion_date","available_signal_date","revenue_yoy_pct","profit_yoy_pct","roe_pct","cash_per_share_cny","eps_cny","deducted_eps_cny","gross_margin_pct","balance_report_date","balance_latest_notice_date","debt_assets_pct","research_coverage"],"stocks":current,"limitations":["source current-revision record may omit revisions and first-as-known history","balance notice only: unknown update lineage","cash/EPS ratio only meaningful for positive EPS; unknown remains null","all values cumulative reported periods, not certified quarterly-only accounting","2018 source ingestion gaps are unknown, not zero"]}
    text=json.dumps(evidence,ensure_ascii=False,separators=(",",":"),allow_nan=False)
    assert len(text.encode("utf-8"))<1024*1024
    (ROOT/"current-stock-evidence.json").write_text(text,encoding="utf-8")
    audit={"income":ia,"balance":ba,"coverage_by_year":coverage,"latest_financial_known_stocks":int(common[-1].sum()),"current_stock_evidence_count":len(current),"current_stock_evidence_bytes":len(text.encode("utf-8")),"missing_features_latest":{name:int((~np.isfinite(features[-1,:,j])&eligible[-1]).sum()) for j,name in enumerate(FIN_NAMES)}}
    save(ROOT/"coverage-audit.json",audit)
    print("COVERAGE",coverage,flush=True)
    return features,common,audit


def self_check():
    rows=[{"report_day":20201231,"available_calendar_day":20210331},{"report_day":20210331,"available_calendar_day":20210420},{"report_day":20191231,"available_calendar_day":20210501}]
    d=np.array([20210331,20210401,20210420,20210421,20210504,20230101]);assert select_rows(rows,d).tolist()==[-1,0,0,1,1,-1]
    row={"REPORTDATE":"2020-12-31","NOTICE_DATE":"2021-03-31","UPDATE_DATE":"2022-04-01","EITIME":"2021-03-30 19:00:00"}
    assert timestamp(row,"income")==20220401;assert timestamp({**row,"UPDATE_DATE":None},"income") is None
    values=financial_values({"report_day":20210331,"BASIC_EPS":-.2,"MGJYXJJE":1},None,20210501);assert math.isnan(values[5]) and math.isnan(values[10])
    assert all(math.isnan(v) for i,v in enumerate(financial_values(None,None,20210501)) if i not in ())
    print("self-check passed: late revision, strictly later signal, newest report wins, stale data unknown, missing/negative EPS not zero",flush=True)


def sources():
    paths={"runner":Path(__file__),"financial_manifest":ROOT/"fetch-manifest-final.json","original_financial_manifest":ROOT/"fetch-manifest.json","balance_manifest":ROOT/"balance-manifest.json","snapshot":r.DATA/"matrices.npz","metadata":r.DATA/"matrices-metadata.json","frozen22_scores":BASE/".cache/breadth22_h20_scores.npy","baseline_summary":BASE/"summary.json",
        "stock_features":Path(s.__file__),"learning_features":Path(m.__file__),"absolute_features":Path(r.a.__file__),"execution":Path(e.__file__),"rules":Path(rules.__file__),"action_overlay":r.OPEN/"online/action-overlay.json","benchmark":r.OPEN/"online/index-sh-000300.ndjson"}
    for folder in ("quarters","balance-quarters"):
        for path in (ROOT/folder).glob("*.ndjson"):paths[str(path.relative_to(ROOT))]=path
    return paths


def register():
    if (ROOT/"preregistered.json").exists():raise ValueError("no overwrite")
    paths=sources();hashes={k:r.sha(v) for k,v in paths.items()}
    save(ROOT/"preregistered.json",{"schema":"conservative-financial-ablation-v1","created_at_utc":dt.datetime.now(dt.timezone.utc).isoformat(),"configs":CONFIGS,"financial_feature_names":FIN_NAMES,"holding":HOLD,"max_report_age_calendar_days":MAX_AGE,"training":"annual previous3calendar years; same original96hash eligible/day then common financial coverage filter for both new models, strict20sessionlabelendpoint before predictionyear; HistGB unchanged80iters/leaves15/l2/seed","common":"income publication, revision, ingestion dates all known and signal strictly later max; latest known report≤550calendar days old; ≥5 of9 source financial measurements finite; unobserved financial fields NaN","basis":"both new models use exactly same stock/day/target; frozen full and covered22 separate universes; same-pool neutral seeds0,1,2","financial_history_version_certified":False,"balance_revision_chain_certified":False,"all_later_returns_already_seen":True,"production_admission":False,"input_sha256":hashes})


def main(coverage_only=False):
    start=time.time();self_check();paths=sources();hashes={k:r.sha(v) for k,v in paths.items()}
    if not coverage_only:
        if (ROOT/"summary.json").exists():raise ValueError("no overwrite")
        assert json.loads((ROOT/"preregistered.json").read_text(encoding="utf-8"))["input_sha256"]==hashes
    with np.load(paths["snapshot"],allow_pickle=False) as data:
        codes,dates=data["codes"].tolist(),data["dates"];raw={k.removeprefix("raw_"):data[k] for k in data.files if k.startswith("raw_")};valid,seen,factors=data["valid"],data["seen"],data["factors"]
    meta=json.loads(paths["metadata"].read_text(encoding="utf-8"));events={(row["i"],row["c"]):row["event"] for row in meta["events"]};details,action_coverage=rules.action_map(codes,dates,events)
    rules.SZ_EFFECTIVE=20260706;f=s.features(raw,valid,factors,seen);financial,common,audit=construct(codes,dates,f["eligible"])
    if coverage_only:return
    _,arrays,market=m.inputs(raw,valid,f);index,_=r.cli.index_inputs(dates,paths["benchmark"],np,s)
    original=m.sample_rows(codes,dates,f["eligible"]);samples=[cols[common[i,cols]] for i,cols in enumerate(original)]
    xs=[r.xday(arrays,market,index,i,cols,"breadth22") for i,cols in enumerate(samples)];ys=[r.target(i,cols,HOLD,raw,valid,factors,f["close"]) if len(cols) and i+HOLD<len(dates) else None for i,cols in enumerate(samples)]
    summary={"schema":"conservative-financial-ablation-results-v1","as_of":"2026-09-30","exploration":True,"production_admission":False,"input_sha256_start":hashes,"coverage":audit,"action_coverage":action_coverage,"fits":[],"experiments":{},"annual":{},"neutral_baselines":{},"failures":[],"limitations":["current provider financial revisions conservatively delayed but all historical as-known lineage not certified","balance data latest notice has no independent revision timestamp","550day stale ceiling may admit one-year-old revised reports; age feature and coverage disclosed","cumulative quarter accounting not converted using uncertified past revisions","all earlier execution corporate-action/ST/auction limitations retained","later test repeatedly seen and not fresh OOS"]}
    cache=ROOT/".cache";modeldir=cache/"models";modeldir.mkdir(parents=True,exist_ok=True)
    frozen=np.load(paths["frozen22_scores"],mmap_mode="r");scores={"frozen22_full":frozen,"frozen22_financial_pool":np.where(common,frozen,np.nan)}
    for name,spec in CONFIGS.items():
        if not spec["fit"]:continue
        score=np.full(valid.shape,np.nan,np.float32)
        for year in range(2020,2027):
            train,boundary=m.train_days(dates,year,HOLD);train=[int(i) for i in train if len(samples[i])]
            if not train:
                summary["failures"].append({"config":name,"year":year,"error":"no known financial training rows"});continue
            x=np.concatenate([np.column_stack((xs[i],financial[i,samples[i]])) if spec["financial"] else xs[i] for i in train]);y=np.concatenate([ys[i] for i in train])
            model=m.fit("hist_tree",x,y);path=modeldir/f"{name}-{year}.pkl"
            with path.open("wb") as stream:pickle.dump(model,stream)
            for i in np.flatnonzero(dates//10000==year):
                cols=np.flatnonzero(common[i]);xp=r.xday(arrays,market,index,int(i),cols,"breadth22")
                if len(cols):score[i,cols]=m.predict(model,np.column_stack((xp,financial[i,cols])) if spec["financial"] else xp)
            samples_sha=hashlib.sha256(b"".join(np.array([i],np.int32).tobytes()+samples[i].astype(np.int32).tobytes() for i in train)).hexdigest()
            summary["fits"].append({"config":name,"prediction_year":year,"rows":len(y),"features":x.shape[1],"first_training_signal":int(dates[train[0]]),"last_training_signal":int(dates[train[-1]]),"last_actual_label_endpoint":int(dates[train[-1]+HOLD]),"training_stock_day_sha256":samples_sha,"targets_sha256":hashlib.sha256(y.tobytes()).hexdigest(),"model_sha256":r.sha(path)})
            print("fit",name,year,len(y),round(time.time()-start,1),flush=True)
        np.save(cache/f"{name}-scores.npy",score,allow_pickle=False);scores[name]=score;save(ROOT/"fit-progress.json",summary["fits"])
    accounts=[];buys=0;ledger_count=0;old_open=e.tradable_open;e.tradable_open=rules.tradable_open
    def replay(name,score,span,cost=1,path=None):
        nonlocal buys,ledger_count
        f["score"]=score;f["signals"][name]=f["eligible"]&np.isfinite(score)&(score>0)
        ledger=e.backtest(name,*span,codes,dates,raw,valid,factors,events,f,multiplier=cost,full_entry=True,hold=HOLD,journal=True,exit_policy="time",max_positions=10,action_details=details)
        account=rules.accounting(ledger);assert account["minimum_cash_cny"]>=0 and abs(account["reconcile_cny"])<.1;accounts.append(account);rules.GATE_CHANGES.clear()
        for row in ledger["orders"]:
            assert s.STOCK.fullmatch(row["code"])
            if row["side"]=="buy":assert raw["is_st"][int(np.searchsorted(dates,row["date"])),codes.index(row["code"])]==0;buys+=1
        if path:save(path,ledger);ledger_count+=1
        return {"metrics":ledger["metrics"],"accounting":{key:value for key,value in account.items() if key!="unclosed"},"cycle_diagnostics":r.cycles_audit(ledger)}
    try:
        for name,spec in CONFIGS.items():
            score=scores[name];periods={p:replay(name,score,span,path=ROOT/"ledgers"/f"{name}-{p}.json") for p,span in PERIODS.items()};double=replay(name,score,PERIODS["test"],2,ROOT/"ledgers"/f"{name}-test-double.json")
            summary["experiments"][name]={"spec":spec,"periods":periods,"double_cost":double,"current_positive_count":int(np.sum(np.isfinite(score[-1])&(score[-1]>0))),"scores_sha256":hashes["frozen22_scores"] if not spec["fit"] and not spec["common"] else None}
            summary["annual"][name]={str(year):replay(name,score,(year*10000+101,min(year*10000+1231,20260930)),path=ROOT/"annual-ledgers"/f"{name}-{year}.json") for year in range(2020,2027)}
            if spec["fit"]:
                pool=f["eligible"]&np.isfinite(score)&(score>0);seeds=[]
                for seed in (0,1,2):
                    neutral=r.neutral_rank(codes,dates,pool,seed);rows={p:replay(f"neutral_{name}_{seed}",neutral,span,path=ROOT/"neutral-ledgers"/f"{name}-s{seed}-{p}.json") for p,span in PERIODS.items()};rows["test_double_cost"]=replay(f"neutral_{name}_{seed}",neutral,PERIODS["test"],2,ROOT/"neutral-ledgers"/f"{name}-s{seed}-test-double.json");seeds.append({"seed":seed,"periods":rows})
                summary["neutral_baselines"][name]={p:{"returns_pct":[row["periods"][p]["metrics"]["net_return_pct"] for row in seeds],"median_return_pct":float(np.median([row["periods"][p]["metrics"]["net_return_pct"] for row in seeds]))} for p in list(PERIODS)+["test_double_cost"]};save(ROOT/"neutral-results"/f"{name}.json",seeds)
            save(ROOT/"progress.json",summary);print("replay",name,{p:v["metrics"]["net_return_pct"] for p,v in periods.items()},flush=True)
    finally:e.tradable_open=old_open
    baseline=json.loads(paths["baseline_summary"].read_text(encoding="utf-8"))
    assert all(summary["experiments"]["frozen22_full"]["periods"][p]["metrics"]==baseline["experiments"]["breadth22_h20"]["periods"][p]["metrics"] for p in PERIODS)
    for year in range(2020,2027):
        fits=[row for row in summary["fits"] if row["prediction_year"]==year];assert len({row["training_stock_day_sha256"] for row in fits})==1 and len({row["targets_sha256"] for row in fits})==1
    after={key:r.sha(value) for key,value in paths.items()};assert after==hashes
    summary["checks"]={"new_fits":len(summary["fits"]),"portfolio_replays":len(accounts),"saved_ledgers":ledger_count,"buy_orders_checked":buys,"ST_BJ_buys":0,"minimum_cash_cny":min(row["minimum_cash_cny"] for row in accounts),"maximum_reconcile_cny":max(abs(row["reconcile_cny"]) for row in accounts),"same_stock_day_targets_two_new_models":True,"strict_training_endpoints":all(row["last_actual_label_endpoint"]<row["prediction_year"]*10000+101 for row in summary["fits"]),"frozen22_exact_reproduced":True,"input_sha_unchanged":True,"elapsed_seconds":round(time.time()-start,3)}
    summary["input_sha256_end"]=after;save(ROOT/"summary.json",summary)
    save(ROOT/"manifest.json",{"schema":"financial-research-artifacts-v1","inputs_unchanged":True,"files":{str(path.relative_to(ROOT)):{"sha256":r.sha(path),"bytes":path.stat().st_size} for path in ROOT.rglob("*") if path.is_file() and "__pycache__" not in path.parts and path.name!="manifest.json"}})
    print("DONE",summary["checks"],flush=True)


if __name__=="__main__":
    parser=argparse.ArgumentParser();parser.add_argument("--self-check",action="store_true");parser.add_argument("--register",action="store_true");parser.add_argument("--coverage-only",action="store_true");args=parser.parse_args()
    if args.self_check:self_check()
    elif args.register:register()
    else:main(args.coverage_only)
