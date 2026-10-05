"""Read-only execution inputs for a frozen baseline/20-session forward model run.

The run supplies scores and candidates. Only the SHA-verified original study
computes adjusted prices and ATR; this exporter never predicts, fits or replays.
"""
import argparse
import datetime as dt
import hashlib
import importlib
import importlib.util
import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUNNER_PATH = HERE / "model_runner.py"
REGISTRY_PATH = HERE / "registry.json"
# Audit a new exporter version if either frozen identity is intentionally changed.
SOURCE_RUNNER_SHA256 = "fbf86d0069291ba62d735f405998be7b8c64bc16bd71e128bcc0fc40a4bf7bcb"
SOURCE_REGISTRY_SHA256 = "ba272c1201ab82b8ecf5527db191d41bf3fd37668b3dbee047c7092e430fc551"
MODELS = ("breadth22_h20", "index26_h20", "breadth22_excess_csi20",
          "breadth22_rank20", "breadth22_open_downside20")
CHINA = dt.timezone(dt.timedelta(hours=8))


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(4 * 1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def _stat(path):
    value = path.stat()
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


class Fingerprints:
    """SHA plus file identity/timestamps, checked on both sides of each read."""

    def __init__(self):
        self.files = {}

    def capture(self, path, expected=None):
        path = Path(path).resolve()
        before = _stat(path)
        sha = digest(path)
        if before != _stat(path) or (expected is not None and sha != expected):
            raise ValueError(f"文件指纹改变或SHA不匹配：{path}")
        value = (sha, before)
        if path in self.files and self.files[path] != value:
            raise ValueError(f"文件在验证期间改变：{path}")
        self.files[path] = value
        return sha

    def check(self):
        for path, (sha, stat) in tuple(self.files.items()):
            if _stat(path) != stat or self.capture(path) != sha:
                raise ValueError(f"文件在计算期间改变：{path}")


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"JSON字段重复：{key}")
        result[key] = value
    return result


def _bad_constant(value):
    raise ValueError(f"JSON含非有限数：{value}")


def parse_json(text):
    return json.loads(text, object_pairs_hook=_unique_object, parse_constant=_bad_constant)


def read_json(path):
    return parse_json(Path(path).read_text(encoding="utf8"))


def compact(value):
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"))


def read_bundle(path):
    envelope = read_json(path)
    if not isinstance(envelope, dict) or envelope.get("schema") != "model-run-bundle-v1":
        raise ValueError("输入必须为原model-run-bundle-v1，不能复用旧context作为模型run")
    text = envelope.get("content")
    if not isinstance(text, str):
        raise ValueError("模型run的content须为JSON字符串")
    sha = hashlib.sha256(text.encode("utf8")).hexdigest()
    if envelope.get("content_sha256") != sha:
        raise ValueError("模型run content_sha256不匹配")
    run = parse_json(text)
    if not isinstance(run, dict) or run.get("schema") != "ashare-model-run-v1":
        raise ValueError("模型run正文schema无效")
    return run, sha


def number(value, label):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f"{label}须为有限数")
    return value


def validate_run(run, registry, hashes, runner_sha):
    model_id = run.get("model_id")
    if model_id not in MODELS or model_id not in registry["models"]:
        raise ValueError("仅支持五个冻结技术模型；禁止基本面/TSP/随机/分层")
    if (run.get("comparison") != "baseline" or type(run.get("holding_days")) is not int
            or run["holding_days"] != 20 or type(run.get("label_holding_days")) is not int
            or run["label_holding_days"] != 20 or run.get("mode") != "forward"):
        raise ValueError("仅支持baseline、holding20、mode forward的原模型run")
    spec = registry["models"][model_id]
    if run.get("input_sha256") != hashes:
        raise ValueError("模型run与完整受信registry输入身份不一致")
    if (run.get("runner_sha256") != runner_sha
            or run.get("model_sha256") != hashes[model_id + "_model2026"]
            or run.get("score_cache_sha256") != hashes[model_id + "_score"]
            or run.get("source_run_id") != spec["source_run_id"]):
        raise ValueError("原runner/model/评分缓存/研究run身份不一致")
    if (number(run.get("signal_threshold"), "信号阈值") != spec["signal_threshold"]
            or spec["label_holding_days"] != 20):
        raise ValueError("信号阈值或标签期限不同于冻结模型")
    checks = run.get("checks", {})
    if (run.get("training_refitted") is not False or run.get("frozen_prefix_preserved") is not True
            or not isinstance(checks, dict) or any(checks.get(key) is not True for key in (
                "trusted_inputs_sha_verified", "model_score_prefix_preserved",
                "current_scores_match_signal_watch", "no_AI_script_or_refit"))):
        raise ValueError("模型run缺少原冻结/无重训/当前信号一致性验证")


