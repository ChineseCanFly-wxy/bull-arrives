"""Fixed-sample, reproducible A-share research for the three V2 hypotheses.

Run: python scripts/historical-mode-study.py --stockdb-dir D:/github项目/stockdb
This is an independent historical check, not an order simulator or success gate.
"""

import argparse
import datetime as dt
import json
import math
import sys
from collections import defaultdict
from pathlib import Path

SYMBOLS = [
    "600519", "600036", "600000", "600030", "600276", "601318",
    "601398", "000001", "000333", "000651", "002415", "300750",
]
COST_PCT = 0.46  # Conservative round trip; includes fees, tax and slippage proxy.
DAILY_SPLIT = 20240101
MINUTE_SPLIT = 20260101


def mean(rows, n, lag=0):
    end = len(rows) - lag
    return sum(row["close"] for row in rows[end - n:end]) / n


def daily_action(rows, long=False):
    if len(rows) < (125 if long else 65):
        return "wait"
    close = rows[-1]["close"]
    ma20, ma60 = mean(rows, 20), mean(rows, 60)
    exit_window = 60 if long else 20
    if close < mean(rows, exit_window) and rows[-2]["close"] < mean(rows, exit_window, 1):
        return "sell"
    aligned = close > ma20 > ma60 and ma20 > mean(rows, 20, 5)
    if long:
        aligned = aligned and ma60 > mean(rows, 120) and ma60 > mean(rows, 60, 5) and close > rows[-61]["close"]
    return "watch_buy" if aligned and close <= ma20 * 1.04 else "wait"


def valid_day(q, raw):
    return q["date"] == raw["date"] and all(
        math.isfinite(row[key]) and row[key] > 0 for row in (q, raw) for key in ("open", "high", "low", "close")
    ) and raw.get("volume", 0) > 0 and not raw.get("is_st", False)


def daily_trades(code, qfq, raw, long=False):
    raw_by_date = {r["date"]: r for r in raw}
    qfq = [r for r in qfq if r["date"] in raw_by_date]
    trades = []
    position = None
    for i in range(125 if long else 65, len(qfq) - 1):
        signal = qfq[i]
        tomorrow = raw_by_date[qfq[i + 1]["date"]]
        if not valid_day(qfq[i + 1], tomorrow) or not valid_day(signal, raw_by_date[signal["date"]]):
            continue
        action = daily_action(qfq[:i + 1], long)
        if position:
            held = i + 1 - position["index"]
            stop = position["entry"] * 0.85  # Risk ceiling; strategy exit remains primary.
            if (action == "sell" or held >= (120 if long else 40) or tomorrow["open"] < stop) and held >= 1:
                pnl = (tomorrow["open"] / position["entry"] - 1) * 100 - COST_PCT
                trades.append({"code": code, "entry_date": position["date"], "exit_date": tomorrow["date"],
                               "pnl_pct": round(pnl, 5), "reason": "trend" if action == "sell" else "risk_or_time"})
                position = None
        elif action == "watch_buy" and tomorrow["open"] <= raw_by_date[signal["date"]]["close"] * 1.04:
            position = {"index": i + 1, "date": tomorrow["date"], "entry": tomorrow["open"]}
    return trades


def minute_action(rows):
    usable = [r for r in rows if r.get("volume", 0) > 0]
    if len(usable) < 32:
        return "wait"
    last, prev = usable[-1], usable[-31:-1]
    vwap = sum(r["amount"] for r in usable) / sum(r["volume"] for r in usable)
    prior_vwap = sum(r["amount"] for r in usable[:-1]) / sum(r["volume"] for r in usable[:-1])
    if last["close"] < vwap and usable[-2]["close"] < prior_vwap:
        return "sell"
    if last["close"] > max(r["high"] for r in prev) and last["close"] > vwap and last["volume"] > 1.5 * sum(r["volume"] for r in prev) / 30:
        return "watch_buy"
    return "wait"


