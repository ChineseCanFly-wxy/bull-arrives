"""Finite, retrospective nested A-share research. No model admission or live orders.
Each outer year selects only on its previous year, with holding-label endpoint purges.
The candidate bank and three selection policies are fixed before accounts run.
"""
import argparse, datetime as dt, hashlib, json, os, sys, time
from pathlib import Path
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
os.environ.setdefault('OMP_NUM_THREADS','4')
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'research/research-center-runner'))
import model_runner as trusted
FAMILIES={'trend':[2,3,4,5,6,7,13], 'pullback':[0,1,2,5,7,9,12,13], 'price_volume':[0,2,3,6,7,8,9,10,11,12,13]}
POLICIES=('balanced','return','defensive')
YEARS=(2022,2023,2024,2025,2026)
SCHEMA='ashare-nested-technical-v1'

def digest(p):return trusted.digest(p)
def save(p,obj):
    p=Path(p);p.parent.mkdir(parents=True,exist_ok=True)
    if p.exists():raise ValueError('refuse to overwrite a research artifact')
    p.write_text(json.dumps(obj,ensure_ascii=False,allow_nan=False,indent=2),encoding='utf8')
def bank():
    return [{'id':f'{family}_s{state}_h{hold}','family':family,'state_interactions':bool(state),'hold':hold,'columns':cols,'ridge_alpha':.02}
            for family,cols in FAMILIES.items() for state in (0,1) for hold in (10,20)]
def policy_key(row,policy):
    m=row['metrics'];ret=m['net_return_pct'];dd=m['max_drawdown_pct']
    if policy=='return':return (ret,-dd,row['id'])
    if policy=='defensive':return (-dd,ret,row['id'])
    return (ret-.5*dd,ret,-dd,row['id'])
def select(rows,policy):
    # A strategy with negative validation net PnL cannot beat holding cash.
    feasible=[r for r in rows if r['metrics']['net_return_pct']>0 and r['metrics']['completed_holding_cycles']>=30]
    return max(feasible,key=lambda r:policy_key(r,policy)) if feasible else None

def design(np,r,arrays,market,index,i,cols,spec,states):
    base=r.a.xday(arrays,market,index,i,cols)[:,spec['columns']]
    # Factors generated deterministically from current/past bars: level x volume, trend x location.
    full=r.a.xday(arrays,market,index,i,cols)
    x=np.column_stack((base,full[:,3]*full[:,8],full[:,2]*full[:,9]))
    if spec['state_interactions']:
        one=np.eye(3,dtype=np.float32)[states[i]] if states[i]>=0 else np.zeros(3,np.float32)
        tiled=np.tile(one,(len(cols),1))
        x=np.column_stack((x,tiled,np.einsum('ij,k->ijk',x,one).reshape(len(cols),-1),np.tile(index[i],(len(cols),1))))
    return np.nan_to_num(x,nan=0,posinf=0,neginf=0).astype(np.float64)

def causal_states(np,f,index,warmup=59):
    distance=index[:,2];breadth=f['breadth']
    states=np.full(len(distance),2,dtype=np.int8)
    states[(distance>0)&(breadth>=.55)]=0
    states[(distance<0)&(breadth<.40)]=1
    states[~np.isfinite(distance)|~np.isfinite(breadth)]=-1
    states[:warmup]=-1
    return states

def split_days(np,dates,year,hold):
    ix=np.arange(len(dates));inner=int(np.searchsorted(dates,(year-1)*10000+101));outer=int(np.searchsorted(dates,year*10000+101))
    train=np.flatnonzero((dates>=max(2018,year-4)*10000+101)&(ix+hold<inner))
    validation=np.flatnonzero((ix>=inner)&(ix+hold<outer))
    refit=np.flatnonzero((dates>=max(2018,year-4)*10000+101)&(ix+hold<outer))
    evaluation=np.flatnonzero((ix>=outer)&(dates<=(year*10000+1231)))
    assert all(i+hold<inner for i in train) and all(i+hold<outer for i in validation) and all(i+hold<outer for i in refit)
    return train,validation,refit,evaluation

def fit(np,r,arrays,market,index,samples,raw,valid,factors,f,dates,spec,days,states):
    xs=[];ys=[]
    for i in days:
        cols=samples[int(i)]
        if len(cols):
            xs.append(design(np,r,arrays,market,index,int(i),cols,spec,states))
            ys.append(r.target(int(i),cols,spec['hold'],raw,valid,factors,f['close']))
    x=np.vstack(xs);y=np.concatenate(ys).astype(np.float64)
    # Train-only normalization and deterministic closed-form ridge, no future tuning.
    mu=x.mean(0);std=np.maximum(x.std(0),.01);z=np.column_stack((np.ones(len(x)),(x-mu)/std))
    reg=np.eye(z.shape[1])*spec['ridge_alpha'];reg[0,0]=0
    coef=np.linalg.solve(z.T@z/len(z)+reg,z.T@y/len(z))
    return {'mean':mu,'std':std,'coef':coef,'sample_count':len(x),'last_label_endpoint':int(dates[int(days[-1])+spec['hold']])}

