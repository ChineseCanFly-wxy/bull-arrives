"""Annual walk-forward A-share models; all later history is exploratory."""
import argparse
import hashlib
import json
import os
import pickle
import sys
import warnings
from pathlib import Path

os.environ.setdefault("OMP_NUM_THREADS", "4")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "4")
ROOT = Path(__file__).parent
OLD = ROOT.parent.parent / "ashare-2026-10-01"
DEPS = Path.home() / ".cache/ashare-research/ml"
sys.path.insert(0, str(DEPS))
sys.path.insert(0, str(OLD))
import numpy as np
import sklearn
from sklearn.ensemble import HistGradientBoostingRegressor
from threadpoolctl import threadpool_limits
import study as s

HOLDS = (3, 5, 10, 20, 30)
MODES = ("layered_any", "layered_expansion", "full_any")


def scale(a, eligible):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", RuntimeWarning)
        keep = np.where(eligible & np.isfinite(a), a, np.nan)
        mean = np.nanmean(keep, axis=1, keepdims=True)
        std = np.nanstd(keep, axis=1, keepdims=True)
    return np.nan_to_num(np.clip((a-mean)/np.maximum(std, .0001), -3, 3)).astype(np.float32)


def inputs(raw, valid, f):
    c, op, h, l, atr = [f[k] for k in ("close", "open", "high", "low", "atr")]
    safe_atr = np.maximum(atr, .001)
    volume = np.where(valid, raw["volume"], 0)
    values = {
        "r1": c / s.lag(c) - 1,
        "previous5": s.lag(c) / s.lag(c, 6) - 1,
        "r5": c / s.lag(c, 5) - 1, "r20": f["r20"],
        "r60": c / s.lag(c, 60) - 1, "distance_ma20": c / f["ma20"] - 1,
        "distance_prior60high": c / s.lag(s.roll(h, 60, "max")) - 1,
        "atr_pct": atr / c,
        "volume_ratio": volume / np.maximum(s.lag(s.roll(volume, 20)), 1),
        "close_location": f["location"], "body_atr": (c-op)/safe_atr,
        "upper_wick_atr": (h-np.maximum(c, op))/safe_atr,
        "lower_wick_atr": (np.minimum(c, op)-l)/safe_atr,
        "amount20_log": np.log1p(s.roll(np.where(valid, raw["amount"], 0), 20)),
        "earnings_yield_proxy": np.divide(1., raw["pe_ttm"], out=np.zeros_like(c), where=raw["pe_ttm"]>0),
        "book_yield_proxy": np.divide(1., raw["pb"], out=np.zeros_like(c), where=raw["pb"]>0)}
    names = list(values)
    arrays = [scale(v, f["eligible"]) for v in values.values()]
    breadth = f["breadth"].astype(np.float32)
    markets = np.nan_to_num(np.column_stack((2*breadth-1, breadth-s.lag(breadth,5),
        s.roll(breadth[:,None], 20)[:,0]-s.lag(s.roll(breadth[:,None],20)[:,0],20)))).astype(np.float32)
    return names, arrays, markets


def day_x(arrays, markets, i, cols):
    base = np.column_stack([a[i, cols] for a in arrays])
    market = np.tile(markets[i], (len(cols), 1))
    # State interactions let the same price/volume feature have a different effect by breadth.
    return np.column_stack((base, market, base[:,[0,1,4,7,8,14]]*market[:,0:1])).astype(np.float32)


def sample_rows(codes, dates, eligible):
    code_hash = np.array([int.from_bytes(hashlib.sha256(c.encode()).digest()[:4], "little") for c in codes], dtype=np.uint64)
    rows = []
    for i, date in enumerate(dates):
        cols = np.flatnonzero(eligible[i])
        mix = (code_hash[cols] ^ np.uint64(int(date)*2654435761)) & np.uint64(0xFFFFFFFF)
        rows.append(cols[np.argsort(mix, kind="stable")[:96]])
    return rows


def labels(i, cols, hold, raw, valid, factors, qclose):
    entry, end = i+1, i+hold
    op = raw["open"][entry, cols] * factors[entry, cols]
    exit_value = np.where(valid[end, cols], raw["open"][end, cols]*factors[end,cols], qclose[end,cols])
    available = valid[entry, cols] & (raw["is_st"][entry, cols]==0) & np.isfinite(op) & (op>0)
    proxy = np.divide(exit_value, op, out=np.ones(len(cols)), where=available)-1
    # Keep sampled missing endpoints using a disclosed last mark, rather than select future survivors.
    proxy = np.where(available, proxy-.005, 0.)
    target = np.clip(proxy-np.median(proxy), -.4, .4)
    return target.astype(np.float32), int((~valid[end,cols]).sum()), int((~available).sum())


def train_days(dates, year, hold):
    boundary = int(np.searchsorted(dates, year*10000+101))
    rows = np.flatnonzero((dates >= max(2018,year-3)*10000+101) & (np.arange(len(dates))+hold < boundary))
    assert all(i+hold < boundary for i in rows)
    return rows, boundary


