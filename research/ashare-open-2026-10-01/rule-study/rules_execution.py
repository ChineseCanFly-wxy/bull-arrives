"""Isolated historical ST-limit rule fork; audited engine and model scores unchanged.

Only tradable_open is temporarily swapped by this runner, then restored. The
original all-period 5% mainboard-ST assumption remains a disclosed proxy.
"""
import argparse
import hashlib
import json
import math
import sys
import time
from collections import Counter
from pathlib import Path

import numpy as np

ROOT = Path(__file__).parent
OPEN = ROOT.parent
OLD = OPEN.parent / "ashare-2026-10-01"
sys.path.insert(0, str(OPEN))
import execution as e
from execution_study import action_map
s = e.s
ORIGINAL_OPEN = e.tradable_open
SSE_EFFECTIVE = 20260706
SZ_EFFECTIVE = None
GATE_CHANGES = []
ANCHORS = {
    "relative20tenfull": {"score": "learning/results/hist_tree_h20_scores.npy", "expansion": True, "full": True},
    "absolute20tenfull": {"score": "learning/absolute-results/hist_tree_h20_scores.npy", "expansion": False, "full": True},
    "relative20tenlayers": {"score": "learning/results/hist_tree_h20_scores.npy", "expansion": True, "full": False}}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def st_limit(code, day):
    if code.startswith(("600", "601", "603", "605")) and day >= SSE_EFFECTIVE:
        return .1
    if SZ_EFFECTIVE is not None and code.startswith(("000", "001", "002", "003")) and day >= SZ_EFFECTIVE:
        return .1
    return .05


def tradable_open(raw, valid, i, c, code, sell, factors, multiplier=1, events=None):
    """Same open-only proxy, changing only the effective mainboard ST limit."""
    if i == 0 or not valid[i - 1, c]:
        return False
    price = float(raw["open"][i, c])
    st = float(raw["is_st"][i, c])
    if not math.isfinite(price) or price <= 0 or not math.isfinite(st):
        return False
    if st != 0 and not sell:
        return False
    ref = float(raw["close"][i - 1, c]) * factors[i - 1, c] / factors[i, c]
    event = (events or {}).get((i, c))
    if event:
        detail = e.ACTION_DETAILS.get((i, c), {})
        div = detail.get("reference_div", s.number(event.get("div", 0)))
        trans = detail.get("reference_trans", s.number(event.get("trans", 0)))
        ref = (float(raw["close"][i - 1, c]) - div) / (1 + s.number(event.get("give", 0)) + trans)
    day = int(e.DATES[i])
    pct = s.limit_fraction(code, day)
    if st != 0 and not code.startswith(("300", "301", "688", "689")):
        pct = st_limit(code, day)
    boundary = round(ref * (1 - pct if sell else 1 + pct) + 1e-8, 2)
    slipped = price * (1 - .001 * multiplier if sell else 1 + .001 * multiplier)
    new = min(price, slipped) > boundary + .005 if sell else max(price, slipped) < boundary - .005
    old = ORIGINAL_OPEN(raw, valid, i, c, code, sell, factors, multiplier, events)
    if old != new:
        GATE_CHANGES.append({"date": day, "code": code, "side": "sell" if sell else "buy",
                             "raw_open": price, "reference_price": ref, "is_st": st,
                             "old_proxy_allowed": bool(old), "new_rule_allowed": bool(new),
                             "new_fraction": pct, "new_boundary": boundary, "slipped_price": slipped})
    return new


