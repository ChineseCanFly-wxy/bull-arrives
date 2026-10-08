"""Offline A-share study. Read-only input; research exports, never brokerage orders.

Run with the bundled Python: study.py --data <export directory> --output <directory>.
All decisions use completed sessions; fills use the next session's unadjusted open.
"""
import argparse
import datetime as dt
import gzip
import hashlib
import json
import math
import re
from collections import defaultdict
from pathlib import Path

import numpy as np
import pandas as pd

STOCK = re.compile(r"^(000|001|002|003|300|301|600|601|603|605|688|689)\d{3}$")
FIELDS = ["open", "high", "low", "close", "volume", "amount", "is_st", "pe_ttm", "pb", "pct_chg"]
PERIODS = {"train": (20200101, 20221231), "validation": (20230101, 20241231), "test": (20250101, 20261001)}


def records(path):
    opener = gzip.open if str(path).endswith(".gz") else open
    with opener(path, "rt", encoding="utf-8") as f:
        for line in f:
            if line.strip():
                yield json.loads(line)


def number(value):
    if isinstance(value, bool):
        return float(value)
    try:
        return float(value) if value is not None else math.nan
    except (ValueError, TypeError):
        return math.nan


def load(data):
    daily_path = data / "daily.ndjson.gz"
    codes, dates, count = set(), set(), 0
    for r in records(daily_path):
        code, date = str(r["code"]), int(r["date"])
        if STOCK.fullmatch(code) and date >= 20180101:
            codes.add(code)
            dates.add(date)
            count += 1
    codes, dates = sorted(codes), np.array(sorted(dates), dtype=np.int32)
    ci, di = {c: i for i, c in enumerate(codes)}, {int(d): i for i, d in enumerate(dates)}
    shape = (len(dates), len(codes))
    raw = {f: np.full(shape, np.nan, dtype=np.float32) for f in FIELDS}
    seen = np.zeros(shape, dtype=bool)
    for r in records(daily_path):
        c, d = ci.get(str(r["code"])), di.get(int(r["date"]))
        if c is None or d is None:
            continue
        if seen[d, c]:
            raise ValueError(f"duplicate logical daily key {r['code']}:{r['date']}")
        seen[d, c] = True
        for f in FIELDS:
            raw[f][d, c] = number(r.get(f))
    factors = np.ones(shape, dtype=np.float64)
    events, factor_rows = {}, defaultdict(list)
    for r in records(data / "factors.ndjson.gz"):
        code, day = str(r["code"]), int(r["date"])
        c = ci.get(code)
        if c is not None:
            factor_rows[c].append((day, r))
    for c in range(len(codes)):
        rows = sorted(factor_rows[c], key=lambda x: x[0])
        for day, r in rows:
            at = int(np.searchsorted(dates, day))
            cum = number(r.get("cum"))
            if not (math.isfinite(cum) and cum > 0):
                raise ValueError(f"invalid adjustment factor {codes[c]}:{day}")
            factors[at:, c] = cum
            if day >= int(dates[0]):
                if (at, c) in events:
                    raise ValueError(f"duplicate factor session {codes[c]}:{day}")
                events[(at, c)] = r
    prices = (raw["open"] > 0) & (raw["low"] > 0) & (raw["close"] > 0)
    prices &= (raw["high"] + 0.011 >= np.maximum(raw["open"], raw["close"]))
    prices &= (raw["low"] - 0.011 <= np.minimum(raw["open"], raw["close"]))
    valid = seen & prices & (raw["volume"] > 0) & (raw["amount"] > 0)
    damaged = np.zeros(shape, dtype=bool)
    damage_path = data / "invalid_records.ndjson.gz"
    if damage_path.exists():
        for r in records(damage_path):
            parts = str(r.get("key", "")).removeprefix("k").split(":")
            code = r.get("code") or (parts[1] if len(parts) > 2 else None)
            day = r.get("date") or (parts[2] if len(parts) > 2 else None)
            c = ci.get(str(code))
            if c is not None and day and str(day).isdigit():
                at = int(np.searchsorted(dates, int(day)))
                damaged[at:at+61, c] = True
    action_mismatches = []
    for (at, c), r in events.items():
        if 0 < at < len(dates) and valid[at - 1, c]:
            div, give, trans = [number(r.get(k, 0)) for k in ("div", "give", "trans")]
            previous = float(raw["close"][at - 1, c])
            implied = (1 + give + trans) * previous / (previous - div) if previous > div else math.nan
            observed = factors[at, c] / factors[at - 1, c]
            if not math.isfinite(implied) or abs(observed / implied - 1) > 0.003:
                action_mismatches.append({"code": codes[c], "date": int(dates[at]), "factor_ratio": observed,
                                          "event_implied_ratio": implied if math.isfinite(implied) else None})
    quality = {"daily_rows": count, "stocks": len(codes), "sessions": len(dates),
               "first_date": int(dates[0]), "last_date": int(dates[-1]),
               "invalid_price_or_volume_rows": int((seen & ~valid).sum()),
               "missing_st_rows": int((seen & ~np.isfinite(raw["is_st"])).sum()),
               "st_rows": int((seen & (raw["is_st"] != 0)).sum()),
               "factor_events": len(events), "corruption_embargo_stock_sessions": int(damaged.sum()),
               "company_action_formula_mismatches": len(action_mismatches), "company_action_mismatch_examples": action_mismatches[:20],
               "minimum_valid_stock_count_per_session": int(valid.sum(axis=1).min()),
               "stopped_quotes_before_dataset_end_stocks": int((seen[-1] == 0).sum())}
    raw["damaged"] = damaged
    return codes, dates, raw, seen, valid, factors, events, quality