def session_dates(np, dates, as_of, now=None):
    if (dates.ndim != 1 or dates.dtype.kind not in "iu" or not len(dates)
            or np.any(dates[1:] <= dates[:-1])):
        raise ValueError("日期轴须为非空、唯一且严格递增的整数市场日")
    result = []
    for value in dates:
        text = str(int(value))
        if len(text) != 8:
            raise ValueError("市场日期须为YYYYMMDD")
        day = dt.datetime.strptime(text, "%Y%m%d").date()
        if day.weekday() >= 5 or day.year > 2026:
            raise ValueError("日期不是工作日或超出冻结2026模型版本")
        result.append(day.isoformat())
    local = (now or dt.datetime.now(CHINA)).astimezone(CHINA)
    last = dt.date.fromisoformat(result[-1])
    if last > local.date() or (last == local.date() and local.hour < 16):
        raise ValueError("快照包含未来或尚未完成的行情日")
    if as_of != result[-1]:
        raise ValueError("模型run as_of不同于快照最后市场日，不能将旧信号当新数据")
    return result


def validate_metadata(metadata, original, old_n, shape, study_sha):
    if (not isinstance(metadata, dict) or metadata.get("schema") != original.get("schema")
            or metadata.get("shared_study_sha256") != study_sha
            or not isinstance(metadata.get("events"), list)):
        raise ValueError("metadata格式或冻结study身份不同")
    visited = set()
    for row in metadata["events"]:
        if (not isinstance(row, dict) or type(row.get("i")) is not int
                or type(row.get("c")) is not int or not isinstance(row.get("event"), dict)):
            raise ValueError("metadata公司行动事件格式无效")
        i, c = row["i"], row["c"]
        if not (0 <= i < shape[0] and 0 <= c < shape[1]) or (i, c) in visited:
            raise ValueError("metadata公司行动事件重复或轴错位")
        visited.add((i, c))
    if [row for row in metadata["events"] if row["i"] < old_n] != original["events"]:
        raise ValueError("metadata修改已冻结公司行动前缀")


def index_calendar(path, sessions, original_path, old_n):
    def rows(source):
        return [parse_json(line) for line in Path(source).read_text(encoding="utf8").splitlines()
                if line.strip()]

    updated = rows(path)
    if (any(not isinstance(row, dict) or row.get("code") != "sh.000300" for row in updated)
            or [row.get("date") for row in updated] != sessions):
        raise ValueError("CSI300身份/完整日期轴不匹配，不能填充、压缩或重排市场日")
    for row in updated:
        # The frozen CSI300 provider stores decimal prices as strings.
        value = row.get("close")
        if isinstance(value, bool) or not isinstance(value, (str, int, float)):
            raise ValueError("CSI300价格格式无效")
        try:
            price = float(value)
        except ValueError as exc:
            raise ValueError("CSI300价格格式无效") from exc
        if not math.isfinite(price) or price <= 0:
            raise ValueError("CSI300价格须为有限正值")
    original = rows(original_path)
    if len(original) != old_n or updated[:old_n] != original:
        raise ValueError("CSI300修改已冻结历史前缀")


def symbol(code):
    return ("sh" if code.startswith("6") else "sz") + code


def ranked_symbols(run, codes, eligible, raw_close, frozen_scores=None):
    axis = [symbol(code) for code in codes]
    columns = {name: c for c, name in enumerate(axis)}
    threshold, as_of = run["signal_threshold"], run["as_of"]

    def score_map(key):
        rows = run.get(key)
        if not isinstance(rows, list):
            raise ValueError(f"原run缺少{key}列表")
        result = {}
        for row in rows:
            if not isinstance(row, dict):
                raise ValueError(f"{key}评分记录格式无效")
            name = row.get("symbol")
            if not isinstance(name, str) or name not in columns or name in result:
                raise ValueError(f"{key}股票不在冻结轴内或重复")
            if (row.get("as_of") != as_of or number(row.get("threshold"), "评分阈值") != threshold
                    or not eligible[columns[name]]):
                raise ValueError(f"{key}日期/阈值/原技术资格不一致")
            result[name] = number(row.get("score"), "模型分数")
            if key == "signal_watch":
                if (row.get("signal_eligible") is not True
                        or number(row.get("close"), "信号close") != float(raw_close[columns[name]])):
                    raise ValueError("signal_watch的资格或未复权close与当前快照不一致")
        return result

    scored, watch = score_map("current_scores"), score_map("signal_watch")
    if watch != {name: score for name, score in scored.items() if score > threshold}:
        raise ValueError("signal_watch与current_scores的原阈值信号不同")
    if frozen_scores is not None:
        expected = {name: float(frozen_scores[c]) for c, name in enumerate(axis)
                    if eligible[c] and math.isfinite(float(frozen_scores[c]))}
        if scored != expected:
            raise ValueError("模型run当前分数与原冻结评分缓存不同")
    # Seed sorting with the original axis, never envelope order or lexical symbols.
    return sorted((name for name in axis if name in watch), key=lambda name: -watch[name])