def self_check():
    old_dates = getattr(e, "DATES", None)
    old_actions = getattr(e, "ACTION_DETAILS", None)
    saved_changes = list(GATE_CHANGES)
    raw = {"open": np.full((4, 1), 9.3), "close": np.full((4, 1), 10.), "is_st": np.ones((4, 1))}
    valid, factors = np.ones((4, 1), bool), np.ones((4, 1))
    e.DATES = np.array([20260702, 20260703, 20260706, 20260707])
    e.ACTION_DETAILS = {}
    try:
        assert st_limit("600001", 20260703) == .05
        assert st_limit("600001", 20260706) == .1
        assert not tradable_open(raw, valid, 1, 0, "600001", True, factors)
        assert tradable_open(raw, valid, 2, 0, "600001", True, factors)
        assert not ORIGINAL_OPEN(raw, valid, 2, 0, "600001", True, factors)
        assert not tradable_open(raw, valid, 2, 0, "600001", False, factors)
        if SZ_EFFECTIVE is None:
            assert st_limit("000001", 20260706) == .05
            assert not tradable_open(raw, valid, 2, 0, "000001", True, factors)
        else:
            assert st_limit("000001", SZ_EFFECTIVE - 1) == .05
            assert st_limit("000001", SZ_EFFECTIVE) == .1
            assert not tradable_open(raw, valid, 1, 0, "000001", True, factors)
            assert tradable_open(raw, valid, 2, 0, "000001", True, factors)
            assert not tradable_open(raw, valid, 2, 0, "000001", False, factors)
        for prefix in ("600", "601", "603", "605"):
            assert st_limit(prefix + "001", 20260703) == .05
            assert st_limit(prefix + "001", 20260706) == .1
        if SZ_EFFECTIVE is not None:
            for prefix in ("000", "001", "002", "003"):
                assert st_limit(prefix + "001", SZ_EFFECTIVE - 1) == .05
                assert st_limit(prefix + "001", SZ_EFFECTIVE) == .1
        raw["open"][2, 0] = 9.
        assert not tradable_open(raw, valid, 2, 0, "600001", True, factors), "10% down limit remains blocked"
        raw["open"][2, 0] = 9.3
        assert tradable_open(raw, valid, 2, 0, "688001", True, factors), "STAR 20% unchanged"
        raw["is_st"][:] = 0
        assert tradable_open(raw, valid, 2, 0, "600001", True, factors)
        assert not any(row["side"] == "buy" for row in GATE_CHANGES)
    finally:
        e.DATES, e.ACTION_DETAILS = old_dates, old_actions
        GATE_CHANGES[:] = saved_changes
    print("rules self-check passed: SSE effective-day boundary, old5 vs new10, still no ST buy, down-limit blocked", flush=True)


def accounting(ledger):
    net = ledger["curve"][-1]["equity"] - 100000
    completed = sum(row["net_pnl"] for row in ledger["cycles"])
    unclosed = sum(row["unrealized_and_receivable_pnl"] for row in ledger["unclosed"])
    return {"net_change_cny": round(net, 4), "completed_pnl_cny": round(completed, 3),
            "unclosed_pnl_cny": round(unclosed, 3), "reconcile_cny": round(completed + unclosed - net, 5),
            "minimum_cash_cny": min(row["cash"] for row in ledger["curve"]),
            "unclosed": ledger["unclosed"]}


def trade_difference(old, new):
    def encoded(orders):
        return Counter(json.dumps(row, sort_keys=True, ensure_ascii=False) for row in orders)
    old_counts, new_counts = encoded(old["orders"]), encoded(new["orders"])
    removed = [json.loads(row) for row, count in (old_counts - new_counts).items() for _ in range(count)]
    added = [json.loads(row) for row, count in (new_counts - old_counts).items() for _ in range(count)]
    first = None
    for index, (before, after) in enumerate(zip(old["orders"], new["orders"])):
        if before != after:
            first = {"order_index": index, "old": before, "new": after}
            break
    if first is None and len(old["orders"]) != len(new["orders"]):
        at = min(len(old["orders"]), len(new["orders"]))
        first = {"order_index": at, "old": old["orders"][at:at + 1], "new": new["orders"][at:at + 1]}
    return {"old_order_count": len(old["orders"]), "new_order_count": len(new["orders"]),
            "first_order_difference": first, "removed_old_orders": removed, "added_new_orders": added,
            "metrics_delta": {key: round(float(new["metrics"][key]) - float(old["metrics"][key]), 6)
                              for key in ("net_return_pct", "max_drawdown_pct", "blocked_exit_sessions", "completed_holding_cycles", "max_overdue_sessions")},
            "curve_changed_sessions": sum(before != after for before, after in zip(old["curve"], new["curve"]))}


