#!/usr/bin/env python3
"""Verify a full C01 report against retained artifacts and the current checkout, read-only."""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path
import statistics
import sys

# Importing the runner below must not create scripts/__pycache__.
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('report', nargs='?', type=Path, default=ROOT / 'data/crawler-bench/report.json',
                    help='report.json to verify; defaults to data/crawler-bench/report.json')
REPORT = parser.parse_args().report.resolve()
errors = []
checks = 0

def require(condition, message):
    global checks
    checks += 1
    if not condition:
        errors.append(message)

def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()

def stats(values):
    median = statistics.median(values)
    return {'median': median, 'min': min(values), 'max': max(values),
            'mad': statistics.median(abs(value - median) for value in values)}

report = json.loads(REPORT.read_text())
if not report.get('acceptance', {}).get('passed'):
    print(f"Informe incompleto: {len(report.get('scenarios', []))}/18 grupos; aceptación no aprobada.")
    sys.exit(2)

configuration = report['configuration']
expected_groups = {(name, origins, observer)
                   for name in ('balanced', 'slow', 'adversarial')
                   for origins in (2, 32, 128) for observer in (False, True)}
actual_groups = [(row['name'], row['origins'], row['observer']) for row in report['scenarios']]
require(len(actual_groups) == 18 and set(actual_groups) == expected_groups,
        'Expected each of the 18 matrix cells exactly once')
require(configuration['repetitions'] == 3, 'Expected exactly three published repetitions')
require(all(report['acceptance'].values()), 'A final acceptance condition is false')
require(configuration['min_crawl_delay_ms'] == 5 and configuration['max_crawl_delay_ms'] == 50,
        'Courtesy limits changed from declared baseline')
require(configuration['server']['large_bytes'] == 2 * 1024 * 1024 and
        configuration['server']['oversized_bytes'] == 33 * 1024 * 1024,
        'Controlled body limits changed from declared baseline')

manifest_path = ROOT / 'tests/crawler-bench/manifest.json'
manifest = json.loads(manifest_path.read_text())
require(digest(manifest_path) == report['corpus']['sha256'], 'Manifest hash differs from report')
require(manifest['fixtures'] == report['corpus']['files'], 'Published corpus metadata differs from manifest')
for fixture in manifest['fixtures']:
    path = ROOT / fixture['path']
    require(path.stat().st_size == fixture['bytes'] and digest(path) == fixture['sha256'],
            f"Conserved corpus bytes changed: {fixture['id']}")

