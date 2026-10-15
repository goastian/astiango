#!/usr/bin/env python3
"""Build both ingestion artifacts using their pinned per-component builders."""
import argparse
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-only", action="store_true")
    parser.add_argument("--verify-rebuild", action="store_true")
    args = parser.parse_args()
    flags = (["--package-only"] if args.package_only else []) + (["--verify-rebuild"] if args.verify_rebuild else [])
    for component in ["indexer", "webgraph"]:
        subprocess.run([sys.executable, str(ROOT / "scripts" / f"build-{component}.py"), *flags], cwd=ROOT, check=True)

if __name__ == "__main__":
    main()