def report(summary, output):
    sz_scope = ("深交所000/001/002/003主板ST退出也已依据官方2026修订规则、修订说明及发布通知核实，2026-07-06起按10%，此前按5%。"
                if summary["sz_effective_date"] == 20260706 else
                "深交所日期未确认时保留5%待核，不能把保守代理写成当期真实法律。")
    lines = ["# ST退出规则日期核对：独立执行对照", "",
             "只在本runner中暂换共享执行器的tradable_open，完成后恢复；共享源码和评分不修改。已确认上交所600/601/603/605主板ST退出边界在2026-07-06起按10%，此前按5%。" + sz_scope + "所有时期继续禁止买入ST。", "",
             "本轮只比较2025至数据尾部2026-09-24的既有三个20日模型锚点与同执行的全eligible hash、同正分池hash。相对模型保留广度扩张门禁，模型内池hash使用同一门禁；全eligible hash不受该门禁或评分控制。所有组合十槽位、8%目标，整仓或分层跟随锚点，time退出仍含3ATR失效保护。", "",
             "|锚点|对照|成本|旧5%代理收益|日期规则收益|差值百分点|旧回撤|日期规则回撤|具体交易变化条数|", "|---|---|---|---:|---:|---:|---:|---:|---:|"]
    for anchor, data in summary["anchors"].items():
        for candidate, item in data.items():
            for cost, results in item.items():
                old, new, diff = results["old_proxy"]["metrics"], results["new_rules"]["metrics"], results["difference"]
                lines.append(f"|{anchor}|{candidate}|{cost}|{old['net_return_pct']}%|{new['net_return_pct']}%|{diff['metrics_delta']['net_return_pct']}|{old['max_drawdown_pct']}%|{new['max_drawdown_pct']}%|{len(diff['removed_old_orders'])+len(diff['added_new_orders'])}|")
    lines += ["", "合成检查确认沪深主板代码前缀生效日期前后边界、仍不买ST、10%跌停价和滑点边界仍阻断卖出、科创板20%不受改变。具体开盘门禁差异、第一笔交易差异、旧订单移除和新订单添加均写入JSON；没有交易变化时，只能说这些组合在这段样本未触发规则差异，不能说旧代理等同真实制度。", "",
              "规则依据已读上交所2026修订正文3.3.13、风险警示章节、11.9生效条文及暂缓条文，保存官方文档/提取文本/来源SHA。风险警示50万股累计买入条款不影响此次始终不买ST的模拟，不能拿累计买入限制冒充价格涨跌幅限制。", "",
              "深交所依据：2026修订规则3.3.13、10.9，以及深证上〔2026〕551号发布通知和风险警示板2026指南。官方修订说明确认将主板风险警示股票涨跌幅限制比例由5%调整为10%。已有上交所单独对照文件留在父目录，本次沪深规则对照另存子目录。", "",
              "规则变化仅覆盖ST持有退出。盘后ST标签在开盘前可知时点、真实参考价格、退市整理首日及例外、开盘深度、暂停交易和股息/配股/上市信息仍待认证。本轮不是完整法律规则引擎，也不能以保守5%造成低收益就声称所有误差都只会降低模拟收益；现金释放会改变后续交易路径。"]
    (output / "RESULTS.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--sz-effective-date", type=int)
    parser.add_argument("--sz-source", type=Path)
    parser.add_argument("--output-subdir", type=str)
    args = parser.parse_args()
    global SZ_EFFECTIVE
    if args.sz_effective_date is not None:
        if args.sz_source is None or not args.sz_source.is_file():
            raise ValueError("Verified local SZ official source required for a SZ effective date")
        SZ_EFFECTIVE = args.sz_effective_date
    self_check()
    if args.self_check:
        return
    began = time.time()
    ROOT.mkdir(exist_ok=True)
    output = ROOT if args.output_subdir is None else (ROOT / args.output_subdir).resolve()
    if output != ROOT.resolve() and output.parent != ROOT.resolve():
        raise ValueError("Output subdirectory must be directly within isolated rule-study")
    output.mkdir(exist_ok=True)
    inputs = {"runner": Path(__file__), "execution": Path(e.__file__), "action_map": OPEN / "execution_study.py", "study": Path(s.__file__),
              "matrix": OLD / "exports/matrices.npz", "matrix_metadata": OLD / "exports/matrices-metadata.json",
              "action_overlay": OPEN / "online/action-overlay.json"}
    for name, spec in ANCHORS.items():
        inputs[name + "_score"] = OPEN / spec["score"]
    for path in (OPEN / "online/rules").glob("*"):
        if path.is_file():
            inputs["official_rules_" + path.name] = path
    if args.sz_source:
        inputs["verified_sz_source"] = args.sz_source
    hashes = {key: digest(path) for key, path in inputs.items()}
    s.save(output / "prereg.json", {"created": "2026-10-01", "version": "isolated_st_rule_execution_v1.1",
           "sse_effective_date": SSE_EFFECTIVE, "sz_effective_date": SZ_EFFECTIVE,
           "rules": "SSEmainboardSTexit10%from20260706;previous5%;SZ10%from explicit verified date, else5%pending;STneverbuys",
           "period": s.PERIODS["test"], "anchors": ANCHORS,
           "baselines": "all eligible hash and same positive pool hash, same slots/sizing/actions/exit",
           "costs": [1, 2], "execution_runs": 36, "input_sha256": hashes,
           "limits": "isolated rule date sensitivity, open proxy, historical ST PIT and exceptions not certified; no new winner selection"})
    with np.load(OLD / "exports/matrices.npz", allow_pickle=False) as cache:
        codes, dates = cache["codes"].tolist(), cache["dates"]
        raw = {key.removeprefix("raw_"): cache[key] for key in cache.files if key.startswith("raw_")}
        seen, valid, factors = cache["seen"], cache["valid"], cache["factors"]
    metadata = json.loads((OLD / "exports/matrices-metadata.json").read_text(encoding="utf-8"))
    events = {(int(row["i"]), int(row["c"])): row["event"] for row in metadata["events"]}
    actions, coverage = action_map(codes, dates, events)
    f = s.features(raw, valid, factors, seen)
    summary = {"version": "isolated_st_rule_execution_v1.1", "sse_effective_date": SSE_EFFECTIVE,
               "sz_effective_date": SZ_EFFECTIVE, "input_sha256_start": hashes,
               "action_coverage": coverage, "anchors": {}, "production_admission": False}
    accounting_rows = []
    try:
        for anchor, spec in ANCHORS.items():
            score = np.load(OPEN / spec["score"])
            positive = f["eligible"] & np.isfinite(score) & (score > 0)
            if spec["expansion"]:
                positive &= (f["breadth"][:, None] >= .45) & ((f["breadth"] - s.lag(f["breadth"], 5))[:, None] >= 0)
            summary["anchors"][anchor] = {}
            for candidate, mask in (("model", positive), ("all_eligible_hash", f["eligible"]), ("same_positive_hash", positive)):
                engine_name = anchor if candidate == "model" else "baseline_hash"
                f["signals"][engine_name], f["score"] = mask, score
                summary["anchors"][anchor][candidate] = {}
                for cost, multiplier in (("base_cost", 1), ("double_cost", 2)):
                    ledgers = {}
                    outcomes = {}
                    for rule, function in (("old_proxy", ORIGINAL_OPEN), ("new_rules", tradable_open)):
                        e.tradable_open = function
                        GATE_CHANGES.clear()
                        ledger = e.backtest(engine_name, *s.PERIODS["test"], codes, dates, raw, valid, factors, events, f,
                                            hold=20, max_positions=10, full_entry=spec["full"], exit_policy="time",
                                            action_details=actions, multiplier=multiplier, journal=True)
                        ledgers[rule] = ledger
                        record = accounting(ledger)
                        accounting_rows.append(record)
                        outcomes[rule] = {"metrics": ledger["metrics"], "accounting": record,
                                          "gate_changes_relative_old_proxy": list(GATE_CHANGES)}
                        s.save(output / f"{anchor}-{candidate}-{cost}-{rule}-ledger.json", ledger)
                    outcomes["difference"] = trade_difference(ledgers["old_proxy"], ledgers["new_rules"])
                    summary["anchors"][anchor][candidate][cost] = outcomes
                    print("rules", anchor, candidate, cost,
                          ledgers["old_proxy"]["metrics"]["net_return_pct"], ledgers["new_rules"]["metrics"]["net_return_pct"],
                          "gatechanges", len(outcomes["new_rules"]["gate_changes_relative_old_proxy"]), flush=True)
                    s.save(output / "progress.json", summary)
    finally:
        e.tradable_open = ORIGINAL_OPEN
    summary["open_function_restored"] = e.tradable_open is ORIGINAL_OPEN
    summary["input_sha256_end"] = {key: digest(path) for key, path in inputs.items()}
    summary["inputs_unchanged"] = summary["input_sha256_end"] == hashes
    summary["max_reconcile_cny"] = max(abs(row["reconcile_cny"]) for row in accounting_rows)
    summary["minimum_cash_cny"] = min(row["minimum_cash_cny"] for row in accounting_rows)
    summary["elapsed_seconds"] = round(time.time() - began, 2)
    assert summary["inputs_unchanged"] and summary["open_function_restored"] and summary["minimum_cash_cny"] >= 0 and summary["max_reconcile_cny"] < .1
    s.save(output / "summary.json", summary)
    report(summary, output)
    print("rules finished", summary["elapsed_seconds"], "sourceunchanged", summary["inputs_unchanged"], flush=True)


if __name__ == "__main__":
    main()