def roll(a, n, method="mean"):
    return getattr(pd.DataFrame(a).rolling(n, min_periods=n), method)().to_numpy(dtype=np.float32)


def lag(a, n=1):
    out = np.full_like(a, np.nan)
    out[n:] = a[:-n]
    return out


def features(raw, valid, factors, seen):
    q = {}
    for f in ("open", "high", "low", "close"):
        q[f] = pd.DataFrame(np.where(valid, raw[f] * factors, np.nan)).ffill().to_numpy(dtype=np.float32)
    close, high, low = q["close"], q["high"], q["low"]
    ma10, ma20, ma60 = roll(close, 10), roll(close, 20), roll(close, 60)
    atr = roll(np.maximum(high - low, np.maximum(abs(high - lag(close)), abs(low - lag(close)))), 14)
    r1, r3, r20 = close / lag(close) - 1, close / lag(close, 3) - 1, close / lag(close, 20) - 1
    location = np.divide(close - low, high - low, out=np.full_like(close, 0.5), where=high > low)
    volume = np.where(valid, raw["volume"], 0)
    v20, a20 = roll(volume, 20), roll(np.where(valid, raw["amount"], 0), 20)
    prior_high = lag(roll(high, 20, "max"))
    prior_low = lag(roll(low, 20, "min"))
    first = np.argmax(seen, axis=0)
    age = np.arange(len(close))[:, None] - first[None, :]
    eligible = valid & (raw["is_st"] == 0) & (age >= 120) & (a20 >= 30000000) & (raw["close"] >= 3)
    eligible &= ~raw.get("damaged", np.zeros_like(valid))
    denominator = eligible.sum(axis=1)
    breadth = np.divide((eligible & (close > ma20)).sum(axis=1), denominator,
                        out=np.zeros(len(close)), where=denominator > 0)
    trend = (close > ma60) & (ma20 > ma60) & (ma20 > lag(ma20, 5))
    market = breadth[:, None] >= 0.4
    pullback = trend & (low <= ma10 * 1.015) & (close >= ma10) & (close <= ma20 * 1.06)
    pullback &= (r1 > 0) & (location >= 0.6) & (volume <= lag(v20) * 1.2)
    breakout = (close > prior_high) & (volume >= lag(v20) * 1.5) & (location >= 0.7)
    breakout &= (close > ma20) & (close <= ma20 * 1.15) & (r1 < 0.085)
    compression = (prior_high / prior_low < 1.12) & (close > prior_high)
    compression &= (volume >= lag(v20) * 1.2) & (location >= 0.65) & (close > ma60)
    repair = (close > ma60) & (ma60 > lag(ma60, 10)) & (r3 < -0.04)
    repair &= (r20 > 0) & (location > 0.55) & (close > lag(close))
    # An acute 3-day fall and a positive last day is an intentional, fixed hypothesis.
    signals = {k: v & eligible & market for k, v in {
        "trend_pullback": pullback, "volume_breakout": breakout,
        "compression_breakout": compression, "trend_repair": repair}.items()}
    fundamental = (raw["pe_ttm"] > 0) & (raw["pe_ttm"] <= 60) & (raw["pb"] >= 0.5) & (raw["pb"] <= 8)
    for name, mask in list(signals.items()):
        signals[name + "_valuation"] = mask & fundamental
    signals["baseline_hash"] = eligible & market
    signals["baseline_momentum20"] = eligible & market & (r20 > 0)
    score = r20 / np.maximum(atr / close, 0.005)
    return {**q, "ma10": ma10, "ma20": ma20, "ma60": ma60, "atr": atr,
            "r20": r20, "location": location, "breadth": breadth,
            "eligible": eligible, "signals": signals, "score": score}