def fit(kind, x, y):
    if kind == "state_ridge":
        xx = np.column_stack((np.ones(len(x)), x)).astype(np.float64)
        reg = np.eye(xx.shape[1])*.01
        reg[0,0] = 0
        return np.linalg.solve(xx.T@xx/len(xx)+reg, xx.T@y/len(xx))
    model = HistGradientBoostingRegressor(max_iter=80, learning_rate=.07, max_leaf_nodes=15,
        min_samples_leaf=80, l2_regularization=1., early_stopping=False, random_state=20261001)
    with threadpool_limits(limits=4):
        return model.fit(x,y)


def predict(model, x):
    if isinstance(model, np.ndarray):
        return model[0]+x@model[1:]
    with threadpool_limits(limits=4):
        return model.predict(x)


def objective(periods):
    a,b = periods["train"],periods["validation"]
    return np.array([min(a["net_return_pct"],b["net_return_pct"]),
        (a["net_return_pct"]+b["net_return_pct"])/2,
        -max(a["max_drawdown_pct"],b["max_drawdown_pct"]),
        min(a["mean_cycle_net_pct"] or 0,b["mean_cycle_net_pct"] or 0)],float)


def self_check():
    a = np.arange(18,dtype=float).reshape(6,3)
    b=a.copy(); b[-1]*=100
    ok=np.ones_like(a,dtype=bool)
    np.testing.assert_allclose(scale(a,ok)[:-1],scale(b,ok)[:-1])
    dates=np.array([20191230,20191231,20200102,20200103,20200106])
    rows,boundary=train_days(dates,2020,1)
    assert rows.tolist()==[0] and boundary==2
    raw={"open":np.ones((6,3))*10,"is_st":np.zeros((6,3))}
    valid=np.ones((6,3),bool); valid[3,1]=False
    target, missing, rejected=labels(0,np.array([0,1,2]),3,raw,valid,np.ones((6,3)),np.ones((6,3))*9)
    assert missing==1 and rejected==0 and len(target)==3 and target[1]<target[0]
    rng=np.random.default_rng(7); x=rng.normal(size=(800,4)); y=np.where(x[:,0]>.2, .1,-.1)
    for kind in ("state_ridge","hist_tree"):
        model=fit(kind,x,y)
        assert np.mean((predict(model,x)-y)**2)<.005
    print("rolling self-check passed: causal scales, purged train endpoints, missing endpoint retention, actual model branches",flush=True)