def scores(np,r,arrays,market,index,f,states,spec,model,days):
    out=np.full(f['eligible'].shape,np.nan,np.float32)
    for i in days:
        cols=np.flatnonzero(f['eligible'][i])
        if len(cols):
            x=design(np,r,arrays,market,index,int(i),cols,spec,states)
            out[i,cols]=(np.column_stack((np.ones(len(x)),(x-model['mean'])/model['std']))@model['coef']).astype(np.float32)
    return out

def main(args):
    began=time.time();registry,original_hash=trusted.verify_sources()
    sys.path.insert(0,str(ROOT/'research/ashare-models-2026-10-01'));import model_study as r
    np,s,e,rules=r.np,r.s,r.e,r.rules
    paths={'runner':Path(__file__),'snapshot':args.snapshot/'matrices.npz','metadata':args.snapshot/'matrices-metadata.json','index':args.index}
    source={k:digest(v) for k,v in paths.items()}
    data=trusted.load_npz(np,paths['snapshot'])
    if args.snapshot.resolve()!=r.DATA.resolve():trusted.validate_extension(np,data,trusted.load_npz(np,r.DATA/'matrices.npz'))
    codes=data['codes'].tolist();dates=data['dates'];raw={k.removeprefix('raw_'):v for k,v in data.items() if k.startswith('raw_')};valid=data['valid'];factors=data['factors']
    if len(set(codes))!=len(codes) or any(not s.STOCK.fullmatch(c) for c in codes) or np.any(np.diff(dates)<=0):raise ValueError('ordinary SH/SZ stock axes required')
    now=dt.datetime.now(dt.timezone(dt.timedelta(hours=8)));last=int(dates[-1])
    if last>int(now.strftime('%Y%m%d')) or (last==int(now.strftime('%Y%m%d')) and now.hour<16):raise ValueError('incomplete market day')
    ir=[json.loads(l) for l in args.index.read_text(encoding='utf8').splitlines() if l.strip()]
    if [int(v['date'].replace('-','')) for v in ir]!=dates.tolist() or any(v.get('code')!='sh.000300' for v in ir):raise ValueError('CSI300 calendar mismatch')
    meta=json.loads(paths['metadata'].read_text(encoding='utf8'));events={(v['i'],v['c']):v['event'] for v in meta['events']}
    if len(events)!=len(meta['events']):raise ValueError('duplicate company actions')
    original_meta=json.loads((r.DATA/'matrices-metadata.json').read_text(encoding='utf8'))
    if [v for v in meta['events'] if v['i']<2123]!=original_meta['events']:raise ValueError('changed historical actions')
    f=s.features(raw,valid,factors,data['seen']);_,arrays,market=r.m.inputs(raw,valid,f);index,index_day=r.cli.index_inputs(dates,args.index,np,s);states=causal_states(np,f,index)
    # Fixed code hash sample, first 48 of the existing deterministic 96/day, including disappeared stocks.
    samples=[cols[:48] for cols in r.m.sample_rows(codes,dates,f['eligible'])]
    details,coverage=rules.action_map(codes,dates,events);rules.SZ_EFFECTIVE=20260706;e.tradable_open=rules.tradable_open
    outdir=args.output.parent;outdir.mkdir(parents=True,exist_ok=True)
    prereg={'schema':SCHEMA,'bank':bank(),'policies':list(POLICIES),'year':args.year,'holding_labels':[10,20],'training_sample_per_day':48,'selection':'previous-year net PnL > 0 and >=30 closed cycles; balanced=return-.5DD, return=max return, defensive=min DD; otherwise cash','gap':'train label endpoint < inner start; validation/refit label endpoint < outer start; stop new inner entries before purge','state_rule':'risk_on:CSI300 distanceMA60>0 and breadth>=.55; risk_off:distance<0 and breadth<.40; mixed otherwise; unknown warmup','later_history_already_seen':True,'production_admission':False,'input_sha256':source,'frozen_upstream_sha256':original_hash}
    save(outdir/(args.output.stem+'.preregistered.json'),prereg)
    accounts=[]
    def run(spec,score,days,cost,suffix):
        name=spec['id'];f['score']=score;f['signals'][name]=f['eligible']&np.isfinite(score)&(score>0)
        result=e.backtest(name,int(dates[int(days[0])]),int(dates[int(days[-1])]),codes,dates,raw,valid,factors,events,f,multiplier=cost,full_entry=True,hold=spec['hold'],journal=True,exit_policy='time',max_positions=10,action_details=details)
        checked=rules.accounting(result)
        if abs(checked['reconcile_cny'])>.02 or checked['minimum_cash_cny']<-.01:raise ValueError('cash reconciliation')
        day_axis={int(day):i for i,day in enumerate(dates)}
        code_axis={code:i for i,code in enumerate(codes)}
        for order in result['orders']:
            if order['side']=='buy':
                i=day_axis[order['date']];c=code_axis[order['code']]
                if raw['is_st'][i-1,c]!=0 or raw['is_st'][i,c]!=0 or not s.STOCK.fullmatch(order['code']):raise ValueError('ST/BJ buy')
        checked['ST_BJ_buys']=0
        ledger=outdir/(args.output.stem+'.'+suffix+'.ledger.json');save(ledger,result)
        accounts.append({'path':str(ledger),'sha256':digest(ledger),'accounting':checked})
        return result
    candidates=[];models={};specs=bank()
    for spec in specs:
        train,val,refit,test=split_days(np,dates,args.year,spec['hold'])
        if not(len(train) and len(val) and len(test)):raise ValueError('insufficient year folds')
        model=fit(np,r,arrays,market,index,samples,raw,valid,factors,f,dates,spec,train,states)
        # Signals stop at purged validation boundary; account is marked at last prior-year session.
        val_calendar=np.flatnonzero((dates>=(args.year-1)*10000+101)&(dates<args.year*10000+101))
        score=scores(np,r,arrays,market,index,f,states,spec,model,val)
        ledger=run(spec,score,val_calendar,1,'inner_'+spec['id'])
        candidates.append({'id':spec['id'],'metrics':ledger['metrics'],'training_samples':model['sample_count'],'last_training_label_endpoint':model['last_label_endpoint'],'last_selection_label_endpoint':int(dates[int(val[-1])+spec['hold']]),'state_interactions':spec['state_interactions'],'hold':spec['hold']})
    evaluations=[];reused={}
    for policy in POLICIES:
        chosen=select(candidates,policy)
        if chosen is None:
            evaluations.append({'policy':policy,'selected_id':None,'status':'cash','metrics':{'net_return_pct':0,'max_drawdown_pct':0,'completed_holding_cycles':0},'state_breakdown':[]});continue
        spec=next(v for v in specs if v['id']==chosen['id'])
        if spec['id'] not in reused:
            train,val,refit,test=split_days(np,dates,args.year,spec['hold']);model=fit(np,r,arrays,market,index,samples,raw,valid,factors,f,dates,spec,refit,states)
            score=scores(np,r,arrays,market,index,f,states,spec,model,test)
            base=run(spec,score,test,1,'outer_'+spec['id']);double=run(spec,score,test,2,'double_'+spec['id'])
            # Delay the same frozen signals by one session; no refit or reselection.
            delayed=np.full_like(score,np.nan);delayed[1:]=score[:-1];delay=run(spec,delayed,test,1,'delay_'+spec['id'])
            neutral=np.full_like(score,np.nan)
            for i in test:
                cols=np.flatnonzero(f['eligible'][i]&np.isfinite(score[i])&(score[i]>0))
                neutral[i,cols]=(1+(np.array([int(hashlib.sha256((str(int(dates[i]))+codes[c]).encode()).hexdigest()[:8],16) for c in cols],dtype=np.uint64)%1000000))/1000001
            control=run(spec,neutral,test,1,'neutral_'+spec['id'])
            state_rows=[];entry_regime={int(dates[i]):int(states[max(0,i-1)]) for i in test}
            for state,label in ((0,'risk_on'),(1,'risk_off'),(2,'mixed'),(-1,'unknown')):
                cycles=[v for v in base['cycles'] if entry_regime.get(v['entry_date'])==state]
                state_rows.append({'state':label,'completed_cycles':len(cycles),'mean_cycle_net_pct':round(float(np.mean([v['net_pct'] for v in cycles])),4) if cycles else None,'win_rate_pct':round(sum(v['net_pnl']>0 for v in cycles)/len(cycles)*100,2) if cycles else None,'signal_days':sum(int(states[i])==state for i in test)})
            # Causality audits: prefix inputs are unchanged when later data are unavailable.
            cut=int(test[0]);short_raw={k:v[:cut+1] for k,v in raw.items()};short=s.features(short_raw,valid[:cut+1],factors[:cut+1],data['seen'][:cut+1]);_,short_arrays,short_market=r.m.inputs(short_raw,valid[:cut+1],short)
            short_index,_=r.cli.index_inputs(dates[:cut+1],args.index,np,s);short_states=causal_states(np,short,short_index)
            cols=np.flatnonzero(f['eligible'][cut])[:64]
            np.testing.assert_allclose(design(np,r,arrays,market,index,cut,cols,spec,states),design(np,r,short_arrays,short_market,short_index,cut,cols,spec,short_states),rtol=1e-6,atol=1e-6)
            np.testing.assert_array_equal(states[:cut+1],short_states)
            fit_path=outdir/(args.output.stem+'.outer_'+spec['id']+'.model.json');save(fit_path,{'spec':spec,'mean':model['mean'].tolist(),'std':model['std'].tolist(),'coef':model['coef'].tolist(),'training_samples':model['sample_count'],'last_label_endpoint':model['last_label_endpoint'],'production_admission':False})
            reused[spec['id']]={'metrics':base['metrics'],'double_cost':double['metrics'],'delayed_entry':delay['metrics'],'matched_pool_neutral':control['metrics'],'state_breakdown':state_rows,'refit_last_label_endpoint':model['last_label_endpoint'],'model_path':str(fit_path),'model_sha256':digest(fit_path),'prefix_causality_checked_stocks':len(cols)}
        evaluations.append({'policy':policy,'selected_id':spec['id'],'status':'evaluated','selected_on':f'{args.year-1}-12-31','selection_metrics':chosen['metrics'],**reused[spec['id']]})
    if any(digest(p)!=source[k] for k,p in paths.items()):raise ValueError('inputs changed during computation')
    _,end_hash=trusted.verify_sources()
    if end_hash!=original_hash:raise ValueError('upstream sources changed')
    report={'schema':SCHEMA,'year':args.year,'as_of':dt.datetime.strptime(str(int(dates[-1])),'%Y%m%d').date().isoformat(),'bank_size':len(specs),'candidate_identity':hashlib.sha256(json.dumps(bank(),sort_keys=True).encode()).hexdigest(),'policies':list(POLICIES),'selection_candidates':candidates,'evaluations':evaluations,'accounts':accounts,'input_sha256':source,'stocks':len(codes),'sessions':len(dates),'data_rows':int(data['seen'].sum()),'elapsed_seconds':round(time.time()-began,2),'production_admission':False,'new_admitted_models':0,'later_history_already_seen':True,'checks':{'holding_label_endpoints_purged':True,'selection_did_not_read_outer_metrics':True,'prefix_causality':True,'ST_BJ_buys':0,'whole_ledger_reconciled':True},'limitations':['All 2022-2026 history has already been viewed; nested evaluation is retrospective, not a pristine holdout.','Annual accounts reset to 100000; their compounded return is descriptive, not a continuous cash ledger.','Label uses adjusted open proxy minus fixed0.5%; actual cash engine rechecks next-open limits, lots, costs and T+1.','No point-in-time fundamental or industry input; arbitrary AI factors are not executed.','Unknown corporate-action dates retain receivables/locked shares; last quotes may overvalue suspended or delisted positions.','Candidate bank is finite; no strategy is automatically registered or admitted.']}
    save(args.output,report)
    print(json.dumps({'year':args.year,'accounts':len(accounts),'elapsed_seconds':report['elapsed_seconds']},ensure_ascii=False),flush=True)

