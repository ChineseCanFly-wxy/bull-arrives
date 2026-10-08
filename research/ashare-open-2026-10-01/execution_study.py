"""Separate execution/holding experiment on frozen walk-forward model scores."""
import hashlib
import json
from pathlib import Path
import numpy as np
import execution as e
from execution import s

ROOT=Path(__file__).parent
OLD=ROOT.parent/"ashare-2026-10-01"


def action_map(codes,dates,events=None):
    path=ROOT/"online/action-overlay.json"
    if not path.exists():return {},{"status":"not_ready"}
    source=json.loads(path.read_text(encoding="utf-8"))
    rows=source.get("events", source.get("actions", []))
    mapping={}
    for r in rows:
        if not r.get("source_verified_flag") or not r.get("units_match_stockdb"):
            continue
        pdf=ROOT/"online"/r["issuer_pdf"]
        if hashlib.sha256(pdf.read_bytes()).hexdigest()!=r["issuer_pdf_sha256"]:
            raise ValueError("Company-action source digest mismatch")
        code=str(r.get("code", "")).split(".")[-1]
        day=r.get("ex_date") or r.get("operate_date") or r.get("dividOperateDate")
        if code not in codes or not day:continue
        day=int(str(day).replace("-",""))
        i=int(np.searchsorted(dates,day)); c=codes.index(code)
        if i>=len(dates) or int(dates[i])!=day:continue
        if events is not None:
            original=events.get((i,c))
            declared=r.get("stockdb_event",{})
            if not original or any(abs(float(original.get(k,0))-float(declared.get(k,0)))>1e-8 for k in ("div","give","trans")):
                raise ValueError("Company-action units mismatch")
        detail={}
        for key,field in (("reference_div","virtual_exright_cash_per_share_cny"),("reference_trans","virtual_exright_reserve_shares_per_share")):
            if r.get(field) is not None:detail[key]=float(r[field])
        for field,possible in (("pay_i",("pay_date","dividPayDate")),("release_i",("stock_listing_date","release_date","dividStockMarketDate"))):
            value=next((r.get(k) for k in possible if r.get(k)),None)
            if value:
                at=int(np.searchsorted(dates,int(str(value).replace("-","")),side="right" if field=="pay_i" else "left"))
                if at<i:raise ValueError("Payment or listing before ex-date")
                detail[field]=at
        if detail:mapping[(i,c)]=detail
    return mapping,{"path":str(path),"sha256":hashlib.sha256(path.read_bytes()).hexdigest(),"mapped_events":len(mapping),
        "limitation":"issuer-PDF verified exact original-share units; cash becomes reusable next session after pay date; incomplete market-wide coverage"}


def objective(periods):
    a,b=[periods[p] for p in ("train","validation")]
    return np.array([min(a["net_return_pct"],b["net_return_pct"]),
        .5*(a["net_return_pct"]+b["net_return_pct"]),-max(a["max_drawdown_pct"],b["max_drawdown_pct"])])


