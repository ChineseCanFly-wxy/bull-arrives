"""Post-close providers fill a recent missing session, never intraday/model refitting.

Raw shares/yuan and explicit quote timestamps; a frozen prefix is never rewritten.
A missing multi-session history or unverified ex-right reference is not fabricated.
"""
import concurrent.futures
import datetime as dt
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import urllib.request

import numpy as np

CST = dt.timezone(dt.timedelta(hours=8))
FIELDS = ("open", "high", "low", "close", "volume", "amount", "is_st", "pe_ttm", "pb", "pct_chg")
PRICE_TOLERANCE = .001
INDEX_PRICE_TOLERANCE = .011
BATCH_SIZE = 100
NETWORK_TIMEOUT = 10


def finite(value):
    try:
        value = float(value)
        return value if math.isfinite(value) else math.nan
    except (TypeError, ValueError):
        return math.nan


def symbol(code):
    if not re.fullmatch(r"(?:000|001|002|003|300|301|600|601|603|605|688|689)\d{3}", code):
        raise ValueError("代码不在冻结沪深股票轴：" + code)
    return ("sh" if code.startswith("6") else "sz") + code


def stamp(value, pattern):
    return dt.datetime.strptime(value, pattern).replace(tzinfo=CST)


def record(code, name, at, op, close, high, low, volume, amount, previous, provider):
    prices = [finite(v) for v in (op, close, high, low)]
    vol, cash, previous = map(finite, (volume, amount, previous))
    if not name or "\ufffd" in name or not all(math.isfinite(v) and v > 0 for v in prices) or not math.isfinite(previous) or previous <= 0:
        raise ValueError("行情名称/价格/昨收无效：" + code)
    op, close, high, low = prices
    if high + PRICE_TOLERANCE < max(op, close) or low - PRICE_TOLERANCE > min(op, close) or low > high or not math.isfinite(vol) or not math.isfinite(cash) or vol <= 0 or cash <= 0:
        raise ValueError("行情OHLC或量额无效：" + code)
    return {"code": code, "date": at.date().isoformat(), "timestamp": at.isoformat(), "name": name,
            "open": op, "close": close, "high": high, "low": low, "volume": vol, "amount": cash,
            "is_st": "ST" in name.upper() or "退" in name, "pe_ttm": math.nan, "pb": math.nan,
            "pct_chg": (close / previous - 1) * 100, "previous_close": previous, "provider": provider}


def parse_quotes(provider, text):
    result, errors = {}, []
    pattern = r'(?:var hq_str_|v_)((?:sh|sz)\d{6})="([^"\n]*)";?'
    for match in re.finditer(pattern, text):
        code, content = match.groups()
        try:
            if provider == "sina":
                f = content.split(",")
                if len(f) < 32:
                    raise ValueError("新浪时间字段缺失")
                at = stamp(f[30] + " " + f[31], "%Y-%m-%d %H:%M:%S")
                row = record(code, f[0], at, f[1], f[3], f[4], f[5], f[8], f[9], f[2], provider)
            else:
                f = content.split("~")
                if len(f) < 38 or code[2:] != f[2]:
                    raise ValueError("腾讯代码/字段不符")
                at = stamp(f[30], "%Y%m%d%H%M%S")
                # Tencent lots -> shares. The exact amount in its quote aggregate
                # is yuan; otherwise field 37 is 10,000 yuan. Never multiply twice.
                aggregate = f[35].split("/")
                amount = finite(aggregate[2]) if len(aggregate) == 3 else finite(f[37]) * 10000
                row = record(code, f[1], at, f[5], f[3], f[33], f[34], finite(f[6]) * 100, amount, f[4], provider)
            if code in result:
                raise ValueError("行情响应重复代码")
            result[code] = row
        except (ValueError, IndexError) as error:
            errors.append({"symbol": code, "error": str(error)})
    for error in errors:
        result.pop(error["symbol"],None)
    return result, errors


def current_row(row, expected, now):
    at = dt.datetime.fromisoformat(row["timestamp"])
    return row["date"] == expected and at.time().replace(tzinfo=None) >= dt.time(15) and at <= now + dt.timedelta(seconds=30)


def request_batch(provider, codes, output, batch):
    base = "https://hq.sinajs.cn/list=" if provider == "sina" else "https://qt.gtimg.cn/q="
    request = urllib.request.Request(base + ",".join(codes), headers={"User-Agent": "Mozilla/5.0", "Referer": "https://finance.sina.com.cn/" if provider == "sina" else "https://gu.qq.com/"})
    with urllib.request.urlopen(request, timeout=NETWORK_TIMEOUT) as response:
        payload = response.read(4 * 1024 * 1024 + 1)
    if len(payload) > 4 * 1024 * 1024:
        raise ValueError("近期行情响应超过上限")
    with gzip.open(output / f"{provider}-{batch}.txt.gz", "wb") as stream:
        stream.write(payload)
    rows, errors = parse_quotes(provider, payload.decode("gb18030"))
    return rows, {"provider": provider, "batch": batch, "symbols": len(codes), "received_at": dt.datetime.now(dt.timezone.utc).isoformat(), "payload_sha256": hashlib.sha256(payload).hexdigest(), "parsed": len(rows), "invalid_rows": errors}


