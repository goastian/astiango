#!/usr/bin/env python3
"""Separate live ingestion workers without restarting their supervisors."""
import argparse
import json
import os
from pathlib import Path
import time

ROOT = Path(__file__).resolve().parents[1]


def identity(pid):
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return stat[19]


def apply():
    parents = {}
    for name, folder in [("indexer", "indexer-shared"), ("webgraph", "webgraph-ingestion")]:
        value = json.loads((ROOT / "data" / folder / "status.json").read_text())
        value = value.get("snapshot", value)
        pid = value["pid"]
        if str(value["start_ticks"]) != identity(pid):
            raise ValueError("Supervisor identity changed; no affinity applied")
        parents[name] = {"pid": pid, "start_ticks": identity(pid), "before": sorted(os.sched_getaffinity(pid))}
    cpus = sorted(set.intersection(*(set(row["before"]) for row in parents.values())))
    # Reapplying must retain the original allowed set, not its narrowed intersection.
    record_path = ROOT / "data/control-center/ingestion-cpus.json"
    previous = json.loads(record_path.read_text()) if record_path.exists() else None
    if previous and all(previous["parents"][name]["pid"] == row["pid"] and
                        previous["parents"][name]["start_ticks"] == row["start_ticks"] for name, row in parents.items()):
        for name, row in parents.items():
            row["before"] = previous["parents"][name]["before"]
        cpus = sorted(set.intersection(*(set(row["before"]) for row in parents.values())))
    if len(cpus) < 2:
        return {"applied": False, "reason": "one effective CPU; existing affinity retained"}
    for offset, (name, row) in enumerate(parents.items()):
        target = cpus[offset]
        # The historical pre-exec chooses min(parent affinity). Reserve that
        # minimum separately while leaving the supervisor free on higher CPUs.
        mask = {cpu for cpu in row["before"] if cpu >= target}
        if not mask:
            raise ValueError("No allowed worker CPU")
        os.sched_setaffinity(row["pid"], mask)
        row["after"] = sorted(mask)
        row["worker_cpu"] = target
        row["children"] = []
        children = Path(f'/proc/{row["pid"]}/task/{row["pid"]}/children').read_text().split()
        for child in children:
            try:
                pid = int(child)
                arguments = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
                if not arguments or Path(os.fsdecode(arguments[0])).name != f"astiango-{name}":
                    continue
                start = identity(pid)
                before = sorted(os.sched_getaffinity(pid))
                for task in Path(f"/proc/{pid}/task").iterdir():
                    try:
                        if identity(pid) != start:
                            raise ValueError("Worker identity changed")
                        os.sched_setaffinity(int(task.name), {target})
                    except ProcessLookupError:
                        pass
                row["children"].append({"pid": pid, "start_ticks": start, "before": before, "after": [target]})
            except (FileNotFoundError, ProcessLookupError):
                pass
    value = {"version": 1, "applied": True, "at": time.time(), "parents": parents,
             "workers_per_component": 1, "service_restarts": 0}
    record_path.parent.mkdir(exist_ok=True, parents=True)
    temporary = record_path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(record_path)
    return value


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", required=True)
    parser.parse_args()
    print(json.dumps(apply()))