spec = importlib.util.spec_from_file_location('c01_verification_runner', ROOT / 'scripts/crawler-bench.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
require(configuration['server'] == runner.SERVER_CONFIG, 'Server settings differ from measured source')
require(configuration['scenario_paths'] == runner.scenario_paths({fixture['id']: b'' for fixture in manifest['fixtures']}),
        'Scenario composition differs from committed fixture matrix')
# Recompute the same source set directly; avoid git status, whose optional index
# refresh may write .git/index even when only asking for status.
source_paths = {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock',
                ROOT / 'scripts/crawler-bench.py', manifest_path, ROOT / 'crates/core/Cargo.toml'}
for relative in ('crates/core/src', 'crates/core/examples', 'crates/robotstxt/src', 'crates/kuchiki/src'):
    source_paths.update(path for path in (ROOT / relative).rglob('*') if path.is_file())
source_hash = hashlib.sha256()
for path in sorted(source_paths):
    source_hash.update(str(path.relative_to(ROOT)).encode() + b'\0' + path.read_bytes() + b'\0')
require(source_hash.hexdigest() == report['code']['source_sha256'],
        'Current relevant source hash differs from measured source hash')
require(digest(ROOT / 'target/release/examples/crawler_bench') == configuration['binary_sha256'],
        'Current benchmark binary hash differs from measured binary hash')

common_configs = {}
run_count = 0
request_total = 0
saved_total = 0
pressure_failures = []
for row in report['scenarios']:
    name, origins, observer = row['name'], row['origins'], row['observer']
    expected_hosts = {f'origin-{index:03}.test' for index in range(origins)}
    initial_paths = set(configuration['scenario_paths'][name]) - {'/blocked'}
    allowed_paths = initial_paths | {'/robots.txt'}
    require(len(row['runs']) == 3 and {run['repetition'] for run in row['runs']} == {1, 2, 3},
            f'{name}/{origins}/{observer}: incomplete or duplicate repetitions')
    for run in row['runs']:
        run_count += 1
        label = f"{name}-{origins}-{'on' if observer else 'off'}-{run['repetition']}"
        directory = REPORT.parent / 'runs' / label
        config = json.loads((directory / 'config.json').read_text())
        measurement = json.loads((directory / 'measurement.json').read_text())
        require(measurement == run, f'{label}: embedded evidence differs from measurement.json')
        require(config['observer'] == observer, f'{label}: observer config mismatch')
        require(Path(config['output_dir']) == directory, f'{label}: output is not isolated in run directory')
        require({origin['host'] for origin in config['origins']} == expected_hosts and
                len(config['origins']) == origins, f'{label}: config origin set mismatch')
        ports = {origin['port'] for origin in config['origins']}
        require(len(ports) == 1 and all(0 < port < 65536 for port in ports),
                f'{label}: origins do not share the controlled ephemeral listener')
        require(config['paths'] == configuration['scenario_paths'][name], f'{label}: scenario paths changed')
        normalized = {key: value for key, value in config.items()
                      if key not in ('origins', 'output_dir', 'observer', 'paths')}
        for key, value in normalized.items():
            require(configuration.get(key) == value, f'{label}: unreported limit/config change {key}')
        reference = common_configs.setdefault(name, normalized)
        require(normalized == reference, f'{label}: unfair config difference across observer pairs')

        server = run['server']
        server_origins = Counter()
        server_paths = Counter()
        for origin_path, count in server['by_origin_path'].items():
            host, separator, tail = origin_path.partition('/')
            path = '/' + tail
            require(separator and host in expected_hosts, f'{label}: unexpected host in ledger: {host}')
            require(path in allowed_paths, f'{label}: unexpected path in ledger: {path}')
            require(isinstance(count, int) and count > 0, f'{label}: invalid ledger count')
            server_origins[host] += count
            server_paths[path] += count
        require(set(server_origins) == expected_hosts, f'{label}: did not visit every required origin')
        require(dict(server_origins) == run['requests_by_origin'], f'{label}: per-origin counters disagree')
        require(dict(server_paths) == server['by_path'] == run['requests_by_path'],
                f'{label}: per-path counters disagree')
        require(sum(server_origins.values()) == sum(server['statuses'].values()) ==
                server['requests'] == run['client_requests'], f'{label}: total counters disagree')
        require(server['by_path'].get('/blocked', 0) == 0, f'{label}: requested forbidden robots path')
        require(server['by_path'].get('/escape', 0) == 0, f'{label}: followed external escape redirect')
        for host in expected_hosts:
            require(server['by_origin_path'].get(host + '/robots.txt', 0) >= 1,
                    f'{label}: no robots request for {host}')
            for path in initial_paths:
                require(server['by_origin_path'].get(host + path, 0) >= 1,
                        f'{label}: missing initial request for {host}{path}')
        expected_saved = {'balanced': 8, 'slow': 12}.get(name)
        if expected_saved:
            # These scenarios schedule each page once and have no deliberate
            # error/redirect endpoints. Every missing page must therefore have
            # exactly one recorded final process_url error. Stress-induced loss
            # is evidence, not permission to relax the shared timeout or limits.
            recorded_errors = run.get('errors', {})
            valid_errors = isinstance(recorded_errors, dict) and all(
                isinstance(count, int) and not isinstance(count, bool) and count >= 0
                for count in recorded_errors.values()
            )
            require(valid_errors, f'{label}: invalid final error counters')
            final_errors = sum(recorded_errors.values()) if valid_errors else 0
            require(run['saved_pages'] + final_errors == expected_saved * origins,
                    f'{label}: expected {expected_saved * origins} pages, got '
                    f'{run["saved_pages"]} saved + {final_errors} recorded errors')
            if final_errors:
                pressure_failures.append(
                    f'{label}: {run["saved_pages"]}/{expected_saved * origins} páginas guardadas; '
                    f'errores finales={recorded_errors}'
                )
        require(len(list((directory / 'warc').glob('*.warc.gz'))) == run['warc_files'],
                f'{label}: WARC artifacts missing or count differs from report')
        require(run['warc_files'] == run['saved_pages'] and run['warc_verified'],
                f'{label}: saved WARC count/verification mismatch')
        require(all(run[key] for key in ('requests_match', 'origins_verified', 'robots_respected')),
                f'{label}: a per-run reconciliation gate failed')
        request_total += run['client_requests']
        saved_total += run['saved_pages']

    for field, key in [('duration_ms', 'duration_ms'), ('cpu_seconds', 'cpu_seconds'),
                       ('rss_peak_bytes', 'rss_peak_bytes'), ('requests', 'client_requests'),
                       ('saved_pages', 'saved_pages'), ('saved_bytes', 'saved_bytes'),
                       ('process_wall_ms', 'process_wall_ms')]:
        require(row['metrics'][field] == stats([run[key] for run in row['runs']]),
                f'{name}/{origins}/{observer}: incorrect summary dispersion for {field}')
    for stage, value in row['metrics']['stages_ms'].items():
        require(value == stats([run['stages_ms'].get(stage, 0) for run in row['runs']]),
                f'{name}/{origins}/{observer}: incorrect stage dispersion {stage}')
    for resource in ('read_bytes', 'write_bytes', 'syscr', 'syscw', 'rchar', 'wchar'):
        values = []
        for run in row['runs']:
            counters = run['resources_io'] if 'resources_io' in run else run['resources']['sampled_peak']
            values.append(counters.get(resource, 0))
        require(row['metrics'][resource] == stats(values),
                f'{name}/{origins}/{observer}: incorrect IO dispersion {resource}')
    for resource in ('fds', 'threads'):
        values = [run['resources']['sampled_peak'].get(resource, 0) for run in row['runs']]
        require(row['metrics'][f'{resource}_peak'] == stats(values),
                f'{name}/{origins}/{observer}: incorrect process peak dispersion {resource}')

expected_comparisons = {(name, origins) for name, origins, _observer in expected_groups}
actual_comparisons = [(comparison['name'], comparison['origins']) for comparison in report['comparisons']]
require(len(actual_comparisons) == 9 and set(actual_comparisons) == expected_comparisons,
        'Expected each of the nine observer comparisons exactly once')
rows_by_group = {(row['name'], row['origins'], row['observer']): row for row in report['scenarios']}
for comparison in report['comparisons']:
    name, origins = comparison['name'], comparison['origins']
    off = rows_by_group.get((name, origins, False))
    on = rows_by_group.get((name, origins, True))
    require(off is not None and on is not None, f'{name}/{origins}: comparison has no matching groups')
    if off is None or on is None:
        continue
    off_runs = {run['repetition']: run for run in off['runs']}
    on_runs = {run['repetition']: run for run in on['runs']}
    require(set(off_runs) == set(on_runs) == {1, 2, 3},
            f'{name}/{origins}: comparison does not contain three original observer pairs')
    if set(off_runs) != {1, 2, 3} or set(on_runs) != {1, 2, 3}:
        continue
    pairs = [(off_runs[repetition], on_runs[repetition]) for repetition in (1, 2, 3)]
    requests_equal = all(off_run['requests_by_path'] == on_run['requests_by_path']
                         for off_run, on_run in pairs)
    saved_pages_equal = all(off_run['saved_pages'] == on_run['saved_pages']
                           for off_run, on_run in pairs)
    require(comparison['requests_equal'] is requests_equal,
            f'{name}/{origins}: requests_equal misrepresents the original pairs')
    require(comparison['saved_pages_equal'] is saved_pages_equal,
            f'{name}/{origins}: saved_pages_equal misrepresents the original pairs')
    off_median = statistics.median(run['duration_ms'] for run in off_runs.values())
    on_median = statistics.median(run['duration_ms'] for run in on_runs.values())
    require(off_median > 0, f'{name}/{origins}: invalid unobserved duration denominator')
    if off_median > 0:
        difference_pct = 100 * (on_median / off_median - 1)
        require(comparison['duration_overhead_pct'] == difference_pct,
                f'{name}/{origins}: duration difference does not match original run medians')

require(run_count == 54, f'Expected 54 runs, found {run_count}')
if errors:
    print(f'C01 NO verificado: {len(errors)} discrepancias en {checks} comprobaciones.')
    for error in errors[:30]:
        print(f'- {error}')
    if len(errors) > 30:
        print(f'- Otras {len(errors) - 30} discrepancias omitidas.')
else:
    print(f'C01 verificado: {run_count} ejecuciones / {len(actual_groups)} grupos; '
          f'{request_total} peticiones contrastadas; {saved_total} WARC; '
          f'{checks} comprobaciones aprobadas.')
if pressure_failures:
    print('Fallos bajo presión contabilizados; estas repeticiones no tienen igual trabajo exitoso:')
    for failure in pressure_failures:
        print(f'- {failure}')
sys.exit(bool(errors))
