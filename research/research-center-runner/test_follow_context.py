"""Contract and refusal tests; all writable inputs are temporary fixtures."""
import copy
import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import numpy as np

import follow_context as context
import model_runner


def load_npz(np_module, path):
    with np_module.load(path, allow_pickle=False) as data:
        return {key: data[key] for key in data.files}


class FollowContextTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="frozen-follow-test-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.snapshot = self.root / "original"
        self.snapshot.mkdir()
        self.index = self.root / "CSI300.ndjson"
        self.bundle = self.root / "model-run.json"
        self.output = self.root / "follow.json"
        self.runner_path = self.root / "model_runner.py"
        self.runner_path.write_text("# frozen test runner\n", encoding="utf8")
        self.registry_path = self.root / "registry.json"
        self.codes = ["000001", "600000", "300001", "688001"]
        self.dates = np.array([20260924, 20260925, 20260928], dtype=np.int32)
        shape = (len(self.dates), len(self.codes))
        self.data = {"codes": np.array(self.codes), "dates": self.dates,
                     "valid": np.ones(shape, bool), "seen": np.ones(shape, bool),
                     "factors": np.full(shape, 2., np.float32)}
        for field in ("open", "high", "low", "close", "volume", "amount", "is_st", "pe_ttm", "pb"):
            self.data["raw_" + field] = np.full(shape, 0 if field == "is_st" else 10, np.float32)
        self.data["raw_damaged"] = np.zeros(shape, bool)
        self.data["valid"][-1, -1] = False
        self.data["seen"][-1, -1] = False
        for field in ("close", "amount", "is_st"):
            self.data["raw_" + field][-1, -1] = np.nan
        np.savez(self.snapshot / "matrices.npz", **self.data)
        self.metadata = {"schema": "test matrices", "shared_study_sha256": None,
                         "events": [{"i": 1, "c": 0, "event": {"div": 0, "give": 0, "trans": 0}}]}
        self.index_rows = [{"code": "sh.000300", "date": day, "close": 3500.}
                           for day in ("2026-09-24", "2026-09-25", "2026-09-28")]
        self.write_index(self.index, self.index_rows)
        scores = np.array([.5, .5, -.25, np.nan], np.float32)
        self.score_path = self.root / "scores.npy"
        np.save(self.score_path, scores, allow_pickle=False)
        (self.root / "model2026.pkl").write_bytes(b"model identity only, never unpickled")
        (self.root / "study.py").write_text("# verified test feature source\n", encoding="utf8")
        self.metadata["shared_study_sha256"] = context.digest(self.root / "study.py")
        self.write_json(self.snapshot / "matrices-metadata.json", self.metadata)
        paths = {"snapshot": "original/matrices.npz", "metadata": "original/matrices-metadata.json",
                 "index": "CSI300.ndjson", "study": "study.py"}
        for model in context.MODELS:
            paths[model + "_score"] = "scores.npy"
            paths[model + "_model2026"] = "model2026.pkl"
        self.registry = {"schema": "trusted-model-runner-registry-v1", "as_of": "2026-09-28", "seed_sessions": len(self.dates),
                         "files": {key: {"path": path, "sha256": context.digest(self.root / path)}
                                   for key, path in paths.items()},
                         "models": {model: {"signal_threshold": .6 if model == "breadth22_rank20" else 0,
                                            "label_holding_days": 20, "source_run_id": "frozen-test-run"}
                                    for model in context.MODELS}}
        self.write_json(self.registry_path, self.registry)
        self.hashes = {key: row["sha256"] for key, row in self.registry["files"].items()}
        self.run = {"schema": "ashare-model-run-v1", "model_id": "breadth22_h20", "model_name": "冻结模型",
                    "holding_days": 20, "label_holding_days": 20, "comparison": "baseline", "mode": "forward",
                    "signal_threshold": 0, "source_run_id": "frozen-test-run", "as_of": "2026-09-28",
                    "forward_start": 20260928, "stocks": len(self.codes), "sessions": len(self.dates),
                    "training_refitted": False, "frozen_prefix_preserved": True,
                    "runner_sha256": context.digest(self.runner_path), "input_sha256": self.hashes.copy(),
                    "model_sha256": self.hashes["breadth22_h20_model2026"],
                    "score_cache_sha256": self.hashes["breadth22_h20_score"],
                    "data_sha256": {key: self.hashes[key] for key in ("snapshot", "metadata", "index")},
                    "checks": {key: True for key in ("trusted_inputs_sha_verified", "model_score_prefix_preserved",
                                                     "current_scores_match_signal_watch", "no_AI_script_or_refit")},
                    "current_scores": [{"symbol": context.symbol(code), "score": score, "threshold": 0,
                                        "as_of": "2026-09-28"}
                                       for code, score in zip(self.codes[:3], (.5, .5, -.25))]}
        # Original runner's lexical watch order differs from the original code axis.
        self.run["signal_watch"] = [{**row, "close": 10., "signal_eligible": True,
                                     "cash_reference_eligible": False}
                                    for row in reversed(self.run["current_scores"][:2])]
        self.save_bundle()
        self.features = Mock(return_value=self.feature_fixture(self.data))
        self.validate_snapshot = Mock(side_effect=self.check_snapshot_fixture)
        self.research = SimpleNamespace(np=np, s=SimpleNamespace(STOCK=re.compile(r"\d{6}"), features=self.features),
                                         cli=SimpleNamespace(validate_snapshot=self.validate_snapshot, index_inputs=lambda dates,path,np,s: (np.zeros((len(dates),4),np.float32),int(dates[-1]))))
        self.runner = SimpleNamespace(ROOT=self.root, load_npz=load_npz,
                                      validate_extension=model_runner.validate_extension,
                                      verify_sources=Mock(side_effect=self.verify_fixture))
        for name, value in (("RUNNER_PATH", self.runner_path), ("REGISTRY_PATH", self.registry_path),
                            ("SOURCE_RUNNER_SHA256", context.digest(self.runner_path)),
                            ("SOURCE_REGISTRY_SHA256", context.digest(self.registry_path))):
            patcher = patch.object(context, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        source_patch = patch.object(context, "load_source", return_value=self.runner)
        self.source_loader = source_patch.start()
        self.addCleanup(source_patch.stop)
        research_patch = patch.object(context, "load_research", return_value=self.research)
        self.research_loader = research_patch.start()
        self.addCleanup(research_patch.stop)

    @staticmethod
    def write_json(path, value):
        path.write_text(context.compact(value), encoding="utf8")

    @staticmethod
    def write_index(path, rows):
        path.write_text("\n".join(context.compact(row) for row in rows), encoding="utf8")

    def save_bundle(self):
        text = context.compact(self.run)
        self.write_json(self.bundle, {"schema": "model-run-bundle-v1", "content": text,
                                     "content_sha256": hashlib.sha256(text.encode("utf8")).hexdigest()})

    def verify_fixture(self):
        for key, row in self.registry["files"].items():
            if context.digest(self.root / row["path"]) != row["sha256"]:
                raise ValueError("受信源漂移：" + key)
        return copy.deepcopy(self.registry), self.hashes.copy()

    @staticmethod
    def feature_fixture(data):
        # Distinct output values demonstrate that the exporter uses s.features.
        close = np.full(data["valid"].shape, 23.5, np.float32)
        close[-1, -1] = 25.
        atr = np.full(close.shape, 6.75, np.float32)
        atr[-1, -1] = np.nan
        return {"close": close, "atr": atr, "ma10": np.full(close.shape,22.,np.float32), "eligible": data["valid"].copy()}

    @staticmethod
    def check_snapshot_fixture(codes, dates, raw, seen, valid, factors, np_module, pattern):
        shape = (len(dates), len(codes))
        if (any(not pattern.fullmatch(code) for code in codes) or len(set(codes)) != len(codes)
                or any(array.shape != shape for array in (*raw.values(), seen, valid, factors))
                or not np_module.all(np_module.isfinite(factors) & (factors > 0))):
            raise ValueError("冻结形状/因子无效")

    def export(self, snapshot=None, index=None):
        return context.export_context(self.bundle, snapshot or self.snapshot, index or self.index, self.output)

    def extend(self):
        directory = self.root / "extension"
        directory.mkdir()
        data = {key: value.copy() for key, value in self.data.items()}
        data["dates"] = np.append(data["dates"], np.int32(20260929))
        for key in data.keys() - {"dates", "codes"}:
            data[key] = np.concatenate((data[key], data[key][-1:]), axis=0)
        np.savez(directory / "matrices.npz", **data)
        self.write_json(directory / "matrices-metadata.json", self.metadata)
        index = self.root / "extended-CSI300.ndjson"
        self.write_index(index, self.index_rows + [{"code": "sh.000300", "date": "2026-09-29", "close": 3501.}])
        self.run["as_of"] = "2026-09-29"
        self.run["sessions"] = len(data["dates"])
        for key in ("current_scores", "signal_watch"):
            for row in self.run[key]:
                row["as_of"] = self.run["as_of"]
        self.features.return_value = self.feature_fixture(data)
        self.bind_data(directory, index)
        return directory, index, data

    def bind_data(self, directory, index):
        self.run["data_sha256"] = {"snapshot": context.digest(directory / "matrices.npz"),
                                   "metadata": context.digest(directory / "matrices-metadata.json"),
                                   "index": context.digest(index)}
        self.save_bundle()

    def test_full_export_contract_and_source_feature_values(self):
        before = {path: context.digest(path) for path in (self.bundle, self.index, self.registry_path,
                  self.runner_path, self.snapshot / "matrices.npz", self.snapshot / "matrices-metadata.json")}
        result = self.export()
        envelope = context.read_json(self.output)
        content = context.parse_json(envelope["content"])
        self.assertEqual(set(envelope), {"schema", "content", "content_sha256"})
        self.assertEqual(envelope["schema"], "model-follow-context-bundle-v1")
        self.assertEqual(envelope["content_sha256"], hashlib.sha256(envelope["content"].encode("utf8")).hexdigest())
        self.assertEqual(set(content), {"schema", "as_of", "model_id", "model_sha256", "source_run_sha256",
                                       "source_runner_sha256", "context_runner_sha256", "data_sha256",
                                       "session_dates", "ranked_symbols", "rows", "market"})
        self.assertEqual(content["schema"], "frozen-model-execution-v1")
        self.assertEqual(content["source_run_sha256"], context.read_json(self.bundle)["content_sha256"])
        self.assertEqual(content["source_runner_sha256"], context.digest(self.runner_path))
        self.assertEqual(content["context_runner_sha256"], context.digest(Path(context.__file__)))
        self.assertEqual(content["data_sha256"], self.run["data_sha256"])
        self.assertEqual(content["session_dates"], [row["date"] for row in self.index_rows])
        self.assertEqual(content["ranked_symbols"], ["sz000001", "sh600000"])
        self.assertEqual(result["rows"], 4)
        self.assertEqual(content["rows"][0], {"symbol": "sz000001", "close": 10., "adjusted_close": 23.5,
                                              "atr14": 6.75, "factor": 2., "ma10": 22., "amount": 10., "valid": True, "is_st": 0})
        self.assertEqual(content["rows"][-1], {"symbol": "sh688001", "close": None, "adjusted_close": 25.,
                                               "atr14": None, "factor": 2., "ma10": 22., "amount": None, "valid": False, "is_st": None})
        self.features.assert_called_once()
        self.validate_snapshot.assert_called_once()
        self.assertEqual(self.runner.verify_sources.call_count, 2)
        self.assertEqual(before, {path: context.digest(path) for path in before})
        self.assertEqual(envelope["content"], context.compact(content))

    def test_tampered_bundle_hash_is_rejected_before_research_import(self):
        envelope = context.read_json(self.bundle)
        envelope["content"] += " "
        self.write_json(self.bundle, envelope)
        with self.assertRaisesRegex(ValueError, "content_sha256"):
            self.export()
        self.research_loader.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_execution_facts_do_not_invent_short_market_history(self):
        context.export_context(self.bundle, self.snapshot, self.index, self.output)
        body=json.loads(context.read_json(self.output)["content"])
        self.assertEqual(body["market"], {"as_of":"2026-09-28","csi300_return5":None,"csi300_ma60_deviation":None})
        self.assertEqual(body["rows"][0]["ma10"],22.)

    def test_execution_facts_reject_market_date_mismatch(self):
        self.research.cli.index_inputs=lambda dates,path,np,s: (np.zeros((len(dates),4),np.float32),20260925)
        with self.assertRaisesRegex(ValueError,"执行市场条件"):
            context.export_context(self.bundle,self.snapshot,self.index,self.output)
        self.assertFalse(self.output.exists())

    def test_rehashed_bundle_scores_are_checked_against_frozen_cache(self):
        self.run["current_scores"][0]["score"] = .75
        self.run["signal_watch"][-1]["score"] = .75
        self.save_bundle()
        with self.assertRaisesRegex(ValueError, "原冻结评分缓存"):
            self.export()
        self.assertFalse(self.output.exists())

    def test_baseline_holding_and_forward_restrictions(self):
        original = copy.deepcopy(self.run)
        changes = [("comparison", value) for value in ("holding15", "staged", "cost_double", "verified_actions")]
        changes += [("holding_days", 15), ("holding_days", True), ("label_holding_days", 15), ("mode", "replay")]
        for key, value in changes:
            with self.subTest(key=key, value=value):
                self.run = copy.deepcopy(original)
                self.run[key] = value
                self.save_bundle()
                with self.assertRaisesRegex(ValueError, "baseline"):
                    self.export()
        self.research_loader.assert_not_called()

    def test_only_five_frozen_technical_models(self):
        for model in ("fundamental37_h20", "tsp_sequence", "baseline_hash", "layered_any"):
            with self.subTest(model=model):
                self.run["model_id"] = model
                self.save_bundle()
                with self.assertRaisesRegex(ValueError, "五个冻结技术模型"):
                    self.export()
        for model in context.MODELS:
            run = copy.deepcopy(self.run)
            run.update(model_id=model, model_sha256=self.hashes[model + "_model2026"],
                       score_cache_sha256=self.hashes[model + "_score"],
                       signal_threshold=self.registry["models"][model]["signal_threshold"])
            context.validate_run(run, self.registry, self.hashes, context.digest(self.runner_path))

    def test_all_identity_hashes_and_original_threshold_are_bound(self):
        original = copy.deepcopy(self.run)
        mutations = [("runner_sha256", "0" * 64), ("model_sha256", "0" * 64),
                     ("score_cache_sha256", "0" * 64), ("source_run_id", "other"),
                     ("signal_threshold", .1), ("input_sha256", {}), ("training_refitted", True)]
        for key, value in mutations:
            with self.subTest(key=key):
                self.run = copy.deepcopy(original)
                self.run[key] = value
                self.save_bundle()
                with self.assertRaises(ValueError):
                    self.export()
        self.research_loader.assert_not_called()

    def test_every_data_sha_must_match(self):
        for key in ("snapshot", "metadata", "index"):
            with self.subTest(key=key):
                self.run["data_sha256"] = {name: self.hashes[name] for name in ("snapshot", "metadata", "index")}
                self.run["data_sha256"][key] = "0" * 64
                self.save_bundle()
                with self.assertRaisesRegex(ValueError, "data SHA"):
                    self.export()
        self.source_loader.assert_not_called()

    def test_runner_and_registry_are_pinned_even_if_bundle_is_rehashed(self):
        before = self.runner_path.read_bytes()
        self.runner_path.write_bytes(before + b"# changed\n")
        self.run["runner_sha256"] = context.digest(self.runner_path)
        self.save_bundle()
        with self.assertRaisesRegex(ValueError, "SHA"):
            self.export()
        self.runner_path.write_bytes(before)
        self.registry_path.write_text(self.registry_path.read_text(encoding="utf8") + " ", encoding="utf8")
        with self.assertRaisesRegex(ValueError, "SHA"):
            self.export()
        self.source_loader.assert_not_called()

    def test_stale_as_of_and_axis_counts_are_rejected(self):
        original = copy.deepcopy(self.run)
        for key, value in (("as_of", "2026-09-25"), ("sessions", 4), ("stocks", 5), ("forward_start", 20260925)):
            with self.subTest(key=key):
                self.run = copy.deepcopy(original)
                self.run[key] = value
                self.save_bundle()
                with self.assertRaises(ValueError):
                    self.export()
        self.features.assert_not_called()

    def test_dates_order_duplicate_unsigned_and_invalid_calendar(self):
        for dates in ([20260928, 20260925], [20260925, 20260925], [20260927], [20260230], []):
            with self.subTest(dates=dates):
                with self.assertRaises(ValueError):
                    context.session_dates(np, np.array(dates, dtype=np.uint32), "2026-09-28")

    def test_future_unfinished_day_and_model_year(self):
        before_close = dt.datetime(2026, 9, 28, 15, 59, tzinfo=context.CHINA)
        with self.assertRaisesRegex(ValueError, "尚未完成"):
            context.session_dates(np, self.dates, "2026-09-28", before_close)
        with self.assertRaisesRegex(ValueError, "未来"):
            context.session_dates(np, np.array([20260929]), "2026-09-29", before_close)
        with self.assertRaisesRegex(ValueError, "2026"):
            context.session_dates(np, np.array([20270104]), "2027-01-04", dt.datetime(2027, 1, 5, tzinfo=context.CHINA))
        self.assertEqual(context.session_dates(np, self.dates, "2026-09-28", before_close.replace(hour=16)),
                         [row["date"] for row in self.index_rows])

    def test_ties_use_code_axis_and_candidates_are_never_truncated(self):
        codes = [f"{code:06d}" for code in range(30, 0, -1)]
        scored = [{"symbol": context.symbol(code), "score": .5, "threshold": 0, "as_of": self.run["as_of"]}
                  for code in reversed(codes)]
        run = {**self.run, "current_scores": scored,
               "signal_watch": [{**row, "signal_eligible": True, "close": 10.} for row in scored]}
        ranked = context.ranked_symbols(run, codes, np.ones(30, bool), np.full(30, 10.))
        self.assertEqual(ranked, [context.symbol(code) for code in codes])
        self.assertEqual(len(ranked), 30)
        run["signal_watch"][0]["score"] = .75
        run["current_scores"][0]["score"] = .75
        ranked = context.ranked_symbols(run, codes, np.ones(30, bool), np.full(30, 10.))
        self.assertEqual(ranked, ["sz000001"] + [context.symbol(code) for code in codes[:-1]])

    def test_watch_must_equal_positive_current_scores_without_duplicates(self):
        original = copy.deepcopy(self.run)
        for change in ("missing", "duplicate", "unknown", "as_of", "threshold", "close"):
            with self.subTest(change=change):
                self.run = copy.deepcopy(original)
                if change == "missing":
                    self.run["signal_watch"].pop()
                elif change == "duplicate":
                    self.run["signal_watch"].append(copy.deepcopy(self.run["signal_watch"][0]))
                elif change == "unknown":
                    self.run["signal_watch"][0]["symbol"] = "sz999999"
                else:
                    self.run["signal_watch"][0][change] = "2026-09-25" if change == "as_of" else 11.
                self.save_bundle()
                with self.assertRaises(ValueError):
                    self.export()
                self.assertFalse(self.output.exists())

    def test_extension_uses_run_scores_without_predicting(self):
        directory, index, _ = self.extend()
        for key in ("current_scores", "signal_watch"):
            for row in self.run[key]:
                if row["score"] > 0:
                    row["score"] = .75  # Deliberately differs from the old cache's final scores.
        self.save_bundle()
        self.export(directory, index)
        content = context.parse_json(context.read_json(self.output)["content"])
        self.assertEqual(content["as_of"], "2026-09-29")
        self.assertEqual(len(content["session_dates"]), 4)
        self.assertEqual(content["ranked_symbols"], ["sz000001", "sh600000"])
        self.features.assert_called_once()

    def test_extension_preserves_every_matrix_prefix_and_code_axis(self):
        directory, index, data = self.extend()
        for key in ("raw_close", "codes", "dates", "factors"):
            with self.subTest(key=key):
                changed = {name: value.copy() for name, value in data.items()}
                if key == "codes":
                    changed[key][[0, 1]] = changed[key][[1, 0]]
                elif key == "dates":
                    changed[key][0] = 20260923
                else:
                    changed[key][0, 0] += 1
                np.savez(directory / "matrices.npz", **changed)
                self.bind_data(directory, index)
                with self.assertRaisesRegex(ValueError, "冻结历史/股票轴"):
                    self.export(directory, index)
        self.features.assert_not_called()

    def test_extension_cannot_change_array_dtype(self):
        directory, index, data = self.extend()
        data["raw_close"] = data["raw_close"].astype(np.float64)
        np.savez(directory / "matrices.npz", **data)
        self.bind_data(directory, index)
        with self.assertRaisesRegex(ValueError, "dtype"):
            self.export(directory, index)

    def test_metadata_and_index_frozen_prefixes_are_verified(self):
        directory, index, _ = self.extend()
        metadata = copy.deepcopy(self.metadata)
        metadata["events"][0]["event"]["div"] = .1
        self.write_json(directory / "matrices-metadata.json", metadata)
        self.bind_data(directory, index)
        with self.assertRaisesRegex(ValueError, "公司行动前缀"):
            self.export(directory, index)
        self.write_json(directory / "matrices-metadata.json", self.metadata)
        rows = copy.deepcopy(self.index_rows)
        rows[0]["close"] += 1
        rows.append({"code": "sh.000300", "date": "2026-09-29", "close": 3501.})
        self.write_index(index, rows)
        self.bind_data(directory, index)
        with self.assertRaisesRegex(ValueError, "CSI300修改"):
            self.export(directory, index)
        self.features.assert_not_called()

    def test_index_requires_exact_identity_and_complete_ordered_sessions(self):
        for rows in (self.index_rows[::-1], self.index_rows[:-1],
                     [{**row, "code": "sz.000300"} for row in self.index_rows]):
            with self.subTest(rows=rows):
                index = self.root / "bad-index.ndjson"
                self.write_index(index, rows)
                self.bind_data(self.snapshot, index)
                with self.assertRaisesRegex(ValueError, "CSI300身份/完整日期轴"):
                    self.export(index=index)
        self.features.assert_not_called()

    def test_index_accepts_frozen_provider_decimal_strings_but_rejects_bad_prices(self):
        index = self.root / "decimal-index.ndjson"
        sessions = [row["date"] for row in self.index_rows]
        rows = [{**row, "close": "3500.0000"} for row in self.index_rows]
        self.write_index(index, rows)
        context.index_calendar(index, sessions, index, len(sessions))
        for price in (True, None, "NaN", "Infinity", "invalid", "0", "-1"):
            with self.subTest(price=price):
                bad = copy.deepcopy(rows)
                bad[-1]["close"] = price
                self.write_index(index, bad)
                with self.assertRaisesRegex(ValueError, "CSI300价格"):
                    context.index_calendar(index, sessions, index, len(sessions))

    def test_invalid_latest_factor_and_st_are_rejected(self):
        directory, index, data = self.extend()
        for key, value in (("factors", 0.), ("factors", np.inf), ("raw_is_st", 2.)):
            with self.subTest(key=key, value=value):
                changed = {name: array.copy() for name, array in data.items()}
                changed[key][-1, 0] = value
                np.savez(directory / "matrices.npz", **changed)
                self.bind_data(directory, index)
                with self.assertRaises(ValueError):
                    self.export(directory, index)
        self.features.assert_not_called()

    def test_concurrent_bundle_change_leaves_no_output(self):
        def changed(*args):
            self.bundle.write_text(self.bundle.read_text(encoding="utf8") + " ", encoding="utf8")
            return self.feature_fixture(self.data)
        self.features.side_effect = changed
        with self.assertRaisesRegex(ValueError, "计算期间改变"):
            self.export()
        self.assertFalse(self.output.exists())

    def test_concurrent_trusted_source_change_leaves_no_output(self):
        def changed(*args):
            (self.root / "model2026.pkl").write_bytes(b"changed")
            return self.feature_fixture(self.data)
        self.features.side_effect = changed
        with self.assertRaisesRegex(ValueError, "受信源漂移"):
            self.export()
        self.assertFalse(self.output.exists())

    def test_existing_output_and_creation_race_preserve_existing_bytes(self):
        self.output.write_bytes(b"existing user result")
        with self.assertRaisesRegex(ValueError, "拒绝覆盖"):
            self.export()
        self.assertEqual(self.output.read_bytes(), b"existing user result")
        self.features.assert_not_called()
        self.output.unlink()
        def created(*args):
            self.output.write_bytes(b"concurrently created result")
            return self.feature_fixture(self.data)
        self.features.side_effect = created
        with self.assertRaises(FileExistsError):
            self.export()
        self.assertEqual(self.output.read_bytes(), b"concurrently created result")

    def test_context_cannot_be_used_as_run_and_json_is_strict(self):
        self.write_json(self.bundle, {"schema": "model-follow-context-bundle-v1", "content": "{}", "content_sha256": ""})
        with self.assertRaisesRegex(ValueError, "旧context"):
            self.export()
        for text in ('{"x":1,"x":2}', '{"score":NaN}', '{"score":Infinity}'):
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    context.parse_json(text)


if __name__ == "__main__":
    unittest.main()
