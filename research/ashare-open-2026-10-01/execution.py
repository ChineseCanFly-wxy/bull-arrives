"""Research-only execution fork; original baseline source is preserved."""
import hashlib
import math
import sys
from pathlib import Path
import numpy as np
OLD=Path(__file__).parent.parent / "ashare-2026-10-01"
sys.path.insert(0,str(OLD))
import study as s
fee,lot_buy,lot_sell,number=s.fee,s.lot_buy,s.lot_sell,s.number

def tradable_open(raw, valid, i, c, code, sell, factors, multiplier=1, events=None):
    """Open-only proxy; no execution-day completed high/low/close/volume quality gate."""
    if i == 0 or not valid[i-1,c]:
        return False
    price = float(raw["open"][i,c])
    st = float(raw["is_st"][i,c])
    if not math.isfinite(price) or price<=0 or not math.isfinite(st):
        return False
    if st != 0 and not sell:
        return False
    ref = float(raw["close"][i-1,c]) * factors[i-1,c] / factors[i,c]
    event = (events or {}).get((i,c))
    if event:
        detail=ACTION_DETAILS.get((i,c),{})
        div=detail.get("reference_div",number(event.get("div",0)))
        trans=detail.get("reference_trans",number(event.get("trans",0)))
        ref=(float(raw["close"][i-1,c])-div)/(1+number(event.get("give",0))+trans)
    pct=s.limit_fraction(code,int(DATES[i]))
    if st != 0 and not code.startswith(("300","301","688","689")):
        pct=.05
    boundary=round(ref*(1-pct if sell else 1+pct)+1e-8,2)
    slipped=price*(1-.001*multiplier if sell else 1+.001*multiplier)
    return min(price,slipped)>boundary+.005 if sell else max(price,slipped)<boundary-.005


def backtest(name, start, end, codes, dates, raw, valid, factors, events, f,
             multiplier=1, full_entry=False, hold=15, journal=False, exit_policy="legacy",
             max_positions=4, action_details=None):
    global DATES, ACTION_DETAILS
    DATES = dates
    ACTION_DETAILS=action_details or {}
    indices = np.where((dates >= start) & (dates <= end))[0]
    cash, receivable, positions, curve, completed, orders = 100000.0, 0.0, {}, [], [], []
    target=min(.2,.8/max_positions)
    payments, releases = [], []
    stats = {"blocked_exit_sessions": 0, "blocked_entries": 0, "corporate_events": 0,
             "verified_cash_payments": 0, "verified_share_releases": 0,
             "model_edge_exit_requests": 0,
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
        for at, amount in list(payments):
            if at<=i:
                cash+=amount; receivable-=amount
                payments.remove((at,amount)); stats["verified_cash_payments"]+=1
        for at,c,qty in list(releases):
            if at<=i:
                if c in positions:
                    positions[c]["bonus_locked"]-=qty
                releases.remove((at,c,qty)); stats["verified_share_releases"]+=1
        for c, p in list(positions.items()):
            event = events.get((i, c))
            if event:
                detail=(action_details or {}).get((i,c))
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
                old_qty=p["qty"]
                new_qty = int(math.floor(old_qty * (1 + give + trans) + 1e-6))
                p["bonus_locked"] += new_qty - old_qty
                if new_qty > p["qty"] and (not detail or detail.get("release_i") is None):
                    stats["bonus_share_events_without_listing_date"] += 1
                p["qty"] = new_qty
                p["last"] = (p["last"] - div) / (1 + give + trans)
                if detail:
                    pay_i, release_i=detail.get("pay_i"),detail.get("release_i")
                    if pay_i is not None and cash_div>0:
                        if pay_i<=i:
                            cash+=cash_div; receivable-=cash_div
                            stats["verified_cash_payments"]+=1
                        else:payments.append((pay_i,cash_div))
                    # Release only this event's new shares; earlier unknown events remain locked.
                    new_shares=new_qty-old_qty
                    if release_i is not None and new_shares>0:
                        if release_i<=i:
                            p["bonus_locked"]-=new_shares; stats["verified_share_releases"]+=1
                        else:releases.append((release_i,c,new_shares))
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
            elif exit_policy in ("time", "model"):
                if qclose < p["q_entry"] - 3*p["atr_entry"]:
                    reason="three_atr_failure"
                elif exit_policy=="model" and (not np.isfinite(score[i-1,c]) or score[i-1,c]<=0):
                    reason="model_edge_lost"
                    stats["model_edge_exit_requests"]+=1
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
                    budget = min(cash - 10, prev_equity * target*.3, max(0, prev_equity * target - current_value), float(raw["amount"][i - 1, c]) * 0.01)
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
            if len(positions) >= max_positions:
                break
            if c in positions:
                continue
            if not tradable_open(raw, valid, i, c, codes[c], False, factors, multiplier, events):
                stats["blocked_entries"] += 1
                continue
            if float(raw["open"][i, c]) > float(raw["close"][i - 1, c]) * 1.04:
                continue
            price = float(raw["open"][i, c]) * (1 + 0.001 * multiplier)
            budget = min(cash - 10, prev_equity * target*(1 if full_entry else .4), float(raw["amount"][i - 1, c]) * 0.01)
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
        assert cash >= -0.01 and all(p["qty"] > 0 and 0<=p["bonus_locked"]<=p["qty"] for p in positions.values())
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