def intraday_trades(code, daily_q, daily_raw, minutes):
    raw_by_date = {r["date"]: r for r in daily_raw}
    daily_q = [r for r in daily_q if r["date"] in raw_by_date]
    day_index = {row["date"]: i for i, row in enumerate(daily_q)}
    grouped = defaultdict(list)
    for row in minutes:
        grouped[row["date"] // 1000000].append(row)
    trades, position = [], None
    for day in sorted(grouped):
        i = day_index.get(day)
        if i is None or i < 65 or not valid_day(daily_q[i], raw_by_date[day]):
            continue
        # At this session only the preceding completed daily bar is known.
        direction = daily_action(daily_q[:i]) == "watch_buy"
        bars = sorted(grouped[day], key=lambda r: r["date"])
        if len(bars) < 35:
            continue
        for j in range(32, len(bars) - 1):
            signal, fill = bars[j], bars[j + 1]
            if fill.get("volume", 0) <= 0 or fill["open"] <= 0:
                continue
            action = minute_action(bars[:j + 1])
            if position:
                held_days = (dt.datetime.strptime(str(day), "%Y%m%d") - dt.datetime.strptime(str(position["date"]), "%Y%m%d")).days
                if day > position["date"] and (action == "sell" or held_days >= 15 or fill["open"] < position["entry"] * 0.85):
                    trades.append({"code": code, "entry_date": position["date"], "exit_date": day,
                                   "pnl_pct": round((fill["open"] / position["entry"] - 1) * 100 - COST_PCT, 5),
                                   "reason": "vwap" if action == "sell" else "risk_or_time"})
                    position = None
                    break
            elif direction and action == "watch_buy":
                position = {"date": day, "entry": fill["open"]}
                break
    return trades


def metrics(trades):
    values = [t["pnl_pct"] for t in sorted(trades, key=lambda t: (t["exit_date"], t["code"]))]
    if not values:
        return {"trades": 0, "win_rate_pct": None, "mean_net_pct": None, "median_net_pct": None, "worst_trade_pct": None, "portfolio_max_drawdown_pct": None}
    ordered = sorted(values)
    return {"trades": len(values), "win_rate_pct": round(100 * sum(v > 0 for v in values) / len(values), 2),
            "mean_net_pct": round(sum(values) / len(values), 3),
            "median_net_pct": round(ordered[len(values) // 2], 3),
            "worst_trade_pct": round(ordered[0], 3),
            "portfolio_max_drawdown_pct": None}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--stockdb-dir", default="D:/github项目/stockdb")
    parser.add_argument("--output", default="docs/历史策略检验-2026-09-26.json")
    args = parser.parse_args()
    sys.path.insert(0, str(Path(args.stockdb_dir) / "pybao"))
    from stock_sdk import init, rd
    init("127.0.0.1", 7899)
    all_trades = {"swing_v2": [], "intraday_v2": [], "long_term_v2": []}
    coverage = {}
    for code in SYMBOLS:
        q = rd.get_data(code, start="20200101", end="20260926", frequency="1d", fq="qfq")
        raw = rd.get_data(code, start="20200101", end="20260926", frequency="1d", fq=None)
        minute = rd.get_data(code, start="20250101", end="20260926", frequency="1m", fq=None)
        coverage[code] = {"daily": len(q), "minute": len(minute),
                          "first_daily": q[0]["date"] if q else None, "last_daily": q[-1]["date"] if q else None,
                          "last_minute": minute[-1]["date"] if minute else None}
        all_trades["swing_v2"].extend(daily_trades(code, q, raw))
        all_trades["long_term_v2"].extend(daily_trades(code, q, raw, long=True))
        all_trades["intraday_v2"].extend(intraday_trades(code, q, raw, minute))
        print(code, coverage[code], flush=True)
    result = {"symbols": SYMBOLS, "coverage": coverage, "cost_round_trip_pct": COST_PCT,
              "splits": {"daily_oos_from": DAILY_SPLIT, "minute_oos_from": MINUTE_SPLIT},
              "portfolio_drawdown_note": "未构造资金、仓位、并发订单和公司行动账本；组合最大回撤不可得，禁止把逐笔复利当组合收益", "modes": {}}
    for mode, trades in all_trades.items():
        split = MINUTE_SPLIT if mode == "intraday_v2" else DAILY_SPLIT
        result["modes"][mode] = {"all": metrics(trades), "train": metrics([t for t in trades if t["entry_date"] < split]),
                                  "oos": metrics([t for t in trades if t["entry_date"] >= split]),
                                  "exits": {reason: sum(t["reason"] == reason for t in trades) for reason in ("trend", "vwap", "risk_or_time")}}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(result["modes"], ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