def lot_buy(code, budget, price):
    n = math.floor(max(0, budget) / price)
    return (n if n >= 200 else 0) if code.startswith(("688", "689")) else n // 100 * 100


def lot_sell(code, held, fraction):
    if fraction >= 1:
        return held
    n = math.floor(held * fraction)
    if code.startswith(("688", "689")):
        return n if n >= 200 else 0
    return n // 100 * 100


def fee(value, sell, date, multiplier=1):
    commission = max(5.0, value * 0.0003)
    transfer = value * (0.00002 if date < 20220429 else 0.00001)
    stamp = value * (0.001 if date < 20230828 else 0.0005) if sell else 0
    return (commission + transfer + stamp) * multiplier


def limit_fraction(code, date):
    return 0.2 if code.startswith(("688", "689")) or (code.startswith(("300", "301")) and date >= 20200824) else 0.1


def tradable_open(raw, valid, i, c, code, sell, factors, multiplier=1, events=None):
    if not valid[i, c]:
        return False
    # Only adjacent raw closes in the same scale; vendor pre_close is mixed-scale.
    if i == 0 or not valid[i - 1, c]:
        return False
    ref = float(raw["close"][i - 1, c]) * factors[i - 1, c] / factors[i, c]
    event = (events or {}).get((i, c))
    if event:
        ref = (float(raw["close"][i - 1, c]) - number(event.get("div", 0))) / (1 + number(event.get("give", 0)) + number(event.get("trans", 0)))
    pct = limit_fraction(code, int(DATES[i]))
    if not np.isfinite(raw["is_st"][i, c]):
        return False
    if raw["is_st"][i, c] != 0:
        if not sell:
            return False
        # Use the more restrictive legacy 5% for main-board ST exits throughout.
        # Unknown status-transition rule details cannot justify an optimistic fill.
        if not code.startswith(("300", "301", "688", "689")):
            pct = 0.05
    boundary = round(ref * (1 - pct if sell else 1 + pct) + 1e-8, 2)
    price = float(raw["open"][i, c])
    # Conservatively reject opens at the relevant limit, including later unlocks.
    slipped = price * (1 - 0.001 * multiplier if sell else 1 + 0.001 * multiplier)
    return min(price, slipped) > boundary + 0.005 if sell else max(price, slipped) < boundary - 0.005


