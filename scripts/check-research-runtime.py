"""Run with the shipped Python, not the maintainer's interpreter."""
import importlib
import importlib.metadata
import json
import os
from pathlib import Path
import pickle
import re
import sys
import tempfile

root = Path(sys.argv[1]).resolve()
sys.path.insert(0, str(Path(__file__).resolve().parent))
from importlib.util import spec_from_file_location, module_from_spec
spec = spec_from_file_location("research_bundle", Path(__file__).with_name("research-bundle.py"))
bundle = module_from_spec(spec)
spec.loader.exec_module(bundle)
files = json.loads((root/"runtime.json").read_text(encoding="utf8"))["files"]
bundle.verify(root, files)
assert Path(sys.executable).resolve().is_relative_to(root), "Python was not shipped with the application"
requirements = (Path(__file__).with_name("research-requirements.txt")).read_text(encoding="utf8")
for name, version in re.findall(r"^([\w-]+)==([^\s]+)", requirements, re.M):
    assert importlib.metadata.version(name) == version, (name, version)
for name in ("numpy", "pandas", "sklearn", "scipy", "joblib", "threadpoolctl"):
    module = importlib.import_module(name)
    assert Path(module.__file__).resolve().is_relative_to(root), "external package imported: " + name
import numpy as np
models = 0
for name in files:
    if name.endswith(".pkl"):
        with (root/name).open("rb") as source:
            fitted = pickle.load(source)
        # Every annual frozen sklearn object must load and run with the shipped ABI.
        prediction = np.full(2, fitted[0]) if isinstance(fitted, np.ndarray) else fitted.predict(np.zeros((2, fitted.n_features_in_), dtype=np.float32))
        assert prediction.shape == (2,) and np.all(np.isfinite(prediction)), name
        models += 1
sys.path.insert(0, str(root/"research/research-center-runner"))
import model_runner
model_runner.verify_sources()
model_runner.self_check()
import refresh_market
refresh_market.self_check()
sys.path.insert(0, str(root/"research/ashare-fundamental-2026-10-01"))
import replay_adapter
replay_adapter.verify_sources()
with tempfile.TemporaryDirectory() as home:
    os.environ["USERPROFILE"] = home
    os.environ["HOME"] = home
    sys.path.insert(0, str(root/"research/ashare-models-2026-10-01"))
    import model_study
    assert model_study.DATA.resolve().is_relative_to(root)
    for name in ("numpy", "pandas", "sklearn"):
        assert Path(sys.modules[name].__file__).resolve().is_relative_to(root)
print(json.dumps({"state": "passed", "python": sys.version.split()[0], "frozen_files": len(files), "models_loaded_and_predicted": models, "external_python_or_cache": False}))