def collect(codes, expected, output, now=None, requester=None):
    now = now or dt.datetime.now(CST)
    requester = requester or request_batch
    selected, requests = {}, []
    # Sina uses actual shares/yuan. Tencent fills unavailable/old/invalid rows.
    # Fetch only frozen symbols plus the explicitly identified CSI300 index.
    for provider in ("sina", "tencent"):
        missing = [code for code in codes if code not in selected]
        batches = [missing[i:i+BATCH_SIZE] for i in range(0, len(missing), BATCH_SIZE)]
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            futures = {pool.submit(requester, provider, chunk, output, n): chunk for n, chunk in enumerate(batches)}
            for future in concurrent.futures.as_completed(futures):
                chunk = futures[future]
                try:
                    rows, provenance = future.result()
                    requests.append(provenance)
                    for code, row in rows.items():
                        if code in chunk and current_row(row, expected, now):
                            selected[code] = row
                except Exception as error:
                    requests.append({"provider": provider, "symbols": len(chunk), "error": str(error)})
    return selected, requests


def append_online(original, metadata, quotes, requested, previous, minimum_coverage=2000, minimum_ratio=.95):
    cutoff = int(original["dates"][-1])
    if cutoff != int(previous.replace("-", "")):
        raise ValueError("快照与目标日间缺少多个交易日；在线收盘快照不能回填未留存历史，等待StockDB补齐")
    codes = original["codes"].tolist()
    if len(set(codes))!=len(codes):
        raise ValueError("冻结股票代码重复")
    if not 0<minimum_ratio<=1:
        raise ValueError("覆盖比例必须在(0,1]内")
    old_n = len(original["dates"])
    shape = (old_n+1, len(codes))
    data = {"codes": original["codes"], "dates": np.append(original["dates"], np.asarray([int(requested.replace("-", ""))],dtype=original["dates"].dtype))}
    for key, values in original.items():
        if key in data:
            continue
        if values.shape != original["seen"].shape:
            raise ValueError("原矩阵形状不一致：" + key)
        fill = False if values.dtype.kind == "b" else np.nan
        data[key] = np.full(shape,fill,values.dtype)
        data[key][:old_n] = values
    data["factors"][-1] = original["factors"][-1]
    rejected = []
    providers = {}
    for c, code in enumerate(codes):
        quote = quotes.get(symbol(code))
        if quote is None or quote["date"] != requested:
            rejected.append({"symbol":symbol(code),"reason":"目标日收盘行情缺失"})
            continue
        # An ex-right reference, missing frozen previous observation or damage
        # has no certified action details. Keep the stock unavailable, never
        # infer a factor or apply a made-up dividend to a held position.
        prior = float(original["raw_close"][-1,c])
        damaged = bool(original.get("raw_damaged",np.zeros_like(original["valid"]))[-1,c])
        if not original["valid"][-1,c] or not math.isfinite(prior) or abs(quote["previous_close"]-prior) > PRICE_TOLERANCE or damaged:
            if "raw_damaged" in data:
                data["raw_damaged"][-1,c] = True
            rejected.append({"symbol":symbol(code),"reason":"昨收衔接/公司行动或历史损坏未认证"})
            continue
        data["seen"][-1,c] = True
        data["valid"][-1,c] = True
        for field in FIELDS:
            data["raw_"+field][-1,c] = float(quote[field])
        providers[quote["provider"]] = providers.get(quote["provider"],0)+1
    if not np.all(np.isfinite(data["factors"]) & (data["factors"]>0)):
        raise ValueError("原复权因子无效")
    prior_count = int(original["valid"][-1].sum())
    # Do not allow repeated provider fallbacks to gradually erode coverage.
    floor = max(prior_count,int(metadata.get("online_refresh",{}).get("reference_coverage",prior_count)))
    required = max(minimum_coverage,math.ceil(floor*minimum_ratio))
    actual = int(data["valid"][-1].sum())
    if actual < required:
        raise ValueError(f"在线收盘截面有效{actual}只，不足{required}只；缺失、停牌或复权未认证，停止模型更新")
    for key, values in original.items():
        prefix = data[key] if key == "codes" else data[key][:old_n]
        if not np.array_equal(prefix,values,equal_nan=values.dtype.kind=="f"):
            raise ValueError("在线增量改变了历史前缀："+key)
    updated = dict(metadata)
    updated["quality"] = {**metadata["quality"],"last_date":int(requested.replace("-","")),"sessions":len(data["dates"])}
    updated["online_refresh"] = {"policy":"StockDB preferred; recent completed session from Sina/Tencent; no historical substitution","requested_as_of":requested,"reference_coverage":floor,"valid_coverage":actual,"required_coverage":required,"providers":providers,"excluded_symbols":rejected,"actions":"No uncertified company action is applied; incompatible symbols are unavailable","input_features":"technical-only registered models; unavailable valuation fields remain NaN"}
    return data,updated


