"""A-share technical-only absolute net-return proxy models with real index inputs."""
import argparse
import hashlib
import json
import pickle
from pathlib import Path
import numpy as np
import rolling_models as m
from rolling_models import s

ROOT=Path(__file__).parent
OPEN=ROOT.parent


def index_features(dates):
    rows=[json.loads(line) for line in (OPEN/"online/index-sh-000300.ndjson").read_text(encoding="utf-8").splitlines() if line.strip()]
    byday={int(r["date"].replace("-","")):float(r["close"]) for r in rows}
    values=np.array([byday.get(int(day),np.nan) for day in dates],np.float32)
    r1=values/s.lag(values)-1
    volatility=s.roll(r1[:,None],20,"std")[:,0]
    data=np.column_stack((values/s.lag(values,5)-1,values/s.lag(values,20)-1,
        values/s.roll(values[:,None],60)[:,0]-1,volatility)).astype(np.float32)
    return np.nan_to_num(data),hashlib.sha256((OPEN/"online/index-sh-000300.ndjson").read_bytes()).hexdigest()


def target(i,cols,hold,raw,valid,factors,qclose):
    entry,end=i+1,i+hold
    op=raw["open"][entry,cols]*factors[entry,cols]
    exit_value=np.where(valid[end,cols],raw["open"][end,cols]*factors[end,cols],qclose[end,cols])
    available=valid[entry,cols]&(raw["is_st"][entry,cols]==0)&np.isfinite(op)&(op>0)
    out=np.divide(exit_value,op,out=np.ones(len(cols)),where=available)-1
    # Absolute direction and a proxy cost; relative rank positivity alone cannot justify entry.
    return np.clip(np.where(available,out-.005,0),-.4,.4).astype(np.float32)


def xday(arrays,market,index,i,cols):
    x=m.day_x(arrays,market,i,cols)
    # Drop two historical valuation proxies and their valuation interaction; technical inputs only.
    x=x[:,[k for k in range(x.shape[1]) if k not in (14,15,24)]]
    return np.column_stack((x,np.tile(index[i],(len(cols),1)))).astype(np.float32)