def main():
    parser=argparse.ArgumentParser(); parser.add_argument("--self-check",action="store_true")
    args=parser.parse_args(); self_check()
    if args.self_check: return
    ROOT.mkdir(parents=True,exist_ok=True)
    results=ROOT/"results"; results.mkdir(exist_ok=True)
    models_dir=ROOT/"models"; models_dir.mkdir(exist_ok=True)
    s.save(ROOT/"prereg.json",{"created":"2026-10-01", "status":"open exploratory expansion; later history already observed",
        "holds":HOLDS,"models":["state_ridge","hist_tree"],"modes":MODES,"configurations":30,
        "training":"annually rolling previous 3 calendar years, label endpoint strictly before prediction year",
        "sampling":"96 stocks per session, fixed code/date hash before outcome inspection",
        "label":"relative clipped next-open to hold-day open adjusted-return proxy minus 0.5%; missing exit last mark, rejected entry zero; not actual executable PnL",
        "missing":"missing endpoints retained; last mark may overvalue delisted stock; unknown company-action value not certified",
        "tree":{"iterations":80,"leaves":15,"min_leaf":80,"l2":1,"early_stopping":False},
        "ridge":"mean squared error plus .01 coefficient squared, intercept unpenalized",
        "selection":"Pareto and multiple styles using training/validation only; no 15% drawdown admission ceiling",
        "execution":"shared staged/full launch state, maximum 4 positions, next-open proxy with T+1 and fees",
        "versions":{"numpy":np.__version__,"sklearn":sklearn.__version__},"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "shared_study_sha256":hashlib.sha256(Path(s.__file__).read_bytes()).hexdigest()})
    with np.load(OLD/"exports/matrices.npz",allow_pickle=False) as cache:
        codes,dates=cache["codes"].tolist(),cache["dates"]
        raw={k.removeprefix("raw_"):cache[k] for k in cache.files if k.startswith("raw_")}
        seen,valid,factors=cache["seen"],cache["valid"],cache["factors"]
    meta=json.loads((OLD/"exports/matrices-metadata.json").read_text(encoding="utf-8"))
    events={(int(r["i"]),int(r["c"])):r["event"] for r in meta["events"]}
    f=s.features(raw,valid,factors,seen)
    names,arrays,markets=inputs(raw,valid,f)
    sampled=sample_rows(codes,dates,f["eligible"])
    xs=[day_x(arrays,markets,i,cols) for i,cols in enumerate(sampled)]
    summary={"status":"annual walk-forward scores; all later evaluation retrospective", "experiments":{},"fits":[],"feature_names":names,
        "quality":meta["quality"],"selection_uses_later_results":False,"production_admission":False}
    for hold in HOLDS:
        scores={kind:np.full(valid.shape,np.nan,dtype=np.float32) for kind in ("state_ridge","hist_tree")}
        for year in range(2020,2027):
            train,boundary=train_days(dates,year,hold)
            xx,yy,missing,rejected=[],[],0,0
            for i in train:
                cols=sampled[i]
                if not len(cols):continue
                y,bad,rej=labels(int(i),cols,hold,raw,valid,factors,f["close"])
                xx.append(xs[i]); yy.append(y); missing+=bad; rejected+=rej
            x,y=np.concatenate(xx),np.concatenate(yy)
            test=np.flatnonzero((dates>=year*10000+101)&(dates<=(year*10000+1231)))
            for kind in scores:
                model=fit(kind,x,y)
                with (models_dir/f"{kind}_h{hold}_y{year}.pkl").open("wb") as handle:pickle.dump(model,handle)
                for i in test:
                    cols=np.flatnonzero(f["eligible"][i])
                    if len(cols):scores[kind][i,cols]=predict(model,day_x(arrays,markets,int(i),cols))
                summary["fits"].append({"model":kind,"hold":hold,"prediction_year":year,"training_rows":len(y),
                    "last_possible_label_date":int(dates[boundary-1]),"first_prediction_date":int(dates[test[0]]),
                    "sampled_missing_exit_rows":missing,"sampled_rejected_entry_rows":rejected})
                print("fitted",kind,hold,year,len(y),"missing",missing,flush=True)
        for kind,score in scores.items():
            # ponytail: numeric scores are cache, not portable/trusted executable model artifacts.
            np.save(results/f"{kind}_h{hold}_scores.npy",score)
            f["score"]=score
            for mode in MODES:
                name=f"{kind}_h{hold}_{mode}"
                signal=f["eligible"]&np.isfinite(score)&(score>0)
                if mode=="layered_expansion":signal&=(f["breadth"][:,None]>=.45)&((f["breadth"]-s.lag(f["breadth"],5))[:,None]>=0)
                f["signals"][name]=signal
                periods={p:s.backtest(name,*span,codes,dates,raw,valid,factors,events,f,hold=hold,
                    exit_policy="launch",full_entry=(mode=="full_any"))["metrics"] for p,span in s.PERIODS.items()}
                summary["experiments"][name]={"model":kind,"hold":hold,"mode":mode,"periods":periods}
                s.save(results/"progress.json",summary)
                print(name,{p:(m["net_return_pct"],m["max_drawdown_pct"]) for p,m in periods.items()},flush=True)
    objectives={n:objective(v["periods"]) for n,v in summary["experiments"].items()}
    summary["pareto_train_validation"]=[n for n,a in objectives.items() if not any(np.all(b>=a)&np.any(b>a) for k,b in objectives.items() if k!=n)]
    selection={"return_focus":max(objectives,key=lambda n:objectives[n][1]),
        "cross_period_floor":max(objectives,key=lambda n:objectives[n][0]),
        "drawdown_focus":max(objectives,key=lambda n:objectives[n][2])}
    summary["style_comparators"]=selection
    for style,name in selection.items():
        spec=summary["experiments"][name]; hold,mode=spec["hold"],spec["mode"]
        f["score"]=np.load(results/f"{spec['model']}_h{hold}_scores.npy")
        f["signals"][name]=f["eligible"]&np.isfinite(f["score"])&(f["score"]>0)
        if mode=="layered_expansion":f["signals"][name]&=(f["breadth"][:,None]>=.45)&((f["breadth"]-s.lag(f["breadth"],5))[:,None]>=0)
        kwargs={"hold":hold,"exit_policy":"launch","full_entry":mode=="full_any"}
        ledger=s.backtest(name,*s.PERIODS["test"],codes,dates,raw,valid,factors,events,f,journal=True,**kwargs)
        s.save(results/f"{style}-ledger.json",ledger)
        spec.setdefault("diagnostics",{})[style]={"double_cost":s.backtest(name,*s.PERIODS["test"],codes,dates,raw,valid,factors,events,f,multiplier=2,**kwargs)["metrics"],
            "calendar_years_independent_reset":{str(year):s.backtest(name,year*10000+101,year*10000+1231,codes,dates,raw,valid,factors,events,f,**kwargs)["metrics"] for year in range(2020,2027)}}
    summary["shared_source_unchanged"]=hashlib.sha256(Path(s.__file__).read_bytes()).hexdigest()==json.loads((ROOT/"prereg.json").read_text(encoding="utf-8"))["shared_study_sha256"]
    s.save(results/"summary.json",summary)
    print("rolling complete",len(summary["experiments"]),selection,flush=True)


if __name__=="__main__":main()