def main():
    out=ROOT/"execution-results"; out.mkdir(exist_ok=True)
    variants={"basket10_time":dict(max_positions=10,full_entry=True,exit_policy="time"),
        "basket10_expansion":dict(max_positions=10,full_entry=True,exit_policy="time"),
        "layers4_model":dict(max_positions=4,full_entry=False,exit_policy="model")}
    s.save(ROOT/"execution-prereg.json",{"created":"2026-10-01","status":"after initial broad-model failure, exploratory follow-up",
        "candidate_configurations":30,"holds":[3,5,10,20,30],"models":["state_ridge","hist_tree"],"variants":variants,
        "signal":"frozen annual walk-forward scores positive; expansion uses known breadth>=.45 and nonnegative5day breadth change",
        "new_execution":"same-date rawopen/ST plus previous completed rawclose; no same-date completed OHLC/volume validity gate; real auction depth and ST preopen knowledge assumed/unverified",
        "time_policy":"three entry ATR failure or planned time; model_policy also exits score<=0; avoids old premature launch exit mismatch",
        "sizing":"4 slots20%target or10slots8%target; full or40/30/30%target",
        "selection":"multiple train/validation preferences; later history not used to choose",
        "code_sha256":hashlib.sha256(Path(e.__file__).read_bytes()).hexdigest()})
    with np.load(OLD/"exports/matrices.npz",allow_pickle=False) as cache:
        codes,dates=cache["codes"].tolist(),cache["dates"]
        raw={k.removeprefix("raw_"):cache[k] for k in cache.files if k.startswith("raw_")}
        seen,valid,factors=cache["seen"],cache["valid"],cache["factors"]
    meta=json.loads((OLD/"exports/matrices-metadata.json").read_text(encoding="utf-8"))
    events={(int(r["i"]),int(r["c"])):r["event"] for r in meta["events"]}
    details,coverage=action_map(codes,dates,events)
    f=s.features(raw,valid,factors,seen)
    summary={"experiments":{},"action_coverage":coverage,"production_admission":False,
        "limitation":"open price remains zero-latency research proxy; exact queue,ST knowledge,unknown actions and liquidation unverified"}
    for hold in (3,5,10,20,30):
        for model in ("state_ridge","hist_tree"):
            score=np.load(ROOT/f"learning/results/{model}_h{hold}_scores.npy")
            f["score"]=score
            for variant,kwargs in variants.items():
                name=f"{model}_h{hold}_{variant}"
                signal=f["eligible"]&np.isfinite(score)&(score>0)
                if variant=="basket10_expansion":signal&=(f["breadth"][:,None]>=.45)&((f["breadth"]-s.lag(f["breadth"],5))[:,None]>=0)
                f["signals"][name]=signal
                periods={p:e.backtest(name,*span,codes,dates,raw,valid,factors,events,f,hold=hold,action_details=details,**kwargs)["metrics"] for p,span in s.PERIODS.items()}
                summary["experiments"][name]={"hold":hold,"model":model,"variant":variant,"kwargs":kwargs,"periods":periods}
                s.save(out/"progress.json",summary)
                print(name,{p:(m["net_return_pct"],m["max_drawdown_pct"]) for p,m in periods.items()},flush=True)
    objectives={n:objective(v["periods"]) for n,v in summary["experiments"].items()}
    summary["frontier"]=[n for n,a in objectives.items() if not any(np.all(b>=a)&np.any(b>a) for k,b in objectives.items() if k!=n)]
    summary["styles"]={"return":max(objectives,key=lambda n:objectives[n][1]),"cross_period":max(objectives,key=lambda n:objectives[n][0]),"drawdown":max(objectives,key=lambda n:objectives[n][2])}
    for style,name in summary["styles"].items():
        spec=summary["experiments"][name]; hold=spec["hold"]; score=np.load(ROOT/f"learning/results/{spec['model']}_h{hold}_scores.npy")
        f["score"]=score
        signal=f["eligible"]&np.isfinite(score)&(score>0)
        if spec["variant"]=="basket10_expansion":signal&=(f["breadth"][:,None]>=.45)&((f["breadth"]-s.lag(f["breadth"],5))[:,None]>=0)
        f["signals"][name]=signal
        kwargs={**spec["kwargs"],"hold":hold,"action_details":details}
        ledger=e.backtest(name,*s.PERIODS["test"],codes,dates,raw,valid,factors,events,f,journal=True,**kwargs)
        s.save(out/f"{style}-ledger.json",ledger)
        spec["double_cost"]=e.backtest(name,*s.PERIODS["test"],codes,dates,raw,valid,factors,events,f,multiplier=2,**kwargs)["metrics"]
        spec["calendar_years_independent_reset"]={str(year):e.backtest(name,year*10000+101,year*10000+1231,codes,dates,raw,valid,factors,events,f,**kwargs)["metrics"] for year in range(2020,2027)}
    s.save(out/"summary.json",summary)
    print("execution study complete",summary["styles"],flush=True)


if __name__=="__main__":main()