def backtest(name, start, end, codes, dates, raw, valid, factors, events, f,
             multiplier=1, full_entry=False, hold=15, journal=False, exit_policy="legacy"):
    global DATES
    DATES = dates
    indices = np.where((dates >= start) & (dates <= end))[0]
    cash, receivable, positions, curve, completed, orders = 100000.0, 0.0, {}, [], [], []
    stats = {"blocked_exit_sessions": 0, "blocked_entries": 0, "corporate_events": 0,
             "bonus_share_events_without_listing_date": 0, "company_action_formula_mismatches_held": 0,
             "st_in_position": 0, "suspended_marks": 0}
    signal, score = f["signals"][name], f["score"]
    code_hash = np.array([int.from_bytes(hashlib.sha256(c.encode()).digest()[:4], "little") for c in codes], dtype=np.uint64)
    prev_equity = 100000.0

    def buy(c, qty, i, reason, p=None):
        nonlocal cash
        price = float(raw["open"][i, c]) * (1 + 0.001 * multiplier)
        value, charges = qty * price, fee(qty * price, False, int(dates[i]), multiplier)
        if qty <= 0 or value + charges > cash + 1e-6:
            return p
        cash -= value + charges
        if p is None:
            p = {"code": codes[c], "entry_i": i, "entry_date": int(dates[i]), "qty": 0,
                 "in": 0.0, "out": 0.0, "dividends": 0.0, "stage": 0, "reduced": False,
                 "last": float(raw["open"][i, c]), "peak": float(f["close"][i - 1, c]),
                 "q_entry": price * factors[i, c], "actions": [], "overdue": 0,
                 "last_quote_i": i, "bonus_locked": 0,
                 "atr_entry": float(f["atr"][i - 1, c]), "support": float(f["low"][i - 1, c]) if "low" in f else 0}
            positions[c] = p
        p["qty"] += qty
        p["in"] += value + charges
        p["stage"] += 1
        item = {"date": int(dates[i]), "code": codes[c], "side": "buy", "qty": qty,
                "price": round(price, 4), "fee": round(charges, 3), "reason": reason,
                "holding_session": i - p["entry_i"] + 1}
        p["actions"].append(item)
        if journal:
            orders.append(item)
        return p

    def sell(c, fraction, i, reason):
        nonlocal cash
        p = positions[c]
        available = p["qty"] - p["bonus_locked"]
        qty = lot_sell(codes[c], available, fraction)
        if qty == 0:
            return
        price = float(raw["open"][i, c]) * (1 - 0.001 * multiplier)
        value, charges = qty * price, fee(qty * price, True, int(dates[i]), multiplier)
        cash += value - charges
        p["out"] += value - charges
        p["qty"] -= qty
        item = {"date": int(dates[i]), "code": codes[c], "side": "sell", "qty": qty,
                "price": round(price, 4), "fee": round(charges, 3), "reason": reason,
                "holding_session": i - p["entry_i"] + 1}
        p["actions"].append(item)
        if journal:
            orders.append(item)
        if p["qty"] == 0:
            pnl = p["out"] + p["dividends"] - p["in"]
            completed.append({"code": p["code"], "entry_date": p["entry_date"], "exit_date": int(dates[i]),
                              "holding_sessions": i - p["entry_i"] + 1, "net_pnl": round(pnl, 3),
                              "net_pct": round(pnl / p["in"] * 100, 5), "overdue": p["overdue"],
                              "exit_reason": reason, "actions": p["actions"]})
            del positions[c]
        else:
            p["reduced"] = True

    for i in indices:
        i = int(i)
        day = int(dates[i])
        if i == 0:
            continue
        for c, p in list(positions.items()):
            event = events.get((i, c))
            if event:
                stats["corporate_events"] += 1
                # Units are per-share ratios/cash and must be cross-checked externally.
                div, give, trans = [number(event.get(k, 0)) for k in ("div", "give", "trans")]
                if not all(math.isfinite(x) and x >= 0 for x in (div, give, trans)):
                    raise ValueError(f"unsupported company action {codes[c]}:{day}")
                previous = float(raw["close"][i - 1, c])
                implied = (1 + give + trans) * previous / (previous - div) if previous > div else math.nan
                if not math.isfinite(implied) or abs((factors[i, c] / factors[i - 1, c]) / implied - 1) > 0.003:
                    stats["company_action_formula_mismatches_held"] += 1
                cash_div = p["qty"] * div * 0.8
                # ponytail: payment date unknown; dividend stays receivable and cannot fund buys.
                receivable += cash_div
                p["dividends"] += cash_div
                new_qty = int(math.floor(p["qty"] * (1 + give + trans) + 1e-6))
                p["bonus_locked"] += new_qty - p["qty"]
                if new_qty > p["qty"]:
                    stats["bonus_share_events_without_listing_date"] += 1
                p["qty"] = new_qty
                p["last"] = (p["last"] - div) / (1 + give + trans)
            if i - p["entry_i"] + 1 >= hold:
                p["overdue"] = max(0, i - p["entry_i"] + 1 - hold)
            if raw["is_st"][i - 1, c] != 0:
                stats["st_in_position"] += 1
            qclose, ma10, ma20, atr = [float(f[k][i - 1, c]) for k in ("close", "ma10", "ma20", "atr")]
            p["peak"] = max(p["peak"], qclose)
            held = i - p["entry_i"] + 1
            reason, fraction = None, 1.0
            if held >= hold:
                reason = "15_session_exit" if hold == 15 else "time_exit"
            elif raw["is_st"][i - 1, c] != 0:
                reason = "became_st_or_unknown"
            elif exit_policy == "legacy":
                if qclose < p["q_entry"] - 2 * atr or qclose < ma20 * 0.98:
                    reason = "structure_failed"
                elif qclose < ma10 and float(f["location"][i - 1, c]) < 0.4:
                    reason, fraction = "weakness_reduce", 0.5 if not p["reduced"] else 1.0
                elif p["peak"] > p["q_entry"] + 2 * atr and qclose < p["peak"] - 1.5 * atr:
                    reason, fraction = "profit_protection", 0.5 if not p["reduced"] else 1.0
            elif exit_policy == "reversion":
                if qclose < p["q_entry"] - 1.5 * p["atr_entry"]:
                    reason = "repair_failed"
                elif qclose > ma10 and qclose > p["q_entry"]:
                    reason, fraction = "normalization_take_part", 0.5 if not p["reduced"] else 1.0
                elif held >= 4 and qclose < p["q_entry"] and float(f["location"][i - 1, c]) < 0.3:
                    reason, fraction = "repair_stalled_reduce", 0.5 if not p["reduced"] else 1.0
            elif exit_policy == "launch":
                if qclose < p["support"] - 0.5 * p["atr_entry"] or qclose < p["q_entry"] - 2 * p["atr_entry"]:
                    reason = "launch_support_failed"
                elif held >= 4 and qclose < ma10 and float(f["location"][i - 1, c]) < 0.35:
                    reason, fraction = "launch_weakened_reduce", 0.5 if not p["reduced"] else 1.0
                elif p["peak"] > p["q_entry"] + 2 * p["atr_entry"] and qclose < p["peak"] - 2 * p["atr_entry"]:
                    reason, fraction = "launch_profit_protection", 0.5 if not p["reduced"] else 1.0
            else:
                raise ValueError("unknown exit policy")
            if reason and i > p["entry_i"]:
                if tradable_open(raw, valid, i, c, codes[c], True, factors, multiplier, events):
                    sell(c, fraction, i, reason)
                else:
                    stats["blocked_exit_sessions"] += 1
                continue
            if p["stage"] < 3 and not p["reduced"] and held <= 7 and qclose > ma10 and qclose > p["q_entry"] + p["stage"] * 0.5 * atr:
                if valid[i - 1, c] and raw["is_st"][i - 1, c] == 0 and tradable_open(raw, valid, i, c, codes[c], False, factors, multiplier, events):
                    price = float(raw["open"][i, c]) * (1 + 0.001 * multiplier)
                    current_value = p["qty"] * price
                    budget = min(cash - 10, prev_equity * 0.06, max(0, prev_equity * 0.2 - current_value), float(raw["amount"][i - 1, c]) * 0.01)
                    buy(c, lot_buy(codes[c], budget, price), i, "strength_confirm_add", p)
        candidates = np.flatnonzero(signal[i - 1])
        if name == "baseline_hash":
            mixed = code_hash[candidates] ^ (np.uint64(day) * np.uint64(2654435761))
            mixed ^= mixed >> np.uint64(16)
            ranking = (mixed * np.uint64(2246822519)) & np.uint64(0xFFFFFFFF)
        elif name == "baseline_momentum20":
            ranking = f["r20"][i - 1, candidates]
        else:
            ranking = score[i - 1, candidates]
        ranked = candidates[np.argsort(-ranking, kind="stable")]
        for c in ranked:
            c = int(c)
            if len(positions) >= 4:
                break
            if c in positions:
                continue
            if not tradable_open(raw, valid, i, c, codes[c], False, factors, multiplier, events):
                stats["blocked_entries"] += 1
                continue
            if float(raw["open"][i, c]) > float(raw["close"][i - 1, c]) * 1.04:
                continue
            price = float(raw["open"][i, c]) * (1 + 0.001 * multiplier)
            budget = min(cash - 10, prev_equity * (0.2 if full_entry else 0.08), float(raw["amount"][i - 1, c]) * 0.01)
            p = buy(c, lot_buy(codes[c], budget, price), i, "full_entry_ablation" if full_entry else "initial_probe")
            if p and full_entry:
                p["stage"] = 3
        equity = cash + receivable
        for c, p in positions.items():
            if valid[i, c]:
                p["last"] = float(raw["close"][i, c])
                p["last_quote_i"] = i
            else:
                stats["suspended_marks"] += 1
            equity += p["qty"] * p["last"]
        assert cash >= -0.01 and all(p["qty"] > 0 for p in positions.values())
        curve.append({"date": day, "equity": round(equity, 4), "cash": round(cash, 4),
                      "dividend_receivable": round(receivable, 4), "positions": len(positions)})
        prev_equity = equity
    values = np.array([r["equity"] for r in curve])
    pnl = np.array([r["net_pnl"] for r in completed])
    trade_pct = np.array([r["net_pct"] for r in completed])
    initial = np.concatenate(([100000.0], values))
    drawdown = float(np.max(1 - initial / np.maximum.accumulate(initial))) if len(values) else 0
    returns = values[-1] / 100000 - 1 if len(values) else 0
    daily_ret = initial[1:] / initial[:-1] - 1
    metrics = {"net_return_pct": round(returns * 100, 3), "max_drawdown_pct": round(drawdown * 100, 3),
               "return_over_drawdown": round(returns / max(drawdown, .001), 3),
               "completed_holding_cycles": len(completed), "open_positions": len(positions),
               "win_rate_pct": round(float((pnl > 0).mean()) * 100, 2) if len(pnl) else None,
               "mean_cycle_net_pct": round(float(trade_pct.mean()), 4) if len(pnl) else None,
               "median_cycle_net_pct": round(float(np.median(trade_pct)), 4) if len(pnl) else None,
               "profit_factor": round(float(pnl[pnl > 0].sum() / -pnl[pnl < 0].sum()), 3) if np.any(pnl < 0) else None,
               "mean_holding_sessions": round(float(np.mean([r["holding_sessions"] for r in completed])), 2) if len(pnl) else None,
               "overdue_cycles": sum(r["overdue"] > 0 for r in completed),
               "overdue_open_positions": sum(p["overdue"] > 0 for p in positions.values()),
               "max_overdue_sessions": max([r["overdue"] for r in completed] + [p["overdue"] for p in positions.values()] + [0]),
               "annualized_sharpe_descriptive": round(float(daily_ret.mean() / daily_ret.std() * math.sqrt(244)), 3) if daily_ret.std() > 0 else None,
               "exposure_mean_pct": round(float(np.mean([(r["equity"]-r["cash"])/r["equity"] for r in curve])) * 100, 3) if curve else 0,
               "stale_open_positions": sum(int(indices[-1]) - p["last_quote_i"] > 0 for p in positions.values()),
               "dividend_receivable_cny": round(receivable, 3),
               **stats}
    return {"metrics": metrics, "curve": curve, "cycles": completed, "orders": orders,
            "unclosed": [{"code": p["code"], "entry_date": p["entry_date"], "qty": p["qty"], "mark": p["last"],
                          "overdue": p["overdue"], "last_quote_date": int(dates[p["last_quote_i"]]),
                          "unrealized_and_receivable_pnl": round(p["out"] + p["qty"] * p["last"] + p["dividends"] - p["in"], 3),
                          "stale_sessions": int(indices[-1]) - p["last_quote_i"], "bonus_locked": p["bonus_locked"],
                          "valuation_status": "last_quote_with_unverified_liquidation"} for p in positions.values()]}


