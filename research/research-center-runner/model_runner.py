"""Trusted, finite A-share model replay/observation. No refit, shell, network or broker.

Input artifacts are verified before importing any research execution module.
Only this fixed runner is launched by the app; an AI-generated script is never run.
"""
import argparse
import datetime as dt
import hashlib
import json
import math
import pickle
import sys
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
BASE = ROOT / "research/ashare-models-2026-10-01"
OPEN = ROOT / "research/ashare-open-2026-10-01"
MODELS = ("breadth22_h20", "index26_h20", "breadth22_excess_csi20", "breadth22_rank20", "breadth22_open_downside20")
COMPARISONS = ("baseline", "holding15", "cost_double", "staged", "verified_actions")


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(4 * 1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def validate_choice(model, hold, comparison, mode):
    if model not in MODELS or comparison not in COMPARISONS or mode not in ("replay", "forward"):
        raise ValueError("仅支持五个已核验模型、五个固定对照和 replay/forward")
    if hold != (15 if comparison == "holding15" else 20):
        raise ValueError("holding15 只改变执行期限为15；基准/双成本保持20日标签与期限")


def current_scores(np, codes, as_of, score, eligible, threshold):
    """Whole eligible scored universe; negative results must stay observable."""
    if score.shape != eligible.shape or score.shape != (len(codes),):
        raise ValueError("当前评分与股票轴不一致")
    return [{"symbol": ("sh" if codes[c].startswith("6") else "sz") + codes[c],
             "score": float(score[c]), "threshold": threshold, "as_of": as_of}
            for c in np.flatnonzero(eligible & np.isfinite(score))]


def verified_actions(np, registry, codes, dates, events, original_details, original_coverage):
    """Apply an explicitly selected, SHA-bound accounting evidence version.

    Exact public ordinary-share schedules affect execution only. They are not
    retrospective alpha features and never silently alter the old baseline.
    """
    row = registry["files"].get("verified_action_overlay")
    if row is None:
        raise ValueError("核验权益日期对照缺少受信额外overlay")
    overlay_path = ROOT / row["path"]
    overlay = json.loads(overlay_path.read_text(encoding="utf8"))
    if overlay.get("schema") != "issuer-tail-action-overlay-v1" or not isinstance(overlay.get("events"), list):
        raise ValueError("核验权益日期overlay格式无效")
    details, provenance, visited = dict(original_details), [], set()
    trusted = {(v["path"].replace("\\", "/"), v["sha256"]) for v in registry["files"].values()}
    for event in overlay["events"]:
        code = event["code"]
        ex_day = int(event["ex_date"].replace("-", ""))
        if not event.get("source_verified_flag") or not event.get("units_match_stockdb") or (code, ex_day) in visited:
            raise ValueError("权益事件未认证/重复")
        visited.add((code, ex_day))
        pdf = event["issuer_pdf"].replace("\\", "/")
        if (pdf, event["issuer_pdf_sha256"]) not in trusted or digest(ROOT / pdf) != event["issuer_pdf_sha256"]:
            raise ValueError("权益公告原文不属于受信输入或SHA不同")
        dates_known = [dt.date.fromisoformat(event[k]) for k in ("source_announcement_date", "ex_date", "pay_date", "stock_listing_date")]
        if dates_known[0] > dates_known[1] or dates_known[2] < dates_known[1] or dates_known[3] < dates_known[1]:
            raise ValueError("权益公告/支付/上市日期先后关系无效")
        if code not in codes:
            continue
        i, c = int(np.searchsorted(dates, ex_day)), codes.index(code)
        if i >= len(dates) or int(dates[i]) != ex_day:
            continue
        actual = events.get((i, c))
        declared = event["stockdb_event"]
        if actual is None or any(abs(float(actual[k]) - float(declared[k])) > 1e-8 for k in ("div", "give", "trans")):
            raise ValueError("权益公告与冻结公司行动的每原股单位不同")
        pay_i = int(np.searchsorted(dates, int(event["pay_date"].replace("-", "")), side="right"))
        release_i = int(np.searchsorted(dates, int(event["stock_listing_date"].replace("-", ""))))
        details[i, c] = {**details.get((i, c), {}), "pay_i": pay_i, "release_i": release_i}
        provenance.append({k: event[k] for k in ("code", "ex_date", "pay_date", "stock_listing_date", "source_announcement_date", "source_url", "issuer_pdf_sha256", "evidence_received_at")})
    return details, {**original_coverage, "mapped_events": len(details), "execution_evidence_version": "verified_actions-v1",
                     "extra_overlay_path": str(overlay_path), "extra_overlay_sha256": row["sha256"], "extra_mapped_events": len(provenance),
                     "source_provenance": provenance, "as_of": dt.datetime.strptime(str(int(dates[-1])), "%Y%m%d").date().isoformat(),
                     "limitation": "原受信公司行动＋额外逐事件公告认证；仅纠正记账，现时取证不是当年可用alpha特征，未认证其他事件仍挂起；现金次日可用、股息税和零碎股仍为原保守代理"}


def verify_sources():
    registry = json.loads((HERE / "registry.json").read_text(encoding="utf8"))
    hashes = {}
    for key, row in registry["files"].items():
        path = (ROOT / row["path"]).resolve()
        if not path.is_relative_to(ROOT) or digest(path) != row["sha256"]:
            raise ValueError(f"受信研究输入漂移或缺失：{key}；不会训练替代模型")
        hashes[key] = row["sha256"]
    return registry, hashes


def load_npz(np, path):
    with np.load(path, allow_pickle=False) as data:
        return {key: data[key] for key in data.files}


def validate_extension(np, data, original):
    """No re-indexing and no correction of already frozen history is permitted."""
    if set(data) != set(original):
        raise ValueError("新快照字段集合不同，须先审计迁移，不能混用旧评分")
    old_n = len(original["dates"])
    if len(data["dates"]) < old_n:
        raise ValueError("新快照历史缩短")
    for key, old in original.items():
        candidate = data[key] if key == "codes" else data[key][:old_n]
        if candidate.shape != old.shape or not np.array_equal(candidate, old, equal_nan=old.dtype.kind == "f"):
            raise ValueError(f"新快照改变已冻结历史/股票轴：{key}")


def emit(args):
    validate_choice(args.model_id, args.holding_days, args.comparison, args.mode)
    output = args.output.resolve()
    if output.exists():
        raise ValueError("输出已存在，拒绝覆盖账本")
    registry, hashes = verify_sources()
    # All executable imports and all pickle loads occur after trusted hashes pass.
    sys.path.insert(0, str(BASE))
    import model_study as r
    np, s, e, m, rules = r.np, r.s, r.e, r.m, r.rules
    snapshot = args.snapshot.resolve()
    updated_sources = {"snapshot": digest(snapshot / "matrices.npz"),
                       "metadata": digest(snapshot / "matrices-metadata.json"), "index": digest(args.index)}
    data = load_npz(np, snapshot / "matrices.npz")
    if snapshot != r.DATA.resolve():
        validate_extension(np, data, load_npz(np, r.DATA / "matrices.npz"))
    codes, dates = data["codes"].tolist(), data["dates"]
    if any(not s.STOCK.fullmatch(code) for code in codes) or len(set(codes)) != len(codes):
        raise ValueError("只允许冻结沪深普通股代码轴，排除北交所/B股")
    if np.any(np.diff(dates) <= 0) or int(dates[-1]) // 10000 > 2026:
        raise ValueError("日期轴无效；2027须独立核验冻结训练版本，本入口不自动训练")
    local = dt.datetime.now(dt.timezone(dt.timedelta(hours=8)))
    last_day = dt.datetime.strptime(str(int(dates[-1])), "%Y%m%d").date()
    if last_day > local.date() or (last_day == local.date() and local.hour < 16):
        raise ValueError("快照包含尚未完成的行情日")
    raw = {k.removeprefix("raw_"): v for k, v in data.items() if k.startswith("raw_")}
    valid, seen, factors = data["valid"], data["seen"], data["factors"]
    if valid.shape != (len(dates), len(codes)) or not np.all(np.isfinite(factors) & (factors > 0)):
        raise ValueError("全截面形状或复权因子无效")
    metadata = json.loads((snapshot / "matrices-metadata.json").read_text(encoding="utf8"))
    original_meta = json.loads((r.DATA / "matrices-metadata.json").read_text(encoding="utf8"))
    old_events = [v for v in metadata["events"] if v["i"] < 2123]
    if old_events != original_meta["events"]:
        raise ValueError("新快照修改历史公司行动，不可延续旧模型账户")
    events = {(v["i"], v["c"]): v["event"] for v in metadata["events"]}
    if len(events) != len(metadata["events"]) or any(not (0 <= i < len(dates) and 0 <= c < len(codes)) for i, c in events):
        raise ValueError("公司行动事件重复/轴错位")
    f = s.features(raw, valid, factors, seen)
    _, arrays, market = m.inputs(raw, valid, f)
    index_path = args.index.resolve()
    index, index_asof = r.cli.index_inputs(dates, index_path, np, s)
    # index_inputs rejects absent sessions; explicit identity and exact calendar are checked here too.
    index_rows = [json.loads(line) for line in index_path.read_text(encoding="utf8").splitlines() if line.strip()]
    if any(v.get("code") != "sh.000300" for v in index_rows):
        raise ValueError("指数必须显式为CSI300，不能把000300股票当指数")
    if [int(v["date"].replace("-", "")) for v in index_rows] != dates.tolist():
        raise ValueError("股票快照与CSI300市场交易日不完整匹配，先更新指数日历")
    original_index_rows = [json.loads(line) for line in (ROOT / registry["files"]["index"]["path"]).read_text(encoding="utf8").splitlines() if line.strip()]
    for old, new in zip(original_index_rows, index_rows[:2123]):
        if old != new:
            raise ValueError("指数新文件改变冻结历史版本，不能延续旧推断")
    spec = registry["models"][args.model_id]
    score_path = ROOT / registry["files"][args.model_id + "_score"]["path"]
    frozen = np.load(score_path, allow_pickle=False)
    if frozen.shape != (2123, len(codes)):
        raise ValueError("评分缓存与冻结股票/日期轴不符")
    score = np.full(valid.shape, np.nan, np.float32)
    score[:2123] = frozen
    if len(dates) > 2123:
        model_path = ROOT / registry["files"][args.model_id + "_model2026"]["path"]
        with model_path.open("rb") as stream:
            model = pickle.load(stream)  # Own fixed artifact hash was verified, never AI/imported pickle.
        width = registry["models"][args.model_id]["feature_count"]
        for i in range(2123, len(dates)):
            cols = np.flatnonzero(f["eligible"][i])
            if len(cols):
                score[i, cols] = m.predict(model, r.a.xday(arrays, market, index, i, cols)[:, :width])
    np.testing.assert_array_equal(score[:2123], frozen)
    threshold = spec["signal_threshold"]
    execution_score = score - threshold
    f["score"] = execution_score
    f["signals"][args.model_id] = f["eligible"] & np.isfinite(score) & (score > threshold)
    details, action_coverage = rules.action_map(codes, dates, events)
    if args.comparison == "verified_actions":
        details, action_coverage = verified_actions(np, registry, codes, dates, events, details, action_coverage)
    rules.SZ_EFFECTIVE = 20260706
    rules.self_check()
    latest = int(dates[-1])
    anchor = args.forward_start or latest
    if args.mode == "forward" and anchor not in dates:
        raise ValueError("前向起点须为已完成且在冻结日期轴中的市场日")
    if args.mode == "forward" and anchor < 20260930:
        raise ValueError("前向观察不能回填过去起点，最早为本轮冻结9/30")
    span = (20200101, latest) if args.mode == "replay" else (anchor + 1, latest)
    old_open = e.tradable_open
    try:
        e.tradable_open = rules.tradable_open
        ledger = e.backtest(args.model_id, *span, codes, dates, raw, valid, factors, events, f,
                            multiplier=2 if args.comparison == "cost_double" else 1,
                            full_entry=args.comparison != "staged", hold=args.holding_days, journal=True,
                            exit_policy="legacy" if args.comparison == "staged" else "time",
                            max_positions=10, action_details=details)
    finally:
        e.tradable_open = old_open
    if args.mode == "forward":
        ledger["curve"].insert(0, {"date": anchor, "equity": 100000., "cash": 100000.,
                                 "dividend_receivable": 0., "positions": 0})
    accounting = rules.accounting(ledger)
    if accounting["minimum_cash_cny"] < 0 or abs(accounting["reconcile_cny"]) > .1:
        raise ValueError("现金或周期/未平损益不守恒，拒绝导出")
    di = {int(day): i for i, day in enumerate(dates)}
    ci = {code: c for c, code in enumerate(codes)}
    for order in ledger["orders"]:
        if order["side"] == "buy":
            i, c = di[order["date"]], ci[order["code"]]
            initial = order["reason"] != "strength_confirm_add"
            if raw["is_st"][i, c] != 0 or (initial and not f["signals"][args.model_id][i - 1, c]):
                raise ValueError("买单未满足前日冻结信号/当日非ST")
            if not initial and (args.comparison != "staged" or not valid[i - 1, c]
                                or raw["is_st"][i - 1, c] != 0 or order["holding_session"] > 7
                                or f["close"][i - 1, c] <= f["ma10"][i - 1, c]):
                raise ValueError("分批加仓没有受信前日走强条件")
    cash = ledger["curve"][-1]["cash"]
    scored = current_scores(np, codes, last_day.isoformat(), score[-1], f["eligible"][-1], threshold)
    watch = []
    for c in np.flatnonzero(f["signals"][args.model_id][-1]):
        close = float(raw["close"][-1, c])
        position_fraction = .032 if args.comparison == "staged" else .08
        budget = min(max(0, cash - 10), ledger["curve"][-1]["equity"] * position_fraction, float(raw["amount"][-1, c]) * .01)
        price = close * (1 + .001 * (2 if args.comparison == "cost_double" else 1))
        qty = s.lot_buy(codes[c], budget, price)
        while qty > 0 and qty * price + s.fee(qty * price, False, latest, 2 if args.comparison == "cost_double" else 1) > budget:
            qty = qty - (1 if codes[c].startswith(("688", "689")) else 100)
            if codes[c].startswith(("688", "689")) and qty < 200:
                qty = 0
        watch.append({"symbol": ("sh" if codes[c].startswith("6") else "sz") + codes[c],
                      "score": float(score[-1, c]), "threshold": threshold, "as_of": last_day.isoformat(), "close": close,
                      "signal_eligible": True, "cash_reference_quantity": qty,
                      "cash_reference_eligible": qty > 0,
                      "cash_reference_stage": "initial_probe_3.2pct" if args.comparison == "staged" else "full_target_8pct",
                      "technical_state": "已完成日有效、非ST、历史长度/量额/价格/坏行禁入检查通过",
                      "reason": "未准入观察；下一开盘、涨跌停、T+1与现金实际成交仍未知"})
    watch.sort(key=lambda row: (-row["score"], row["symbol"]))
    if {row["symbol"]: row["score"] for row in watch} != {row["symbol"]: row["score"] for row in scored if row["score"] > threshold}:
        raise ValueError("当前全评分与正信号观察列表不同")
    if updated_sources != {"snapshot": digest(snapshot / "matrices.npz"),
                           "metadata": digest(snapshot / "matrices-metadata.json"), "index": digest(index_path)}:
        raise ValueError("新输入在计算期间改变，拒绝不一致的账本")
    verify_sources()  # Detect concurrent edits of any trusted old input during calculation.
    content = {"schema": "ashare-model-run-v1", "run_id": str(uuid.uuid4()),
               "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(), "model_id": args.model_id,
               "model_name": spec["name"],
               "score_semantic": spec["score_semantic"], "signal_threshold": threshold,
               "label_holding_days": 20, "holding_days": args.holding_days, "comparison": args.comparison,
               "position_policy": "40/30/30 initial/add, weak reduce/profit protection" if args.comparison == "staged" else "full8pct target",
               "mode": args.mode, "forward_start": anchor if args.mode == "forward" else None,
               "as_of": last_day.isoformat(), "production_admission": False, "exploration": True,
               "initial_cash_cny": 100000, "source_run_id": spec["source_run_id"], "input_sha256": hashes,
               "data_sha256": updated_sources, "model_sha256": hashes[f"{args.model_id}_model2026"],
               "score_cache_sha256": hashes[f"{args.model_id}_score"], "runner_sha256": digest(__file__),
               "frozen_prefix_preserved": True, "stocks": len(codes), "sessions": len(dates),
               "training_refitted": False, "state": "waiting_new_data" if args.mode == "forward" and latest == anchor else "observing" if args.mode == "forward" else "historical_replay",
               "ledger": ledger, "accounting": {k: v for k, v in accounting.items() if k != "unclosed"},
               "signal_watch": watch, "current_scores": scored, "action_coverage": action_coverage,
               "limitations": ["历史回放不是开启账户之前的真实前向交易；2025以后已经反复观察，不称未见样本。",
                    "二十日标签预测分数不是胜率；十五日仅执行期限对照，含3ATR退出、费用与开盘代理。",
                    "缺少完整财报点时版本；公司行动支付/上市日期未知时现金或新增股保守挂起。",
                    "历史ST开盘前可知时点、竞价深度/停牌/退市回收仍为代理，持仓按最后报价估值可能高估回收。",
                    "当前行业成员不回填历史；无分时证据或买入许可；模型未生产准入。"],
               "checks": {"trusted_inputs_sha_verified": True, "ST_BJ_buys": 0,
                          "model_score_prefix_preserved": True, "whole_ledger_pnl_reconciled": True,
                          "current_scores_match_signal_watch": True,
                          "no_AI_script_or_refit": True}}
    text = json.dumps(content, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
    envelope = {"schema": "model-run-bundle-v1", "content": text,
                "content_sha256": hashlib.sha256(text.encode("utf8")).hexdigest()}
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf8", newline="\n") as stream:
        json.dump(envelope, stream, ensure_ascii=False, separators=(",", ":"))
    print(json.dumps({"output": str(output), "state": content["state"], "as_of": content["as_of"],
                      "orders": len(ledger["orders"]), "metrics": ledger["metrics"]}, ensure_ascii=False))


def self_check():
    validate_choice("breadth22_h20", 20, "baseline", "replay")
    validate_choice("index26_h20", 15, "holding15", "forward")
    for choice in [("trend_follow", 20, "baseline", "replay"), ("index26_h20", 15, "baseline", "forward")]:
        try:
            validate_choice(*choice)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid choice accepted")
    text = "测试真实UTF8正文"
    assert hashlib.sha256(text.encode("utf8")).hexdigest() != hashlib.sha256(json.dumps(text).encode()).hexdigest()
    import numpy as np
    rows = current_scores(np, ["000001", "600000", "300001"], "2026-09-30",
                          np.array([-.1, .2, np.nan]), np.array([True, True, True]), 0.)
    assert rows == [{"symbol": "sz000001", "score": -.1, "threshold": 0., "as_of": "2026-09-30"},
                    {"symbol": "sh600000", "score": .2, "threshold": 0., "as_of": "2026-09-30"}]
    print("self-check passed: model identity, fifteen-day execution separation, strict choice, UTF8 SHA")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-id", choices=MODELS, default=MODELS[0])
    parser.add_argument("--holding-days", type=int, choices=(15, 20), default=20)
    parser.add_argument("--comparison", choices=COMPARISONS, default="baseline")
    parser.add_argument("--mode", choices=("replay", "forward"), default="replay")
    parser.add_argument("--snapshot", type=Path, default=OPEN / "stockdb-live/exports")
    parser.add_argument("--index", type=Path, default=OPEN / "online/index-sh-000300.ndjson")
    parser.add_argument("--forward-start", type=int)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if args.self_check:
        self_check()
    elif args.output is None:
        parser.error("--output is required; never overwrites an existing result")
    else:
        emit(args)
