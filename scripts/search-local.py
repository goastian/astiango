#!/usr/bin/env python3
"""Start or inspect the local Search/API without stopping ingestion."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
UNIT = "astiango-search-api-" + hashlib.sha256(str(ROOT).encode()).hexdigest()[:10] + ".service"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "status", "restart"])
    args = parser.parse_args()
    if args.action == "status":
        subprocess.run(["systemctl", "--user", "show", UNIT, "--property=ActiveState,MainPID,NRestarts"], check=True)
        return
    subprocess.run([sys.executable, str(ROOT / "scripts/search-api.py"), "--check-config"], check=True)
    active = subprocess.run(["systemctl", "--user", "is-active", "--quiet", UNIT]).returncode == 0
    if active:
        if args.action == "restart":
            subprocess.run(["systemctl", "--user", "restart", UNIT], check=True)
    else:
        subprocess.run(["systemctl", "--user", "reset-failed", UNIT], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["systemd-run", "--user", "--collect", f"--unit={UNIT}", f"--working-directory={ROOT}",
                        "--property=Restart=on-failure", "--property=RestartSec=5", "--property=StartLimitBurst=5",
                        "--property=StartLimitIntervalSec=300", "--property=KillMode=mixed", "--property=TimeoutStopSec=10",
                        "--property=CPUQuota=100%", "--property=MemoryMax=1G", "--property=TasksMax=64",
                        "/usr/bin/python3", str(ROOT / "scripts/search-api.py"), "--config", str(ROOT / "configs/search/local.toml")], check=True)
    print(json.dumps({"service": UNIT, "url": "http://127.0.0.1:3210/v1/web/search", "dashboard": "http://127.0.0.1:3101/#/search/overview", "payments": "planned_only"}))


if __name__ == "__main__":
    main()