def finite_or_none(value):
    value = float(value)
    return value if math.isfinite(value) else None


def load_source(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    # Execute the verified source bytes, without creating/reading runner bytecode.
    exec(compile(Path(path).read_bytes(), str(path), "exec"), module.__dict__)
    return module


def load_research(runner, registry):
    paths = list(sys.path)
    try:
        sys.path.insert(0, str(runner.BASE))
        research = load_source("_frozen_follow_model_study", runner.BASE / "model_study.py")
        modules = {"model_runner": research, "study": research.s, "relative_features": research.m,
                   "absolute_features": research.a, "rules": research.rules, "execution": research.e,
                   "index_adapter": research.cli}
        for key, module in modules.items():
            if Path(module.__file__).resolve() != (runner.ROOT / registry["files"][key]["path"]).resolve():
                raise ValueError(f"研究模块未从已验证原路径加载：{key}")
        if research.DATA.resolve() != (runner.ROOT / registry["files"]["snapshot"]["path"]).parent.resolve():
            raise ValueError("原研究模块的冻结数据路径与registry不一致")
        return research
    finally:
        sys.path[:] = paths


def build_content(run, run_sha, runner, research, registry, hashes, snapshot, index, context_sha, guard):
    np, s = research.np, research.s
    original_path = (runner.ROOT / registry["files"]["snapshot"]["path"]).resolve()
    data = runner.load_npz(np, snapshot / "matrices.npz")
    original = data if snapshot / "matrices.npz" == original_path else runner.load_npz(np, original_path)
    runner.validate_extension(np, data, original)
    if any(data[key].dtype != old.dtype for key, old in original.items()):
        raise ValueError("新快照改变冻结数组dtype")
    if data["codes"].ndim != 1:
        raise ValueError("股票轴须为一维")
    codes, dates = data["codes"].tolist(), data["dates"]
    sessions = session_dates(np, dates, run.get("as_of"))
    if (type(run.get("stocks")) is not int or run["stocks"] != len(codes)
            or type(run.get("sessions")) is not int or run["sessions"] != len(dates)):
        raise ValueError("原run的股票/市场日数量与快照不同")
    anchor = run.get("forward_start")
    if (type(anchor) is not int or anchor not in dates
            or anchor < int(registry["as_of"].replace("-", ""))):
        raise ValueError("原run前向起点不在冻结后完整市场日期轴内")
    raw = {key.removeprefix("raw_"): value for key, value in data.items() if key.startswith("raw_")}
    valid, seen, factors = data["valid"], data["seen"], data["factors"]
    research.cli.validate_snapshot(codes, dates, raw, seen, valid, factors, np, s.STOCK)
    if any(key not in raw for key in ("pe_ttm", "pb")):
        raise ValueError("原features所需的冻结行情字段缺失")
    st = raw["is_st"][-1]
    if np.any(np.isfinite(st) & (st != 0) & (st != 1)):
        raise ValueError("最新is_st只能为0/1或缺失")
    old_n = len(original["dates"])
    metadata = read_json(snapshot / "matrices-metadata.json")
    old_metadata = read_json(runner.ROOT / registry["files"]["metadata"]["path"])
    validate_metadata(metadata, old_metadata, old_n, valid.shape, hashes["study"])
    index_calendar(index, sessions, runner.ROOT / registry["files"]["index"]["path"], old_n)
    score_path = runner.ROOT / registry["files"][run["model_id"] + "_score"]["path"]
    frozen = np.load(score_path, allow_pickle=False, mmap_mode="r")
    if frozen.shape != (old_n, len(codes)) or frozen.dtype.kind != "f":
        raise ValueError("原评分缓存与冻结股票/日期轴不符")
    guard.check()
    features = s.features(raw, valid, factors, seen)
    for key in ("close", "atr", "eligible"):
        if features[key].shape != valid.shape:
            raise ValueError(f"原features形状不同：{key}")
    index_features, index_as_of = research.cli.index_inputs(dates, index, np, s)
    if int(index_as_of) != int(dates[-1]) or index_features.shape != (len(dates), 4):
        raise ValueError("执行市场条件与原CSI300日期轴不一致")
    market = {"as_of": run["as_of"],
              "csi300_return5": finite_or_none(index_features[-1, 0]) if len(dates) >= 6 else None,
              "csi300_ma60_deviation": finite_or_none(index_features[-1, 2]) if len(dates) >= 60 else None}
    ranked = ranked_symbols(run, codes, features["eligible"][-1], raw["close"][-1],
                            frozen[-1] if len(dates) == old_n else None)
    rows = [{"symbol": symbol(code), "close": finite_or_none(raw["close"][-1, c]),
             "adjusted_close": finite_or_none(features["close"][-1, c]),
             "atr14": finite_or_none(features["atr"][-1, c]), "factor": float(factors[-1, c]),
             "ma10": finite_or_none(features["ma10"][-1, c]) if "ma10" in features else None,
             "amount": finite_or_none(raw["amount"][-1, c]), "valid": bool(valid[-1, c]),
             "is_st": int(st[c]) if math.isfinite(float(st[c])) else None}
            for c, code in enumerate(codes)]
    return {"schema": "frozen-model-execution-v1", "as_of": run["as_of"],
            "model_id": run["model_id"], "model_sha256": run["model_sha256"],
            "source_run_sha256": run_sha, "source_runner_sha256": run["runner_sha256"],
            "context_runner_sha256": context_sha, "data_sha256": run["data_sha256"],
            "session_dates": sessions, "ranked_symbols": ranked, "rows": rows, "market": market}


def export_context(bundle, snapshot, index, output):
    output = Path(output).absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("输出已存在，拒绝覆盖")
    output = output.resolve()
    bundle, snapshot, index = Path(bundle).resolve(), Path(snapshot).resolve(), Path(index).resolve()
    guard = Fingerprints()
    guard.capture(bundle)
    runner_sha = guard.capture(RUNNER_PATH, SOURCE_RUNNER_SHA256)
    guard.capture(REGISTRY_PATH, SOURCE_REGISTRY_SHA256)
    context_sha = guard.capture(Path(__file__))
    run, run_sha = read_bundle(bundle)
    data_hashes = {"snapshot": guard.capture(snapshot / "matrices.npz"),
                   "metadata": guard.capture(snapshot / "matrices-metadata.json"),
                   "index": guard.capture(index)}
    if run.get("data_sha256") != data_hashes:
        raise ValueError("模型run的snapshot/metadata/index三项data SHA不完全匹配")
    runner = load_source("_frozen_follow_runner", RUNNER_PATH)
    registry, hashes = runner.verify_sources()
    if registry != read_json(REGISTRY_PATH) or registry.get("schema") != "trusted-model-runner-registry-v1":
        raise ValueError("受信registry身份无效或并发改变")
    for key, row in registry["files"].items():
        guard.capture(runner.ROOT / row["path"], hashes[key])
    validate_run(run, registry, hashes, runner_sha)
    prior_bytecode = sys.dont_write_bytecode
    sys.dont_write_bytecode = True
    try:
        guard.check()
        research = load_research(runner, registry)
        content = build_content(run, run_sha, runner, research, registry, hashes, snapshot, index, context_sha, guard)
        verified_registry, verified_hashes = runner.verify_sources()
        if verified_registry != registry or verified_hashes != hashes:
            raise ValueError("受信registry在计算期间改变")
        guard.check()
    finally:
        sys.dont_write_bytecode = prior_bytecode
    text = compact(content)
    envelope = {"schema": "model-follow-context-bundle-v1", "content": text,
                "content_sha256": hashlib.sha256(text.encode("utf8")).hexdigest()}
    encoded = compact(envelope)
    output.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation also rejects a file created after the initial exists check.
    with output.open("x", encoding="utf8", newline="\n") as stream:
        stream.write(encoded)
    return {"output": str(output), "as_of": content["as_of"], "model_id": content["model_id"],
            "rows": len(content["rows"]), "ranked_symbols": len(content["ranked_symbols"]),
            "content_sha256": envelope["content_sha256"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--index", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = export_context(args.bundle, args.snapshot, args.index, args.output)
    except (ValueError, OSError, KeyError, TypeError, AttributeError, ImportError) as exc:
        parser.exit(2, f"follow_context: {exc}\n")
    print(compact(result))


if __name__ == "__main__":
    main()