def refresh(args, stockdb_error, digest, read_index, write_json, now=None, requester=None):
    now = now or dt.datetime.now(CST)
    expected = dt.datetime.strptime(args.as_of,"%Y-%m-%d").date()
    if expected > now.date() or (expected == now.date() and now.time().replace(tzinfo=None) < dt.time(16)):
        raise ValueError("收盘日线须16:00后读取，不把盘中行情作为完成日日线")
    source,index = args.source.resolve(),args.index.resolve()
    root = args.output.resolve()
    if root == source or source in root.parents or root in source.parents or root in index.parents or any(p.lower() in ("stockdb", "mydb") for p in root.parts):
        raise ValueError("备用输出必须独立于原快照/StockDB目录")
    if (root / "refresh-audit.json").exists():
        raise ValueError("备用不覆盖已有刷新审计，须使用新的研究目录")
    output = root / "provider-fallback"
    output.mkdir(parents=True,exist_ok=False)
    responses=output/"http-responses";responses.mkdir()
    hashes={"snapshot":digest(source/"matrices.npz"),"metadata":digest(source/"matrices-metadata.json"),"index":digest(index)}
    metadata=json.loads((source/"matrices-metadata.json").read_text(encoding="utf8"))
    old_index=read_index(index)
    with np.load(source/"matrices.npz",allow_pickle=False) as archive:
        original={key:archive[key] for key in archive.files}
    if [int(row["date"].replace("-","")) for row in old_index] != original["dates"].tolist() or int(metadata["quality"]["last_date"])!=int(original["dates"][-1]):
        raise ValueError("冻结矩阵、元数据与指数日期不一致")
    previous=getattr(args,"previous_as_of",None)
    if not previous or int(previous.replace("-",""))!=int(original["dates"][-1]):
        raise ValueError("近期备用只补已核对的最后一个交易日，较早缺口需StockDB历史或已留存完成日，不能编造缺失交易日")
    codes=[symbol(code) for code in original["codes"].tolist()]+["sh000300"]
    quotes,requests=collect(codes,args.as_of,responses,now,requester)
    write_json(output/"provider-attempts.json",{"requested_as_of":args.as_of,"stockdb_error":stockdb_error,"requests":requests,"accepted_quotes":len(quotes)})
    base=quotes.get("sh000300")
    if not base or abs(base["previous_close"]-float(old_index[-1]["close"]))>INDEX_PRICE_TOLERANCE:
        raise ValueError("两路行情未提供同日真实沪深300，或指数昨收与冻结前缀不一致")
    data,updated=append_online(original,metadata,quotes,args.as_of,previous)
    exports=output/"exports";exports.mkdir()
    np.savez(exports/"matrices.npz",**data)
    invalid=source/"invalid_records.ndjson.gz";shutil.copyfile(invalid,exports/invalid.name)
    write_json(exports/"matrices-metadata.json",updated)
    new_index=output/"index-sh-000300.ndjson"
    prefix=index.read_bytes()
    if not prefix.endswith(b"\n"):
        raise ValueError("原指数缺换行，不改写字节前缀")
    new_row={"date":args.as_of,"code":"sh.000300",**{field:str(base[field]) for field in ("open","close","high","low")},"source":base["provider"]+" post-close explicit CSI300 quote","retrieved_at":now.isoformat()}
    new_index.write_bytes(prefix+(json.dumps(new_row,ensure_ascii=False,allow_nan=False)+"\n").encode("utf8"))
    if hashes!={"snapshot":digest(source/"matrices.npz"),"metadata":digest(source/"matrices-metadata.json"),"index":digest(index)}:
        raise ValueError("原文件在备用读取时发生变化，拒绝导入")
    result={"state":"updated","mode":"provider_fallback","snapshot":str(exports),"index":str(new_index),"as_of":args.as_of,"requested_as_of":args.as_of,"stockdb_error":stockdb_error,"requests":requests,"checks":updated["online_refresh"],"input_sha256":hashes,"output_sha256":{"snapshot":digest(exports/"matrices.npz"),"metadata":digest(exports/"matrices-metadata.json"),"index":digest(new_index)},"production_admission":False,"limitations":["备用只补与原快照相邻的完成交易日，不补未留存的多日历史","不认证公司行动或历史修订；昨收无法衔接的股票不可用","整体覆盖至少2000只且保持95%；可用并不证明全市场无遗漏","未修改冻结模型、股票轴、历史前缀与已成交账本"]}
    write_json(output/"refresh-audit.json",result)
    write_json(args.output/"refresh-audit.json",result)
    return result