def save(path, obj):
    path.write_text(json.dumps(obj, ensure_ascii=False, indent=2, allow_nan=False), encoding="utf-8")


def paired_block_bootstrap(candidate, baseline):
    """Descriptive paired 20-session block bootstrap; does not correct selection bias."""
    assert [r["date"] for r in candidate] == [r["date"] for r in baseline]
    a = np.array([100000.] + [r["equity"] for r in candidate])
    b = np.array([100000.] + [r["equity"] for r in baseline])
    delta = np.log(a[1:] / a[:-1]) - np.log(b[1:] / b[:-1])
    rng = np.random.default_rng(20261001)
    length, block = len(delta), 20
    if length < block:
        return {"status": "insufficient_sessions"}
    starts = rng.integers(0, length - block + 1, size=(2000, math.ceil(length / block)))
    picked = (starts[:, :, None] + np.arange(block)[None, None, :]).reshape(2000, -1)[:, :length]
    samples = delta[picked].sum(axis=1)
    return {"block_sessions": block, "replications": 2000,
            "relative_log_return_pct": round(float(delta.sum()) * 100, 3),
            "descriptive_95_interval_pct": [round(float(x) * 100, 3) for x in np.quantile(samples, [.025, .975])],
            "warning": "未校正多候选选择、数据偏差或制度变化；不能作为未来成功概率"}


