"""Read-only StockDB-first incremental export with recent post-close fallback for the trusted model runner.

Keeps the frozen stock axis and every existing matrix/index prefix. Never opens
StockDB files, trains models, starts a service or revises old observations.
Writes only a new caller-chosen directory. A holiday/no-new-session is a state.
"""
import argparse
import datetime as dt
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import urllib.parse
import urllib.request

import numpy as np

FIELDS = ("open", "high", "low", "close", "volume", "amount", "is_st", "pe_ttm", "pb", "pct_chg")
FACTORS = ("div", "give", "trans", "mult", "cum")
STOCK = re.compile(r"^(000|001|002|003|300|301|600|601|603|605|688|689)\d{3}$")


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(4 * 1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_json(path, value):
    with Path(path).open("x", encoding="utf8") as stream:
        json.dump(value, stream, ensure_ascii=False, allow_nan=False, indent=2)


def day(value):
    return int(str(value).replace("-", ""))


def finite(value):
    try:
        result = float(value)
    except (TypeError, ValueError):
        return math.nan
    return result if math.isfinite(result) else math.nan


def read_index(path):
    rows = [json.loads(line) for line in Path(path).read_text(encoding="utf8").splitlines() if line.strip()]
    dates = [day(r["date"]) for r in rows]
    if not rows or any(r.get("code") != "sh.000300" or not finite(r.get("close")) > 0 for r in rows) or any(b <= a for a, b in zip(dates, dates[1:])):
        raise ValueError("CSI300文件必须是显式sh.000300、递增唯一日期、有效价格")
    return rows


class Client:
    def __init__(self, endpoint, output):
        parsed = urllib.parse.urlparse(endpoint)
        if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost") or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("仅允许无凭据本机StockDB HTTP接口")
        self.endpoint = endpoint.rstrip("/") + "/"
        self.output = Path(output)
        self.requests = []

    def query(self, label, command, table, key1, key2):
        params = {"cmd": command, "t": table, "json": "1", "k1": key1, "k2": key2}
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(self.endpoint + "?" + urllib.parse.urlencode(params), timeout=12) as response:
            payload = response.read(32 * 1024 * 1024 + 1)
        if len(payload) > 32 * 1024 * 1024:
            raise ValueError("StockDB单次增量响应过大，停止而不截断")
        value = json.loads(payload)
        if not isinstance(value, list):
            raise ValueError("StockDB数组格式无效：" + label)
        with gzip.open(self.output / (label + ".json.gz"), "wb") as stream:
            stream.write(payload)
        self.requests.append({"label": label, "parameters": params, "received_at": dt.datetime.now(dt.timezone.utc).isoformat(), "rows": len(value), "payload_sha256": hashlib.sha256(payload).hexdigest()})
        return value


def fetch_index(as_of, output):
    params = urllib.parse.urlencode({"param": f"sh000300,day,,{as_of},320,qfq"})
    url = "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?" + params
    request = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0", "Referer": "https://gu.qq.com/"})
    with urllib.request.urlopen(request, timeout=20) as response:
        payload = response.read(4 * 1024 * 1024 + 1)
    if len(payload) > 4 * 1024 * 1024:
        raise ValueError("指数响应过大")
    value = json.loads(payload)
    stock = value.get("data", {}).get("sh000300", {})
    records = stock.get("day", stock.get("qfqday"))
    if not isinstance(records, list):
        raise ValueError("真实sh000300指数响应缺日线")
    rows = []
    for record in records:
        if not isinstance(record, list) or len(record) < 5:
            raise ValueError("指数日线格式异常")
        d = str(record[0])
        o, c, h, lo = map(finite, record[1:5])
        dt.datetime.strptime(d, "%Y-%m-%d")
        if min(o, c, h, lo) <= 0 or not all(math.isfinite(v) for v in (o, c, h, lo)) or h < max(o, c) or lo > min(o, c):
            raise ValueError("指数OHLC无效")
        if d <= as_of:
            rows.append({"date": d, "code": "sh.000300", "open": str(o), "close": str(c), "high": str(h), "low": str(lo), "source": "Tencent explicit sh000300 index daily", "retrieved_at": dt.datetime.now(dt.timezone.utc).isoformat()})
    if not rows or any(day(b["date"]) <= day(a["date"]) for a, b in zip(rows, rows[1:])):
        raise ValueError("指数日期重复或乱序")
    with gzip.open(output / "index-response.json.gz", "wb") as stream:
        stream.write(payload)
    return rows, {"url": url, "sha256": hashlib.sha256(payload).hexdigest(), "provider_precision_note": "新指数接口价格通常到两位小数；旧BaoStock四位历史前缀原样保留，不重写"}


def extend_index(original, live, cutoff, requested):
    old = {day(r["date"]): r for r in original}
    overlap = [r for r in live if day(r["date"]) in old]
    if not overlap or cutoff not in {day(r["date"]) for r in overlap}:
        raise ValueError("新增指数窗口未覆盖旧截点，不能验证来源连续性")
    for row in overlap:
        for field in ("open", "high", "low", "close"):
            if abs(finite(row[field]) - finite(old[day(row["date"])][field])) > .006:
                raise ValueError("指数重叠行情发生超出两位报价舍入的修订，不能延续：" + row["date"])
    new = [r for r in live if cutoff < day(r["date"]) <= requested]
    if not new or day(new[-1]["date"]) != requested:
        raise ValueError("CSI300没有所请求的最新完成交易日；拒绝把旧日叫作今天")
    return original + new


def factor_rows(pairs):
    rows = {}
    for pair in pairs:
        if not isinstance(pair, list) or len(pair) != 2 or not isinstance(pair[1], dict):
            raise ValueError("复权key/value结构无效")
        parts = str(pair[0]).split(":")
        if len(parts) != 3 or parts[0] != "复权":
            raise ValueError("复权键无效")
        code, date = parts[1], day(parts[2])
        if not STOCK.fullmatch(code):
            continue
        row = {"code": code, "date": date, **pair[1]}
        if (code, date) in rows or any(not math.isfinite(finite(row.get(k))) for k in FACTORS) or finite(row["cum"]) <= 0 or finite(row["mult"]) <= 0:
            raise ValueError("复权重复或数值无效")
        rows[code, date] = row
    return rows


def compare_factors(prior, current, cutoff, codes, first_observed_date=0):
    before = {k: r for k, r in prior.items() if k[0] in codes and first_observed_date <= k[1] <= cutoff}
    after = {k: r for k, r in current.items() if k[0] in codes and first_observed_date <= k[1] <= cutoff}
    if set(before) != set(after):
        raise ValueError("上次已观测的历史复权记录有新增/丢失，停止延续")
    for key in before:
        if any(abs(finite(before[key][f]) - finite(after[key][f])) > 1e-8 for f in FACTORS):
            raise ValueError("上次已观测历史复权记录被修订，停止延续：" + str(key))


def append_matrices(original, metadata, daily, boundary, live_factors, new_dates, invalid_rows, minimum_coverage=2000, minimum_coverage_ratio=.95):
    codes = original["codes"].tolist()
    ci = {c: i for i, c in enumerate(codes)}
    cutoff = int(original["dates"][-1])
    old_n = len(original["dates"])
    if any(not STOCK.fullmatch(c) for c in codes) or len(ci) != len(codes) or not new_dates or any(d <= cutoff for d in new_dates) or new_dates != sorted(set(new_dates)):
        raise ValueError("冻结代码轴或新增交易日无效")
    boundary_map = {str(r["code"]): r for r in boundary if str(r.get("code")) in ci}
    if len(boundary_map) != sum(str(r.get("code")) in ci for r in boundary):
        raise ValueError("边界日重复股票")
    for c, code in enumerate(codes):
        if not original["seen"][-1, c]:
            continue
        row = boundary_map.get(code)
        if row is None:
            raise ValueError("冻结最后一天的已观测行情缺失：" + code)
        for field in FIELDS:
            prior = original["raw_" + field][-1, c]
            now = np.asarray(finite(row.get(field)), dtype=original["raw_" + field].dtype).item()
            same = prior == now or (np.isnan(prior) and np.isnan(now))
            provider_boundary = day(metadata.get("online_refresh",{}).get("requested_as_of",0)) == cutoff
            if provider_boundary:
                # Check source agreement without replacing a single stored value.
                # Technical models do not use missing online valuation fields.
                if field in ("pe_ttm","pb") and np.isnan(prior):same = True
                elif field in ("open","high","low","close"):same = math.isfinite(prior) and math.isfinite(now) and abs(float(prior)-float(now))<=.001
                elif field in ("volume","amount"):same = math.isfinite(prior) and math.isfinite(now) and abs(float(prior)-float(now))<=max(100.,abs(float(prior))*.003)
                elif field=="pct_chg":same = math.isfinite(prior) and math.isfinite(now) and abs(float(prior)-float(now))<=.05
            if not same:
                raise ValueError("冻结边界行情被修订或跨源不一致：" + code + "/" + field)
    shape = (old_n + len(new_dates), len(codes))
    data = {"codes": original["codes"], "dates": np.concatenate((original["dates"], np.asarray(new_dates, dtype=original["dates"].dtype)))}
    for key, values in original.items():
        if key in ("codes", "dates"):
            continue
        if values.shape != original["seen"].shape:
            raise ValueError("矩阵字段形状不一致：" + key)
        fill = False if values.dtype.kind == "b" else np.nan
        data[key] = np.full(shape, fill, values.dtype)
        data[key][:old_n] = values
    data["factors"][old_n:] = original["factors"][-1]
    di = {d: old_n + i for i, d in enumerate(new_dates)}
    observed = set()
    ignored = set()
    for row in daily:
        code = str(row.get("code"))
        if code not in ci:
            if STOCK.fullmatch(code):
                ignored.add(code)
            continue
        d = day(row["date"])
        if d not in di or (code, d) in observed or not isinstance(row.get("is_st"), bool) or any(k not in row for k in FIELDS):
            raise ValueError("新增行情日期/重复/ST/schema异常")
        observed.add((code, d))
        i, c = di[d], ci[code]
        data["seen"][i, c] = True
        for field in FIELDS:
            data["raw_" + field][i, c] = finite(row.get(field))
    raw = {k[4:]: v for k, v in data.items() if k.startswith("raw_")}
    if not 0 < minimum_coverage_ratio <= 1:
        raise ValueError("相对覆盖阈值必须在(0,1]内")
    frozen_coverage = int(original["valid"][-1].sum())
    coverage_checks = {}
    for d, i in di.items():
        finite_market = np.logical_and.reduce([np.isfinite(raw[field][i]) for field in ("open", "high", "low", "close", "volume", "amount")])
        prices = finite_market & (raw["open"][i] > 0) & (raw["low"][i] > 0) & (raw["close"][i] > 0) & (raw["high"][i] + .011 >= np.maximum(raw["open"][i], raw["close"][i])) & (raw["low"][i] - .011 <= np.minimum(raw["open"][i], raw["close"][i]))
        data["valid"][i] = data["seen"][i] & prices & (raw["volume"][i] > 0) & (raw["amount"][i] > 0)
        # A partial StockDB batch may exceed 2,000 and agree between two query
        # shapes. Retain >=95% of both the source cutoff and previous completed
        # day's valid coverage, so successive partial days cannot erode the floor.
        previous_coverage = int(data["valid"][i - 1].sum())
        reference_coverage = max(frozen_coverage, previous_coverage)
        required_coverage = max(minimum_coverage, math.ceil(reference_coverage * minimum_coverage_ratio))
        actual_coverage = int(data["valid"][i].sum())
        coverage_checks[str(d)] = {"actual": actual_coverage, "previous_completed": previous_coverage, "source_cutoff": frozen_coverage, "required": required_coverage}
        if actual_coverage < required_coverage:
            raise ValueError(f"新增日{d}有效覆盖{actual_coverage}不足{required_coverage}（至少{minimum_coverage}，且保留源截点/上一完成日较高覆盖的{minimum_coverage_ratio:.0%}）；可能半批更新或大量停牌，停止而不认作完整截面")
    if "raw_damaged" in data:
        for row in invalid_rows:
            parts = str(row.get("key", "")).removeprefix("k").split(":")
            code = row.get("code") or (parts[1] if len(parts) > 2 else None)
            d = row.get("date") or (parts[2] if len(parts) > 2 else None)
            if str(code) in ci and d and str(d).isdigit():
                at = int(np.searchsorted(data["dates"], int(d)))
                data["raw_damaged"][max(old_n, at):at + 61, ci[str(code)]] = True
    events = list(metadata["events"])
    for (code, d), event in sorted(live_factors.items(), key=lambda pair: (pair[0][1], pair[0][0])):
        if code not in ci or d <= cutoff or d > new_dates[-1]:
            continue
        if d not in di:
            raise ValueError("新增公司行动不在真实市场日中")
        c, i = ci[code], di[d]
        previous = [r for (cc, dd), r in live_factors.items() if cc == code and dd <= cutoff]
        live_before = finite(max(previous, key=lambda r: r["date"])["cum"]) if previous else 1.0
        scale = float(original["factors"][-1, c]) / live_before
        data["factors"][i:, c] = finite(event["cum"]) * scale
        events.append({"i": i, "c": c, "event": event})
    for key, values in original.items():
        prefix = data[key] if key == "codes" else data[key][:old_n]
        if not np.array_equal(prefix, values, equal_nan=values.dtype.kind == "f"):
            raise ValueError("新增导出改变历史前缀：" + key)
    if not np.all(np.isfinite(data["factors"]) & (data["factors"] > 0)):
        raise ValueError("复权连续性失败")
    result_meta = dict(metadata)
    result_meta["events"] = events
    result_meta["quality"] = {**metadata["quality"], "stocks": len(codes), "sessions": len(data["dates"]), "last_date": new_dates[-1], "factor_events": len(events)}
    return data, result_meta, {"ignored_new_symbols": sorted(ignored), "coverage": {str(d): int(data["valid"][i].sum()) for d, i in di.items()}, "coverage_gate": {"minimum_absolute": minimum_coverage, "minimum_ratio": minimum_coverage_ratio, "reference": "max(source cutoff valid count, previous completed day valid count)", "days": coverage_checks, "limitation": "5% tolerance is a conservative availability guard, not proof all listed shares are updated; larger suspension/data gaps stop refresh"}, "all_matrix_prefixes_exact": True, "historical_events_unchanged": True}


def refresh_stockdb(args):
    source, index, output = args.source.resolve(), args.index.resolve(), args.output.resolve()
    as_of = dt.datetime.strptime(args.as_of, "%Y-%m-%d").date()
    now = dt.datetime.now(dt.timezone(dt.timedelta(hours=8)))
    if as_of > now.date() or (as_of == now.date() and now.time().replace(tzinfo=None) < dt.time(16)):
        raise ValueError("只接受已完成交易日，16时之前不采用当天日线")
    if output.exists() or source == output or source in output.parents or any(p.lower() in ("stockdb", "mydb") for p in output.parts):
        raise ValueError("输出必须是不存在的独立研究目录，禁止写StockDB目录/覆盖旧快照")
    original_meta = json.loads((source / "matrices-metadata.json").read_text(encoding="utf8"))
    cutoff = int(original_meta["quality"]["last_date"])
    hashes = {"snapshot": digest(source / "matrices.npz"), "metadata": digest(source / "matrices-metadata.json"), "index": digest(index)}
    original_index = read_index(index)
    if day(original_index[-1]["date"]) != cutoff:
        raise ValueError("已有指数截止日与快照不一致")
    output.mkdir(parents=True)
    responses = output / "http-responses"
    responses.mkdir()
    client = Client(args.endpoint, responses)
    requested = day(args.as_of)
    if requested == cutoff:
        result = {"state":"up_to_date","mode":"saved_current","snapshot":str(source),"index":str(index),"as_of":args.as_of,"requested_as_of":args.as_of,"input_sha256":hashes,"requests":[],"production_admission":False}
        write_json(output / "refresh-audit.json",result)
        return result
    probe = client.query("daily-requested-all", "vals", "日k", "all:", "key:" + str(requested))
    if requested < cutoff:
        raise ValueError("请求截止日早于当前快照，不可倒用未来导出")
    probe_keys = [(str(r.get("code")), day(r.get("date", 0))) for r in probe]
    if any(d != requested for _, d in probe_keys) or len(probe_keys) != len(set(probe_keys)):
        raise ValueError("指定日全市场响应日期不同或重复，不能确认数据当前")
    if requested == cutoff or not probe:
        result = {"state": "up_to_date" if requested == cutoff and probe else "waiting_market_data", "snapshot": str(source), "index": str(index), "as_of": str(original_index[-1]["date"]), "requested_as_of": args.as_of, "input_sha256": hashes, "requests": client.requests, "production_admission": False}
        write_json(output / "stockdb-refresh-audit.json", result)
        return result
    live_index, index_provenance = fetch_index(args.as_of, responses)
    merged_index = extend_index(original_index, live_index, cutoff, requested)
    new_dates = [day(r["date"]) for r in merged_index if day(r["date"]) > cutoff]
    daily, boundary = [], []
    start = int((dt.datetime.strptime(str(cutoff), "%Y%m%d").date() + dt.timedelta(days=1)).strftime("%Y%m%d"))
    for prefix in ("0", "3", "6"):
        daily.extend(client.query("daily-new-" + prefix, "vals", "日k", "qz:" + prefix, f"fwz:{start},{requested}"))
        boundary.extend(client.query("daily-boundary-" + prefix, "vals", "日k", "qz:" + prefix, "key:" + str(cutoff)))
    expected = {(str(r["code"]), day(r["date"])) for r in probe if STOCK.fullmatch(str(r.get("code", "")))}
    if expected != {(str(r["code"]), day(r["date"])) for r in daily if STOCK.fullmatch(str(r.get("code", ""))) and day(r["date"]) == requested}:
        raise ValueError("最新全市场查询与批前缀查询不同，不导出部分数据")
    live_factors = factor_rows(client.query("factor-history-audit", "get", "复权", "all:", "all:"))
    prior_api = source.parent / "http-responses/factor-history-audit.json.gz"
    if prior_api.is_file():
        with gzip.open(prior_api, "rt", encoding="utf8") as stream:
            prior = factor_rows(json.load(stream))
    else:
        prior = {(str(r["event"]["code"]), day(r["event"]["date"])): r["event"] for r in original_meta["events"]}
    with np.load(source / "matrices.npz", allow_pickle=False) as archive:
        original = {k: archive[k] for k in archive.files}
    if [day(r["date"]) for r in original_index] != original["dates"].tolist():
        raise ValueError("原CSI300与快照日期轴不一致")
    # The initial matrix metadata only records actions inside its date axis.
    # It cannot attest 1991-2017 actions newly visible in the full API response.
    # After the first successful refresh the whole response is retained, and
    # every previously observed historical API event is compared thereafter.
    first_observed_date = 0 if prior_api.is_file() else int(original["dates"][0])
    compare_factors(prior, live_factors, cutoff, set(original["codes"].tolist()), first_observed_date)
    invalid_path = source / "invalid_records.ndjson.gz"
    with gzip.open(invalid_path, "rt", encoding="utf8") as stream:
        invalid = [json.loads(line) for line in stream if line.strip()]
    data, metadata, checks = append_matrices(original, original_meta, daily, boundary, live_factors, new_dates, invalid)
    exports = output / "exports"
    exports.mkdir()
    np.savez(exports / "matrices.npz", **data)
    shutil.copyfile(invalid_path, exports / invalid_path.name)
    metadata["live_refresh"] = {"endpoint": args.endpoint, "requested_as_of": args.as_of, "stock_axis_policy": "冻结模型代码轴保持不变；新上市代码单列未知，不自动插列", "prefix_source_sha256": hashes, "index_source": index_provenance}
    write_json(exports / "matrices-metadata.json", metadata)
    new_index = output / "index-sh-000300.ndjson"
    original_bytes = index.read_bytes()
    if original_bytes and not original_bytes.endswith(b"\n"):
        raise ValueError("原指数文件无结尾换行，拒绝改字节前缀")
    with new_index.open("xb") as stream:
        stream.write(original_bytes)
        for row in merged_index[len(original_index):]:
            stream.write((json.dumps(row, ensure_ascii=False, allow_nan=False) + "\n").encode("utf8"))
    if hashes != {"snapshot": digest(source / "matrices.npz"), "metadata": digest(source / "matrices-metadata.json"), "index": digest(index)}:
        raise ValueError("源文件在计算期间改变，拒绝混用")
    checks["factor_history_first_observed_date"] = first_observed_date
    result = {"state": "updated", "snapshot": str(exports), "index": str(new_index), "as_of": args.as_of, "input_sha256": hashes, "output_sha256": {"snapshot": digest(exports / "matrices.npz"), "metadata": digest(exports / "matrices-metadata.json"), "index": digest(new_index)}, "checks": checks, "requests": client.requests, "index_provenance": index_provenance, "production_admission": False, "limitations": ["只拒绝上次已观测复权历史与最后完成日行情修订；首次只核矩阵日期轴内事件，此后核留存的全历史API事件；不宣称实时重新审计全八年原始日线", "代码轴固定，新增上市股不自动获得模型资格；日线、开盘代理，不具备分时抢先提醒", "指数新来源两位报价舍入与旧前缀来源不同，已核对重叠价格，不认证历史PIT"]}
    write_json(output / "refresh-audit.json", result)
    return result


def refresh(args):
    # An existing export can be resumed only by the model job, never by writing
    # a new fallback inside it after the StockDB path refuses an overwrite.
    if getattr(args,"output",None) is not None and args.output.exists():
        raise ValueError("刷新输出目录已存在，请使用新的独立研究目录")
    try:
        result = refresh_stockdb(args)
        if result["as_of"] == args.as_of:
            result.setdefault("mode","stockdb")
            return result
        error = f"StockDB最近完成日仅到{result['as_of']}，缺{args.as_of}"
    except Exception as failure:
        error = str(failure)
    # Only the independently audited recent session is a permitted fallback.
    # Frozen prefix/StockDB data/model parameters are never written or revised.
    import recent_market_fallback
    try:
        return recent_market_fallback.refresh(args,error,digest,read_index,write_json)
    except Exception as failure:
        raise ValueError(f"StockDB及近期行情备用均未取得合格日线；StockDB：{error}；行情备用：{failure}") from failure


def self_check():
    original = {"codes": np.array(["000001", "600000", "300001"]), "dates": np.array([20260924], np.int32), "seen": np.ones((1, 3), bool), "valid": np.ones((1, 3), bool), "factors": np.ones((1, 3))}
    for f in FIELDS:
        original["raw_" + f] = np.full((1, 3), 0 if f == "is_st" else 10, np.float32)
    original["raw_damaged"] = np.zeros((1, 3), bool)
    meta = {"quality": {"last_date": 20260924}, "events": []}
    rows = [{"code": c, "date": 20260928, **{f: False if f == "is_st" else 10 for f in FIELDS}} for c in original["codes"]]
    boundary = [{**r, "date": 20260924} for r in rows]
    data, updated, checks = append_matrices(original, meta, rows, boundary, {}, [20260928], [], minimum_coverage=3)
    assert data["dates"].tolist() == [20260924, 20260928] and checks["all_matrix_prefixes_exact"] and updated["events"] == []
    for key, value in original.items():
        assert np.array_equal(data[key] if key == "codes" else data[key][:1], value, equal_nan=value.dtype.kind == "f")
    revised = [dict(r) for r in boundary]
    revised[0]["close"] = 11
    for daily, old in [(rows, revised), (rows + [rows[0]], boundary), (rows[:2], boundary)]:
        try:
            append_matrices(original, meta, daily, old, {}, [20260928], [], minimum_coverage=3)
        except ValueError:
            pass
        else:
            raise AssertionError("revision/duplicate/partial cross-section accepted")
    before = {("000001", 20260924): {f: 1 for f in FACTORS}}
    prehistory = {**before, ("000001", 20100101): {f: 1 for f in FACTORS}}
    compare_factors(before, prehistory, 20260924, {"000001"}, 20180102)
    try:
        compare_factors(before, prehistory, 20260924, {"000001"})
    except ValueError:
        pass
    else:
        raise AssertionError("a previously retained full API must reject extra historical actions")
    changed = {k: {**v, "cum": 2} for k, v in before.items()}
    try:
        compare_factors(before, changed, 20260924, {"000001"})
    except ValueError:
        pass
    else:
        raise AssertionError("historical action revision accepted")
    old_index = [{"date": "2026-09-24", "code": "sh.000300", **{f: "10.1234" for f in ("open", "high", "low", "close")}}]
    live = [{**old_index[0], **{f: "10.12" for f in ("open", "high", "low", "close")}}, {**old_index[0], "date": "2026-09-28"}]
    assert extend_index(old_index, live, 20260924, 20260928)[0] == old_index[0]
    # Exercise the complete export path with a real-size cross-section and deterministic
    # read-only response fixtures; no network or mutable source is involved.
    import tempfile
    from types import SimpleNamespace
    global Client, fetch_index
    real_client, real_index = Client, fetch_index
    with tempfile.TemporaryDirectory(prefix="bull-refresh-full-check-") as temporary:
        root = Path(temporary)
        source = root / "source"
        source.mkdir()
        codes = np.asarray([f"001{i:03d}" for i in range(1000)] + [f"002{i:03d}" for i in range(1000)] + ["300001"])
        original = {"codes": codes, "dates": np.array([20260924], np.int32), "seen": np.ones((1, len(codes)), bool), "valid": np.ones((1, len(codes)), bool), "factors": np.ones((1, len(codes)))}
        for f in FIELDS:
            original["raw_" + f] = np.full((1, len(codes)), 0 if f == "is_st" else 10, np.float32)
        original["raw_damaged"] = np.zeros((1, len(codes)), bool)
        np.savez(source / "matrices.npz", **original)
        write_json(source / "matrices-metadata.json", meta)
        with gzip.open(source / "invalid_records.ndjson.gz", "wt", encoding="utf8"):
            pass
        index_path = root / "index.ndjson"
        index_path.write_text(json.dumps(old_index[0]) + "\n", encoding="utf8")
        boundary = [{"code": c, "date": 20260924, **{f: False if f == "is_st" else 10 for f in FIELDS}} for c in codes]
        rows = [{**r, "date": 20260928} for r in boundary]
        # More than 2,000 rows must still fail when a 2,400-stock source loses
        # over 5% of its previous coverage. No fixture becomes market evidence.
        larger_codes = np.asarray([f"001{i:03d}" for i in range(1000)] + [f"002{i:03d}" for i in range(1000)] + [f"300{i:03d}" for i in range(400)])
        larger_original = {k: (larger_codes if k == "codes" else v if k == "dates" else np.repeat(v[:, :1], len(larger_codes), axis=1)) for k, v in original.items()}
        larger_boundary = [{"code": c, "date": 20260924, **{f: False if f == "is_st" else 10 for f in FIELDS}} for c in larger_codes]
        partial_above_2000 = [{**r, "date": 20260928} for r in larger_boundary[:2100]]
        try:
            append_matrices(larger_original, meta, partial_above_2000, larger_boundary, {}, [20260928], [])
        except ValueError as error:
            assert "2280" in str(error) and "2100" in str(error)
        else:
            raise AssertionError("partial batch above 2000 passed relative coverage gate")
        class FixtureClient(real_client):
            def query(self, label, command, table, key1, key2):
                if label == "factor-history-audit":
                    result = []
                elif label == "daily-requested-all":
                    result = rows
                else:
                    prefix = label[-1]
                    result = [r for r in (boundary if "boundary" in label else rows) if r["code"].startswith(prefix)]
                with gzip.open(self.output / (label + ".json.gz"), "wt", encoding="utf8") as stream:
                    json.dump(result, stream)
                self.requests.append({"label": label, "fixture": True, "rows": len(result)})
                return result
        try:
            Client = FixtureClient
            fetch_index = lambda *_: (live, {"provider": "deterministic synthetic CSI300 fixture, never market evidence"})
            before = digest(source / "matrices.npz")
            import recent_market_fallback
            for blocked in (source, root / "stockdb/exports/new", root / "mydb/new"):
                args = SimpleNamespace(source=source, index=index_path, as_of="2026-09-28", endpoint="http://127.0.0.1:1", output=blocked)
                for invoke in (lambda: refresh_stockdb(args), lambda: recent_market_fallback.refresh(args,"unavailable",digest,read_index,write_json)):
                    try:
                        invoke()
                    except ValueError:
                        pass
                    else:
                        raise AssertionError("source/StockDB output accepted")
            result = refresh(SimpleNamespace(source=source, index=index_path, as_of="2026-09-28", endpoint="http://127.0.0.1:7899", output=root / "data/research-workspace/appended"))
            assert result["state"] == "updated" and result["as_of"] == "2026-09-28"
            assert digest(source / "matrices.npz") == before
            with np.load(Path(result["snapshot"]) / "matrices.npz", allow_pickle=False) as saved:
                for key, value in original.items():
                    assert np.array_equal(saved[key] if key == "codes" else saved[key][:1], value, equal_nan=value.dtype.kind == "f")
                assert saved["dates"].tolist() == [20260924, 20260928]
            assert Path(result["index"]).read_bytes().startswith(index_path.read_bytes())
            assert (Path(result["snapshot"]).parent / "http-responses/factor-history-audit.json.gz").is_file()
        finally:
            Client, fetch_index = real_client, real_index
    print(json.dumps({"self_check": "passed", "checks": ["unchanged axes/prefix", "reject revised boundary", "reject duplicates and incomplete coverage", "reject >2000 partial batch below 95% prior coverage", "reject revised historical actions", "explicit CSI300 append with documented rounding"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path)
    parser.add_argument("--index", type=Path)
    parser.add_argument("--endpoint", default="http://127.0.0.1:7899")
    parser.add_argument("--as-of")
    parser.add_argument("--previous-as-of", help="经交易日历核对的目标日前一交易日")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if args.self_check:
        self_check()
    elif any(v is None for v in (args.source, args.index, args.as_of, args.output)):
        parser.error("--source --index --as-of --output required")
    else:
        print(json.dumps(refresh(args), ensure_ascii=False, allow_nan=False))