def self_check():
    import numpy as np
    dates=np.arange(0,200,dtype=np.int64)+20200000
    # Explicit synthetic date axes spanning three years test strict endpoint inequalities.
    dates=np.array(list(range(20200101,20200161))+list(range(20210101,20210161))+list(range(20220101,20220161)))
    tr,va,re,ev=split_days(np,dates,2022,20)
    assert max(tr)+20<60 and max(va)+20<120 and max(re)+20<120 and min(ev)==120
    rows=[{'id':'a','metrics':{'net_return_pct':-1,'max_drawdown_pct':1,'completed_holding_cycles':50}}]
    assert select(rows,'return') is None
    rows+=[{'id':'b','metrics':{'net_return_pct':5,'max_drawdown_pct':1,'completed_holding_cycles':40}},{'id':'c','metrics':{'net_return_pct':10,'max_drawdown_pct':20,'completed_holding_cycles':40}}]
    assert select(rows,'return')['id']=='c' and select(rows,'balanced')['id']=='b' and select(rows,'defensive')['id']=='b'
    assert len(bank())==12 and len(set(v['id'] for v in bank()))==12
    index=np.array([[0,0,.01,0],[0,0,-.01,0],[0,0,.01,0],[0,0,np.nan,0]])
    np.testing.assert_array_equal(causal_states(np,{'breadth':np.array([.6,.3,.5,.6])},index,warmup=0),[0,1,2,-1])
    print('purge, policies, candidate identity and causal state tests passed')

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--self-check',action='store_true');p.add_argument('--year',type=int,choices=YEARS);p.add_argument('--snapshot',type=Path);p.add_argument('--index',type=Path);p.add_argument('--output',type=Path);args=p.parse_args()
    if args.self_check:self_check()
    elif not all((args.year,args.snapshot,args.index,args.output)):p.error('year, snapshot, index and output required')
    else:main(args)