def self_check():
    assert lot_buy("000001", 999, 10) == 0
    assert lot_buy("000001", 1100, 10) == 100
    assert lot_buy("688001", 1990, 10) == 0
    assert lot_buy("688001", 2010, 10) == 201
    assert lot_sell("000001", 350, 1) == 350
    assert lot_sell("688001", 300, .5) == 0
    assert fee(10000, True, 20230827) > fee(10000, True, 20230828)
    x = np.arange(1, 11, dtype=np.float32).reshape(-1, 1)
    y = roll(x, 3)
    assert math.isnan(y[1, 0]) and y[2, 0] == 2 and lag(y)[3, 0] == 2
    print("self-check passed", flush=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", type=Path, default=Path(__file__).parent / "exports")
    p.add_argument("--output", type=Path, default=Path(__file__).parent / "results")
    p.add_argument("--self-check", action="store_true")
    args = p.parse_args()
    self_check()
    if args.self_check:
        return
    args.output.mkdir(parents=True, exist_ok=True)
    codes, dates, raw, seen, valid, factors, events, quality = load(args.data)
    print("loaded", quality, flush=True)
    f = features(raw, valid, factors, seen)
    summary = {"quality": quality, "families": {}, "selected_on_validation_only": None,
               "data_and_execution_admission": "pending_data_audit"}
    for name in f["signals"]:
        rows = {}
        for period, (start, end) in PERIODS.items():
            result = backtest(name, start, end, codes, dates, raw, valid, factors, events, f)
            rows[period] = result["metrics"]
            print(name, period, result["metrics"], flush=True)
        summary["families"][name] = rows
        save(args.output / "summary-progress.json", summary)
    eligible = [(name, rows["validation"]) for name, rows in summary["families"].items() if not name.startswith("baseline")]
    positive = [(name, m) for name, m in eligible if m["net_return_pct"] > 0 and (m["mean_cycle_net_pct"] or -1) > 0 and m["completed_holding_cycles"] >= 100]
    pool = positive or eligible
    selected = max(pool, key=lambda x: x[1]["return_over_drawdown"])[0]
    summary["selected_on_validation_only"] = selected
    summary["validation_had_qualifying_candidate"] = bool(positive)
    detailed = backtest(selected, *PERIODS["test"], codes, dates, raw, valid, factors, events, f, journal=True)
    save(args.output / "selected-test-ledger.json", detailed)
    summary["paired_test_baselines"] = {}
    for baseline in ("baseline_hash", "baseline_momentum20"):
        reference = backtest(baseline, *PERIODS["test"], codes, dates, raw, valid, factors, events, f)
        summary["paired_test_baselines"][baseline] = paired_block_bootstrap(detailed["curve"], reference["curve"])
        save(args.output / (baseline + "-test-ledger.json"), reference)
    annual, year_start, last_value, current_year = {}, 100000., 100000., None
    for row in detailed["curve"]:
        year = row["date"] // 10000
        if year != current_year:
            if current_year is not None:
                annual[str(current_year)] = round((last_value / year_start - 1) * 100, 3)
            current_year, year_start = year, last_value
        last_value = row["equity"]
    if current_year is not None:
        annual[str(current_year)] = round((last_value / year_start - 1) * 100, 3)
    summary["selected_test_calendar_year_returns_pct"] = annual
    sensitivity = {}
    for label, kwargs in [("double_cost", {"multiplier": 2}), ("full_entry_ablation", {"full_entry": True}),
                          ("12_session", {"hold": 12}), ("14_session", {"hold": 14})]:
        sensitivity[label] = backtest(selected, *PERIODS["test"], codes, dates, raw, valid, factors, events, f, **kwargs)["metrics"]
    summary["selected_test_sensitivity"] = sensitivity
    last = len(dates) - 1
    candidates = []
    for c in np.flatnonzero(f["signals"][selected][last]):
        candidates.append({"code": codes[c], "as_of": int(dates[last]), "raw_close": float(raw["close"][last, c]),
                           "score": float(f["score"][last, c]), "pe_ttm": float(raw["pe_ttm"][last, c]),
                           "pb": float(raw["pb"][last, c])})
    candidates.sort(key=lambda r: (-r["score"], r["code"]))
    save(args.output / "historical-last-session-candidates.json", candidates[:30])
    save(args.output / "summary.json", summary)
    print("selected", selected, "results", args.output, flush=True)


if __name__ == "__main__":
    main()
