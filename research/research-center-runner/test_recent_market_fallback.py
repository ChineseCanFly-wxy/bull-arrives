"""Isolated provider records; no requests, StockDB writes or market claims."""
import datetime as dt
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

import numpy as np
import recent_market_fallback as r
import refresh_market as stock


class FallbackTests(unittest.TestCase):
    def original(self, count=3):
        codes=[f"{i:06d}" for i in range(1,count+1)]
        if count==3: codes=["000001","600000","300001"]
        data={"codes":np.array(codes),"dates":np.array([20260924],np.int32),"seen":np.ones((1,count),bool),"valid":np.ones((1,count),bool),"factors":np.ones((1,count),np.float32),"raw_damaged":np.zeros((1,count),bool)}
        for field in r.FIELDS:data["raw_"+field]=np.full((1,count),0 if field=="is_st" else 10,np.float32)
        return data,{"quality":{"last_date":20260924,"sessions":1},"events":[]}

    def quote(self,code="sz000001",provider="sina"):
        return r.record(code,"样例股票",dt.datetime(2026,9,28,15,10,tzinfo=r.CST),10,10.2,10.3,9.9,1000123,10000555,10,provider)

    def sina(self,code="sz000001",date="2026-09-28",time="15:10:00",name="样例股票"):
        fields=[name,"10","10","10.2","10.3","9.9","10.2","10.3","1000123","10000555"]+["0"]*20+[date,time,"00"]
        return 'var hq_str_'+code+'="'+','.join(fields)+'";'

    def tencent(self,code="sz000001"):
        fields=["0"]*39;values={1:"样例股票",2:code[2:],3:"10.2",4:"10",5:"10",6:"10000",30:"20260928151000",33:"10.3",34:"9.9",35:"10.2/10000/10000555",37:"1000.06"}
        for i,value in values.items():fields[i]=value
        return 'v_'+code+'="'+'~'.join(fields)+'";'

    def test_sina_keeps_shares_and_yuan_and_timestamp(self):
        quotes,errors=r.parse_quotes("sina",self.sina());self.assertEqual(errors,[])
        q=quotes["sz000001"];self.assertEqual(q["volume"],1000123);self.assertEqual(q["amount"],10000555)
        self.assertTrue(r.current_row(q,"2026-09-28",dt.datetime(2026,9,28,16,tzinfo=r.CST)))

    def test_tencent_normalizes_lots_and_uses_yuan_aggregate(self):
        quotes,errors=r.parse_quotes("tencent",self.tencent());self.assertEqual(errors,[])
        self.assertEqual(quotes["sz000001"]["volume"],1000000);self.assertEqual(quotes["sz000001"]["amount"],10000555)

    def test_rejects_duplicate_invalid_or_ambiguous_identity(self):
        for text in [self.sina()+self.sina(),self.tencent().replace('~000001~','~600000~')]:
            quotes,errors=r.parse_quotes("sina" if 'hq_str' in text else "tencent",text)
            self.assertEqual(quotes,{});self.assertTrue(errors)

    def test_old_intraday_and_future_timestamps_are_not_completed(self):
        now=dt.datetime(2026,9,28,16,tzinfo=r.CST)
        for date,time in [("2026-09-24","16:00:00"),("2026-09-28","14:59:59"),("2026-09-28","16:01:00")]:
            quotes,_=r.parse_quotes("sina",self.sina(date=date,time=time));self.assertFalse(r.current_row(quotes['sz000001'],"2026-09-28",now))

    def test_st_and_delisting_current_names_are_excluded(self):
        for name in ["ST样例","*ST样例","退样例"]:
            quotes,_=r.parse_quotes("sina",self.sina(name=name));self.assertTrue(quotes['sz000001']['is_st'])

    def test_stockdb_failure_uses_backup_and_records_both_failures(self):
        args=types.SimpleNamespace(as_of="2026-09-28")
        with patch.object(stock,'refresh_stockdb',side_effect=ValueError('local unavailable')),patch.object(r,'refresh',return_value={'state':'updated','as_of':args.as_of}) as fallback:
            self.assertEqual(stock.refresh(args)['state'],'updated');self.assertIn('local unavailable',fallback.call_args.args[1])
        with patch.object(stock,'refresh_stockdb',side_effect=ValueError('local unavailable')),patch.object(r,'refresh',side_effect=ValueError('provider unavailable')):
            with self.assertRaisesRegex(ValueError,'local unavailable.*provider unavailable'):stock.refresh(args)

    def test_existing_output_does_not_bypass_stockdb_overwrite_protection(self):
        import tempfile
        with tempfile.TemporaryDirectory(prefix='bull-no-overwrite-') as root:
            args=types.SimpleNamespace(as_of='2026-09-28',output=Path(root))
            with patch.object(stock,'refresh_stockdb') as local,patch.object(r,'refresh') as backup:
                with self.assertRaisesRegex(ValueError,'目录已存在'):stock.refresh(args)
                local.assert_not_called();backup.assert_not_called()

    def test_current_stockdb_never_requests_provider(self):
        args=types.SimpleNamespace(as_of="2026-09-28")
        with patch.object(stock,'refresh_stockdb',return_value={'state':'updated','as_of':args.as_of}),patch.object(r,'refresh') as fallback:
            self.assertEqual(stock.refresh(args)['mode'],'stockdb');fallback.assert_not_called()

    def test_provider_switch_only_refetches_missing_or_old_symbols(self):
        calls=[];now=dt.datetime(2026,9,28,16,tzinfo=r.CST)
        def requester(provider,codes,output,batch):
            calls.append((provider,codes));rows={}
            for code in codes:
                q=self.quote(code,provider)
                if provider=='sina' and code=='sh600000':q['date']='2026-09-24'
                rows[code]=q
            return rows,{'provider':provider,'symbols':len(codes)}
        rows,requests=r.collect(['sz000001','sh600000'],'2026-09-28',Path('.'),now,requester)
        self.assertEqual(calls,[('sina',['sz000001','sh600000']),('tencent',['sh600000'])]);self.assertEqual(rows['sh600000']['provider'],'tencent');self.assertEqual(len(requests),2)

    def test_new_session_preserves_every_frozen_prefix(self):
        data,meta=self.original();quotes={r.symbol(c):self.quote(r.symbol(c)) for c in data['codes']}
        updated,metadata=r.append_online(data,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=3)
        for key,value in data.items():np.testing.assert_array_equal(updated[key] if key=='codes' else updated[key][:-1],value)
        self.assertEqual(metadata['events'],[]);self.assertEqual(metadata['online_refresh']['valid_coverage'],3)
        self.assertTrue(np.isnan(updated['raw_pe_ttm'][-1]).all());self.assertEqual(metadata['quality']['last_date'],20260928)

    def test_ex_right_previous_close_is_unknown_not_made_up_factor(self):
        data,meta=self.original();quotes={r.symbol(c):self.quote(r.symbol(c)) for c in data['codes']};quotes['sz000001']['previous_close']=9
        with self.assertRaisesRegex(ValueError,'不足'):r.append_online(data,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=3)
        updated,metadata=r.append_online(data,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=2,minimum_ratio=.6)
        self.assertFalse(updated['valid'][-1,0]);self.assertTrue(updated['raw_damaged'][-1,0]);self.assertEqual(updated['factors'][-1,0],1)

    def test_one_cent_reference_change_is_not_assumed_to_be_float_rounding(self):
        data,meta=self.original();quotes={r.symbol(c):self.quote(r.symbol(c)) for c in data['codes']}
        quotes['sz000001']['previous_close']=9.99
        with self.assertRaisesRegex(ValueError,'不足'):r.append_online(data,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=3)
        quotes['sz000001']['previous_close']=10.00001
        _,metadata=r.append_online(data,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=3)
        self.assertEqual(metadata['online_refresh']['valid_coverage'],3)

    def test_multiday_gap_and_partial_coverage_cannot_be_called_current(self):
        data,meta=self.original();quotes={r.symbol(c):self.quote(r.symbol(c)) for c in data['codes']}
        with self.assertRaisesRegex(ValueError,'多个交易日'):r.append_online(data,meta,quotes,'2026-09-29','2026-09-28',minimum_coverage=3)
        quotes.pop('sz000001')
        with self.assertRaisesRegex(ValueError,'不足'):r.append_online(data,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=3)

    def test_stockdb_recovery_accepts_only_bounded_source_rounding_and_never_rewrites_prefix(self):
        original,meta=self.original();quotes={r.symbol(c):self.quote(r.symbol(c)) for c in original['codes']}
        saved,meta=r.append_online(original,meta,quotes,'2026-09-28','2026-09-24',minimum_coverage=3)
        boundary=[{'code':c,'date':20260928,**{field:float(saved['raw_'+field][-1,i]) for field in r.FIELDS}} for i,c in enumerate(saved['codes'])]
        for row in boundary:row['amount']+=100;row['volume']-=50;row['pe_ttm']=10;row['pb']=1
        daily=[{'code':c,'date':20260929,**{field:False if field=='is_st' else 10 for field in r.FIELDS}} for c in saved['codes']]
        updated,_,_=stock.append_matrices(saved,meta,daily,boundary,{},[20260929],[],minimum_coverage=3)
        for key,values in saved.items():np.testing.assert_array_equal(updated[key] if key=='codes' else updated[key][:-1],values)
        boundary[0]['close']+=.01
        with self.assertRaisesRegex(ValueError,'跨源不一致'):stock.append_matrices(saved,meta,daily,boundary,{},[20260929],[],minimum_coverage=3)

    def test_complete_fallback_exports_after_a_stale_stockdb_audit_without_touching_sources(self):
        import tempfile, gzip, json
        with tempfile.TemporaryDirectory(prefix='bull-provider-export-') as directory:
            root=Path(directory);source=root/'source';source.mkdir();output=root/'refresh'
            original,metadata=self.original()
            codes=np.asarray([f'001{i:03d}' for i in range(1000)]+[f'002{i:03d}' for i in range(1000)])
            original={key:codes if key=='codes' else values if key=='dates' else np.repeat(values[:,:1],len(codes),axis=1) for key,values in original.items()}
            np.savez(source/'matrices.npz',**original)
            stock.write_json(source/'matrices-metadata.json',metadata)
            with gzip.open(source/'invalid_records.ndjson.gz','wt',encoding='utf8'):pass
            index=root/'index.ndjson';index.write_text(json.dumps({'date':'2026-09-24','code':'sh.000300','open':'10','high':'10','low':'10','close':'10'})+'\n',encoding='utf8')
            paths=[source/'matrices.npz',source/'matrices-metadata.json',source/'invalid_records.ndjson.gz',index]
            before={str(path):stock.digest(path) for path in paths}
            args=types.SimpleNamespace(source=source,index=index,output=output,as_of='2026-09-28',previous_as_of='2026-09-24',endpoint='http://127.0.0.1:7899')
            class StaleClient:
                def __init__(self,endpoint,directory):self.requests=[]
                def query(self,*unused):return []
            calls=[]
            def requester(provider,requested,directory,batch):
                calls.append(provider)
                return {code:self.quote(code,provider) for code in requested},{'provider':provider,'symbols':len(requested),'fixture':True}
            real_collect=r.collect
            def collect_fixture(requested,expected,directory,now,unused=None):return real_collect(requested,expected,directory,now,requester)
            with patch.object(stock,'Client',StaleClient),patch.object(r,'collect',side_effect=collect_fixture):
                result=stock.refresh(args)
            self.assertEqual(result['mode'],'provider_fallback');self.assertEqual(result['as_of'],args.as_of)
            self.assertEqual(set(calls),{'sina'})
            self.assertTrue((output/'stockdb-refresh-audit.json').is_file())
            self.assertEqual(json.loads((output/'refresh-audit.json').read_text(encoding='utf8'))['mode'],'provider_fallback')
            self.assertEqual(before,{str(path):stock.digest(path) for path in paths})
            self.assertTrue(Path(result['index']).read_bytes().startswith(index.read_bytes()))
            with np.load(Path(result['snapshot'])/'matrices.npz',allow_pickle=False) as saved:
                for key,value in original.items():np.testing.assert_array_equal(saved[key] if key=='codes' else saved[key][:-1],value)
                self.assertEqual(saved['dates'].tolist(),[20260924,20260928])
                self.assertTrue(np.isnan(saved['raw_pe_ttm'][-1]).all())
            with self.assertRaisesRegex(ValueError,'不覆盖已有'):r.refresh(args,'local unavailable',stock.digest,stock.read_index,stock.write_json)

    def test_intraday_refresh_is_rejected_before_any_network(self):
        args=types.SimpleNamespace(as_of='2026-09-28')
        with self.assertRaisesRegex(ValueError,'16:00'):r.refresh(args,'stockdb unavailable',None,None,None,now=dt.datetime(2026,9,28,14,tzinfo=r.CST))


if __name__=='__main__':unittest.main(verbosity=2)
