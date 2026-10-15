#!/usr/bin/env python3
"""Immutable launch policy for already-running ingestion supervisors.

Source ingestion stays immediate. Generation publication observes its configured
cadence; a skipped graph publication is an explicit deferred outcome (exit 75).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time
import tomllib

_CHECKOUT_ROOT = None
ROOT = Path(_CHECKOUT_ROOT) if _CHECKOUT_ROOT else Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def atomic(path, data):
    temp = path.with_name(path.name + ".runtime-tmp")
    with temp.open("wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    temp.replace(path)


def package(component, binary, output, period):
    policy = {"version": 1, "component": component, "native_binary": str(binary.resolve()),
              "native_sha256": sha(binary), "output_path": str(output.resolve()), "period_seconds": period}
    # Embed the checkout root so the packaged launcher need not infer it from data/.
    source = Path(__file__).read_bytes().replace(b'_CHECKOUT_ROOT = None',
        ("_CHECKOUT_ROOT = " + json.dumps(str(ROOT))).encode(), 1)
    encoded = json.dumps(policy, sort_keys=True, separators=(",", ":")).encode()
    identity = hashlib.sha256(source + encoded).hexdigest()
    directory = ROOT / "data/artifacts/ingestion-runtime" / identity / component
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / f"astiango-{component}-runtime"
    for path, content in [(target, source), (directory / "policy.json", encoded)]:
        if path.exists():
            if path.read_bytes() != content:
                raise ValueError("Existing launch artifact changed")
        else:
            atomic(path, content)
    target.chmod(0o755)
    return target


def install():
    changed = []
    for component, relative, suffix in [("indexer", "configs/indexer/shared-local.toml", "index"),
                                        ("webgraph", "configs/webgraph/ingestion-local.toml", "graph")]:
        profile = ROOT / relative
        text = profile.read_text()
        config = tomllib.loads(text)
        artifact = json.loads((ROOT / "data/artifacts" / component / "latest.json").read_text())
        binary = ROOT / artifact["binary"]
        if sha(binary) != artifact["sha256"]:
            raise ValueError("Native artifact checksum mismatch")
        target = package(component, binary, ROOT / config["state_directory"] / suffix, config["publication_seconds"])
        updated, count = re.subn(r'(?m)^binary\s*=.*$', "binary = " + json.dumps(str(target.relative_to(ROOT))), text)
        if count != 1:
            raise ValueError("Expected exactly one binary profile entry")
        atomic(profile, updated.encode())
        changed.append({"component": component, "launcher": str(target.relative_to(ROOT)),
                        "native_sha256": artifact["sha256"], "publication_seconds": config["publication_seconds"]})
    audit = ROOT / "data/control-center/ingestion-runtime.json"
    audit.parent.mkdir(parents=True, exist_ok=True)
    atomic(audit, json.dumps({"version": 1, "at": time.time(), "components": changed,
                             "service_restarts": 0}).encode())
    print(json.dumps(changed))


def launch():
    directory = Path(__file__).resolve().parent
    policy_bytes = (directory / "policy.json").read_bytes()
    policy = json.loads(policy_bytes)
    fingerprint = hashlib.sha256(Path(__file__).read_bytes() + policy_bytes).hexdigest()
    if directory.parent.name != fingerprint:
        raise ValueError("Launch policy fingerprint mismatch")
    binary = Path(policy["native_binary"])
    if sha(binary) != policy["native_sha256"]:
        raise ValueError("Native binary checksum mismatch")
    args = sys.argv[1:]
    if args == ["--build-info"]:
        os.execv(binary, [str(binary), *args])
    if not args:
        raise ValueError("A native configuration path is required")
    path = Path(args[0]).resolve()
    if not path.is_relative_to(ROOT) or path.stat().st_size > 256 * 1024:
        raise ValueError("Native job configuration outside budget")
    text = path.read_text()
    config = tomllib.loads(text)
    output = Path(config["output_path"]).resolve()
    if str(output) != policy["output_path"]:
        raise ValueError("Launch policy output path mismatch")
    component = policy["component"]
    selector_path = output / ("graph-selector.json" if component == "webgraph" else "generation-selector.json")
    due = not selector_path.exists() or time.time() - selector_path.stat().st_mtime >= policy["period_seconds"]
    if component == "webgraph" and args[1:] == ["--prepare-generation"] and not due:
        selected = json.loads(selector_path.read_text())
        previous = output / "generations" / selected["active"] / "config.json"
        old = json.loads(previous.read_text())
        contract = ["centrality_enabled", "centrality_scope", "centrality_max_nodes", "centrality_max_edges",
                    "centrality_memory_bytes", "centrality_max_iterations", "centrality_timeout_ms"]
        if all(old.get(key) == config.get(key) for key in contract):
            not_before = int(selector_path.stat().st_mtime + policy["period_seconds"])
            print(f"deferred: publication_not_due until={not_before}; previous generation retained", file=sys.stderr)
            return 75
    if component == "indexer" and len(args) == 1:
        if "ingestion" not in config:
            raise ValueError("Cadence policy requires shared ingestion")
        # Parent supervisors can request publication on catch-up. This explicit
        # policy preserves the configured period instead of preparing every input.
        parts = text.split("[ingestion]", 1)
        replacement, count = re.subn(r'(?m)^publish_generations\s*=\s*(?:true|false)\s*$',
                                    "publish_generations=" + str(bool(due)).lower(), parts[1], count=1)
        if count != 1:
            raise ValueError("Missing explicit generation publication policy")
        runtime_path = output.parent / "runtime-job.toml"
        atomic(runtime_path, (parts[0] + "[ingestion]" + replacement).encode())
        args[0] = str(runtime_path)
    os.execv(binary, [str(binary), *args])


if __name__ == "__main__":
    if "--install" in sys.argv:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--install", action="store_true", required=True)
        parser.parse_args()
        ROOT = Path(__file__).resolve().parents[1]
        install()
    else:
        raise SystemExit(launch())
