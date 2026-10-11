#!/usr/bin/env python3
"""Build and manage only the local crawler and its production dashboard."""

import argparse
import csv
from datetime import datetime, timezone
import fcntl
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path
import random
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "data/crawler-local"
MANIFEST = DATA / "processes.json"
BINARY = ROOT / "target/release/astiango"
CONFIG = ROOT / "configs/crawler/local.toml"
DASHBOARD = ROOT / "tools/crawler-dashboard"
UNIT_SUFFIX = hashlib.sha256(str(ROOT).encode()).hexdigest()[:10]
SEEDS = DATA / "seeds.txt"
SEED_SOURCE = "https://tranco-list.eu/top-1m.csv.zip"


def download(url, limit):
    request = urllib.request.Request(url, headers={"User-Agent": "AstianGO-Crawler-Bootstrap/1.0"})
    with urllib.request.urlopen(request, timeout=60) as response:
        content = response.read(limit + 1)
    if len(content) > limit:
        raise RuntimeError(f"La descarga superó el límite de {limit} bytes: {url}")
    return content


def bootstrap():
    if SEEDS.exists():
        if not SEEDS.read_text().strip():
            raise RuntimeError(f"El archivo de semillas está vacío: {SEEDS}")
        print(f"Semillas existentes: {SEEDS}")
        return
    print("Descargando la lista pública de Tranco para elegir 64 dominios al azar...", flush=True)
    source_id = download("https://tranco-list.eu/top-1m-id", 4096).decode().strip()
    content = download(SEED_SOURCE, 64 * 1024 * 1024)
    confirmed_id = download("https://tranco-list.eu/top-1m-id", 4096).decode().strip()
    if source_id != confirmed_id:
        raise RuntimeError("Tranco cambió de lista durante la descarga; repita bootstrap.")
    rng = random.SystemRandom()
    seeds = []
    population = 0
    with zipfile.ZipFile(io.BytesIO(content)) as archive:
        files = [item for item in archive.infolist() if item.filename.endswith(".csv")]
        if len(files) != 1 or files[0].file_size > 128 * 1024 * 1024:
            raise RuntimeError("La lista de Tranco no contiene un único CSV con tamaño aceptable.")
        with archive.open(files[0]) as source:
            for row in csv.reader(io.TextIOWrapper(source, encoding="utf-8")):
                if len(row) != 2 or not row[0].isdigit():
                    continue
                domain = row[1].strip().lower().rstrip(".")
                if len(domain) > 253 or not re.fullmatch(r"(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}", domain):
                    continue
                try:
                    ipaddress.ip_address(domain)
                    continue
                except ValueError:
                    pass
                population += 1
                # Reservoir sampling gives every domain the same probability.
                if len(seeds) < 64:
                    seeds.append(f"https://{domain}/")
                else:
                    slot = rng.randrange(population)
                    if slot < 64:
                        seeds[slot] = f"https://{domain}/"
    if len(seeds) < 64:
        raise RuntimeError("La fuente no contiene al menos 64 dominios válidos.")
    rng.shuffle(seeds)
    provenance = {
        "source": SEED_SOURCE, "list_id": source_id,
        "retrieved_at": datetime.now(timezone.utc).isoformat(),
        "source_sha256": hashlib.sha256(content).hexdigest(),
        "eligible_domains": population, "sample_size": len(seeds),
        "sampling": "uniform reservoir sampling with SystemRandom", "seeds": seeds,
    }
    DATA.mkdir(parents=True, exist_ok=True)
    (DATA / "seed-provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    temporary = SEEDS.with_suffix(".tmp")
    temporary.write_text("\n".join(seeds) + "\n")
    temporary.replace(SEEDS)
    print(f"64 dominios seleccionados entre {population:,}; lista Tranco {source_id}: {SEEDS}")


def command(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def read_manifest():
    if not MANIFEST.exists():
        return {"root": str(ROOT), "processes": {}}
    state = json.loads(MANIFEST.read_text())
    if state.get("root") != str(ROOT):
        raise RuntimeError(f"El registro de procesos no pertenece a {ROOT}")
    return state


def save_manifest(state):
    temporary = MANIFEST.with_suffix(".tmp")
    temporary.write_text(json.dumps(state, indent=2) + "\n")
    temporary.replace(MANIFEST)


def process_details(pid):
    try:
        proc = Path(f"/proc/{int(pid)}")
        # The command name may contain spaces or parentheses.
        stat = (proc / "stat").read_text().rsplit(")", 1)[1].split()
        if stat[0] == "Z":
            return None
        return {
            "start_ticks": stat[19],
            "command": [part.decode() for part in (proc / "cmdline").read_bytes().split(b"\0") if part],
            "cwd": str((proc / "cwd").resolve()),
        }
    except (OSError, ValueError, IndexError):
        return None


def owned_process(record):
    details = process_details(record.get("pid", -1))
    return bool(details and details["start_ticks"] == record.get("start_ticks")
                and details["command"] == record.get("command") and details["cwd"] == str(ROOT))


def systemd_available():
    if not shutil.which("systemd-run") or not shutil.which("systemctl"):
        return False
    result = subprocess.run(
        ["systemctl", "--user", "show", "--property=Version"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5,
    )
    return result.returncode == 0


def health(port):
    try:
        # Local health checks should never use an HTTP proxy from the shell.
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(f"http://127.0.0.1:{port}/healthz", timeout=2) as response:
            body = json.load(response)
            return response.status == 200 and body.get("service") == "crawler-dashboard"
    except (OSError, ValueError, urllib.error.URLError):
        return False


def launch(name, argv, backend, environment):
    log_path = DATA / f"{name}.log"
    record = {"backend": backend, "command": argv, "log": str(log_path)}
    if backend == "systemd":
        unit = f"astiango-{name}-{UNIT_SUFFIX}"
        record["unit"] = unit
        command([
            "systemd-run", "--user", "--quiet", "--collect", f"--unit={unit}",
            f"--working-directory={ROOT}", "--property=Type=exec",
            "--property=Restart=no", "--property=TimeoutStopSec=90", "--property=SendSIGKILL=no",
            f"--property=Description=AstianGO {name} ({ROOT})",
            f"--property=StandardOutput=append:{log_path}",
            f"--property=StandardError=append:{log_path}",
            *[f"--setenv={key}={value}" for key, value in environment.items()], "--", *argv,
        ], stdout=subprocess.DEVNULL)
        deadline = time.monotonic() + 5
        pid = 0
        while time.monotonic() < deadline:
            result = subprocess.run(
                ["systemctl", "--user", "show", unit, "--property=MainPID", "--value"],
                capture_output=True, text=True,
            )
            pid = int(result.stdout.strip() or "0")
            if pid:
                break
            time.sleep(0.1)
    else:
        with log_path.open("ab", buffering=0) as log:
            process = subprocess.Popen(
                argv, cwd=ROOT, env={**os.environ, **environment},
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
        pid = process.pid
    details = process_details(pid)
    if not details or details["command"] != argv or details["cwd"] != str(ROOT):
        raise RuntimeError(f"{name} no pudo iniciarse. Consulte {log_path}")
    record.update(pid=pid, start_ticks=details["start_ticks"])
    return record


def stop_one(name, record):
    if not owned_process(record):
        print(f"{name}: detenido; no se envió ninguna señal a un PID ajeno.")
        return
    if record["backend"] == "systemd":
        command(["systemctl", "--user", "stop", record["unit"]], timeout=100)
    else:
        # A pidfd keeps the signal bound to this process even if its PID is reused.
        descriptor = os.pidfd_open(record["pid"])
        try:
            if owned_process(record):
                signal.pidfd_send_signal(descriptor, signal.SIGTERM)
        finally:
            os.close(descriptor)
        deadline = time.monotonic() + 90
        while owned_process(record) and time.monotonic() < deadline:
            time.sleep(0.25)
        if owned_process(record):
            raise RuntimeError(f"{name} todavía está drenando tras 90 s; no se envió SIGKILL.")
    print(f"{name}: detenido.")


def start(state, args):
    if not BINARY.is_file() or not os.access(BINARY, os.X_OK):
        raise RuntimeError("Falta el binario release. Ejecute scripts/crawler-local.py build")
    if not (DASHBOARD / "dist/index.html").is_file():
        raise RuntimeError("Falta el dashboard compilado. Ejecute scripts/crawler-local.py build")
    node = shutil.which("node")
    if not node:
        raise RuntimeError("Se necesita Node.js para servir el dashboard.")
    command([str(BINARY), "crawler", "local", "--help"], stdout=subprocess.DEVNULL)
    bootstrap()
    backend = args.backend
    if backend == "auto":
        backend = "systemd" if systemd_available() else "detached"
    elif backend == "systemd" and not systemd_available():
        raise RuntimeError("El servicio systemd del usuario no está accesible.")
    running_dashboard = state["processes"].get("dashboard", {})
    if owned_process(running_dashboard) and state.get("port") != args.port:
        raise RuntimeError("El dashboard ya usa otro puerto. Detenga el despliegue antes de cambiarlo.")
    state["port"] = args.port
    specs = [
        ("dashboard", [node, str(DASHBOARD / "server.mjs")], {
            "HOST": "127.0.0.1", "PORT": str(args.port), "NODE_ENV": "production",
            "CRAWLER_STATE_PATH": str(DATA / "status.json"),
        }),
        ("crawler", [str(BINARY), "crawler", "local", str(CONFIG)], {
            "RUST_LOG": os.environ.get("RUST_LOG", "astiango=info"),
        }),
    ]
    for name, argv, environment in specs:
        record = state["processes"].get(name, {})
        if owned_process(record):
            print(f"{name}: ya está activo (PID {record['pid']}).")
        else:
            record = launch(name, argv, backend, environment)
            state["processes"][name] = record
            save_manifest(state)
            print(f"{name}: iniciado (PID {record['pid']}, {backend}).")
        if name == "dashboard":
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if not owned_process(record):
                    raise RuntimeError(f"El dashboard terminó. Consulte {record['log']}")
                if health(args.port):
                    break
                time.sleep(0.2)
            else:
                raise RuntimeError(f"El dashboard no respondió a /healthz. Consulte {record['log']}")
        else:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if not owned_process(record):
                    raise RuntimeError(f"El crawler terminó al iniciar. Consulte {record['log']}")
                try:
                    snapshot = json.loads((DATA / "status.json").read_text())
                    if snapshot.get("pid") == record["pid"]:
                        break
                except (OSError, ValueError):
                    pass
                time.sleep(0.2)
            else:
                raise RuntimeError("El crawler está vivo, pero no publicó un snapshot de esta ejecución en 15 s.")
    print(f"Dashboard: http://127.0.0.1:{args.port}")
    print(f"Datos y registros: {DATA}")


def status(state):
    complete = True
    for name in ("crawler", "dashboard"):
        record = state["processes"].get(name, {})
        alive = owned_process(record)
        complete = complete and alive
        print(f"{name}: {'activo' if alive else 'detenido'}" + (f" (PID {record['pid']}, {record['backend']})" if alive else ""))
    port = state.get("port", 3100)
    dashboard_health = health(port)
    print(f"Dashboard: http://127.0.0.1:{port} (/healthz: {'OK' if dashboard_health else 'sin respuesta'})")
    snapshot_path = DATA / "status.json"
    if snapshot_path.exists():
        snapshot = json.loads(snapshot_path.read_text())
        age = max(0, int(time.time() - snapshot_path.stat().st_mtime))
        print(f"Último estado: {snapshot.get('phase', 'desconocido')}; snapshot hace {age} s; {snapshot_path}")
    return 0 if complete and dashboard_health else 1


def build():
    environment = {**os.environ, "CFLAGS": os.environ.get("CFLAGS", "-std=gnu17"),
                   "CARGO_PROFILE_RELEASE_STRIP": "symbols"}
    command(["cargo", "build", "--locked", "--release", "--no-default-features",
             "-p", "astiango", "--bin", "astiango"], env=environment)
    command(["npm", "--prefix", str(DASHBOARD), "ci"])
    command(["npm", "--prefix", str(DASHBOARD), "run", "build"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "bootstrap", "start", "status", "stop", "restart"))
    parser.add_argument("--backend", choices=("auto", "systemd", "detached"), default="auto")
    parser.add_argument("--port", type=int, default=3100)
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("--port debe estar entre 1 y 65535")
    if args.action == "build":
        build()
        return 0
    DATA.mkdir(parents=True, exist_ok=True)
    with (DATA / "runner.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.action == "bootstrap":
            bootstrap()
            return 0
        state = read_manifest()
        if args.action == "status":
            return status(state)
        if args.action in ("stop", "restart"):
            # Drain the writer before stopping the dashboard that observes it.
            for name in ("crawler", "dashboard"):
                if name in state["processes"]:
                    stop_one(name, state["processes"][name])
                    del state["processes"][name]
                    save_manifest(state)
        if args.action in ("start", "restart"):
            start(state, args)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)