def main():
    parser=argparse.ArgumentParser(); parser.add_argument("--self-check",action="store_true")
    args=parser.parse_args()
    if args.self_check:
        raw={"open":np.ones((6,3))*10,"is_st":np.zeros((6,3))}; valid=np.ones((6,3),bool)
        raw["open"][3]=[11,9,10]
        y=target(0,np.arange(3),3,raw,valid,np.ones((6,3)),raw["open"])
        np.testing.assert_allclose(y,[.095,-.105,-.005],atol=1e-6)
        print("absolute target self-check passed"); return
    out=ROOT/"absolute-results"; out.mkdir(exist_ok=True)
    modeldir=ROOT/"absolute-models"; modeldir.mkdir(exist_ok=True)
    s.save(ROOT/"absolute-prereg.json",{"created":"2026-10-01","reason":"relative-score positive does not imply absolute profit; legacy early exits mismatch horizon",
        "holds":[5,10,20,30],"models":["state_ridge","hist_tree"],"thresholds_net_proxy":[0,.01],"configurations":16,
        "features":"technical-only; historical EP/BP and interaction removed; real sh.000300 past5/20/moving-average60 and20volatility added",
        "training":"previous3calendar-years each prediction year, label ends before prediction year; hash96/day",
        "label":"absolute adjusted next-open to hold-dayopen proxy minus0.5%, clip40%; invalid nextentryzero and missingexitlastmark",
        "execution":"new open-onlyproxy, 10names8%target full,timeexit or3ATRfailure; no same-day OHLC/volume gate",
        "status":"new retrospective exploratory hypothesis, all failures retained"})
    with np.load(m.OLD/"exports/matrices.npz",allow_pickle=False) as cache:
        codes,dates=cache["codes"].tolist(),cache["dates"]
        raw={k.removeprefix("raw_"):cache[k] for k in cache.files if k.startswith("raw_")}
        seen,valid,factors=cache["seen"],cache["valid"],cache["factors"]
    meta=json.loads((m.OLD/"exports/matrices-metadata.json").read_text(encoding="utf-8"))
    events={(int(r["i"]),int(r["c"])):r["event"] for r in meta["events"]}
    f=s.features(raw,valid,factors,seen)
    names,arrays,market=m.inputs(raw,valid,f); index,index_sha=index_features(dates)
    sampled=m.sample_rows(codes,dates,f["eligible"])
    xs=[xday(arrays,market,index,i,cols) for i,cols in enumerate(sampled)]
    import sys
    sys.path.insert(0,str(OPEN)); import execution as e
    from execution_study import action_map
    details,coverage=action_map(codes,dates,events)
    summary={"experiments":{},"fits":[],"online_index_sha256":index_sha,"action_coverage":coverage,"production_admission":False}
    for hold in (5,10,20,30):
        scores={kind:np.full(valid.shape,np.nan,np.float32) for kind in ("state_ridge","hist_tree")}
        for year in range(2020,2027):
            train,boundary=m.train_days(dates,year,hold)
            x=np.concatenate([xs[i] for i in train if len(sampled[i])])
            y=np.concatenate([target(int(i),sampled[i],hold,raw,valid,factors,f["close"]) for i in train if len(sampled[i])])
            predicted=np.flatnonzero((dates>=year*10000+101)&(dates<=year*10000+1231))
            for kind in scores:
                model=m.fit(kind,x,y)
                with (modeldir/f"{kind}_h{hold}_y{year}.pkl").open("wb") as handle:pickle.dump(model,handle)
                for i in predicted:
                    cols=np.flatnonzero(f["eligible"][i])
                    if len(cols):scores[kind][i,cols]=m.predict(model,xday(arrays,market,index,int(i),cols))
                summary["fits"].append({"model":kind,"hold":hold,"prediction_year":year,"training_rows":len(y),"last_training_label_date":int(dates[boundary-1])})
                print("absolute fitted",kind,hold,year,len(y),flush=True)
        for kind,score in scores.items():
            np.save(out/f"{kind}_h{hold}_scores.npy",score); f["score"]=score
            for threshold in (0,.01):
                name=f"{kind}_h{hold}_threshold{threshold}"
                f["signals"][name]=f["eligible"]&np.isfinite(score)&(score>threshold)
                kwargs={"max_positions":10,"full_entry":True,"exit_policy":"time","hold":hold,"action_details":details}
                rows={p:e.backtest(name,*span,codes,dates,raw,valid,factors,events,f,**kwargs)["metrics"] for p,span in s.PERIODS.items()}
                summary["experiments"][name]={"model":kind,"hold":hold,"threshold":threshold,"periods":rows}
                s.save(out/"progress.json",summary); print(name,{p:(r["net_return_pct"],r["max_drawdown_pct"]) for p,r in rows.items()},flush=True)
    objectives={n:m.objective(v["periods"]) for n,v in summary["experiments"].items()}
    summary["styles"]={"return":max(objectives,key=lambda n:objectives[n][1]),"floor":max(objectives,key=lambda n:objectives[n][0]),"drawdown":max(objectives,key=lambda n:objectives[n][2])}
    for style,name in summary["styles"].items():
        spec=summary["experiments"][name];hold=spec["hold"];score=np.load(out/f"{spec['model']}_h{hold}_scores.npy");f["score"]=score
        f["signals"][name]=f["eligible"]&np.isfinite(score)&(score>spec["threshold"])
        kwargs={"max_positions":10,"full_entry":True,"exit_policy":"time","hold":hold,"action_details":details}
        ledger=e.backtest(name,*s.PERIODS["test"],codes,dates,raw,valid,factors,events,f,journal=True,**kwargs)
        s.save(out/f"{style}-ledger.json",ledger)
        spec["double_cost"]=e.backtest(name,*s.PERIODS["test"],codes,dates,raw,valid,factors,events,f,multiplier=2,**kwargs)["metrics"]
        spec["annual_independent_reset"]={str(y):e.backtest(name,y*10000+101,y*10000+1231,codes,dates,raw,valid,factors,events,f,**kwargs)["metrics"] for y in range(2020,2027)}
    s.save(out/"summary.json",summary);print("absolute finished",summary["styles"],flush=True)


if __name__=="__main__":main()
