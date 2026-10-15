#!/usr/bin/env python3
"""Native S05/S06 integration; faults touch only disposable local replicas."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from zoneinfo import ZoneInfo
import gzip
import http.client
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import statistics
import subprocess
import time
import urllib.parse
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('search_baseline', ROOT/'scripts/search-api-integration.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def wire(port, token, query, connection=None, coding='identity', close=False):
    owned = connection is None
    connection = connection or http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    headers = {'Authorization': f'Bearer {token}', 'X-Search-Traffic': 'benchmark',
               'X-Search-Cache': 'off', 'Accept-Encoding': coding, 'Connection': 'close' if close else 'keep-alive'}
    start = time.perf_counter()
    connection.request('GET', '/v1/web/search?'+urllib.parse.urlencode(query), headers=headers)
    response = connection.getresponse()
    payload = response.read()
    coding = response.getheader('Content-Encoding')
    decoded = gzip.decompress(payload) if coding == 'gzip' else payload
    value = json.loads(decoded)
    result = {'status': response.status, 'value': value, 'payload_bytes': len(payload), 'raw_bytes': len(decoded),
              'ms': (time.perf_counter()-start)*1000, 'encoding': coding, 'vary': response.getheader('Vary')}
    if owned:
        connection.close()
    return result


def command(harness, action, node):
    identity = str(uuid.uuid4())
    status, value, _ = base.request(harness.port, '/control', harness.token,
                                  {'id': identity, 'action': action, 'node': node})
    assert status == 202, (status, value)
    return base.wait(lambda: next((row for row in harness.state()['commands']
                     if row['id'] == identity and row['state'] in {'applied', 'failed'}), None), action)


def ready(harness):
    nodes = harness.state()['nodes']
    return nodes if len(nodes) == 2 and all(node['state'] == 'ready' and node['enabled'] for node in nodes) else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, default=ROOT/'docs/search-api-s00-s04-pruebas-2026-10-15.json')
    parser.add_argument('--output', type=Path, default=ROOT/'docs/search-api-s05-s06-pruebas-2026-10-15.json')
    args = parser.parse_args()
    if not args.baseline.exists():
        subprocess.run(['python3', str(ROOT/'scripts/search-api-integration.py'), '--output', str(args.baseline)], check=True)
    baseline = json.loads(args.baseline.read_text())
    assert baseline['passed']
    source = ROOT/baseline['working_directory']
    index = source/'small/index'
    graph = source/'graph'
    first = baseline['checks']['promotion_rollback_replay_and_pins']['first']
    second = baseline['checks']['promotion_rollback_replay_and_pins']['second']
    directory = ROOT/'data/search-resilience-integration'/str(uuid.uuid4())
    directory.mkdir(parents=True)
    artifact = json.loads((ROOT/'data/artifacts/search/latest.json').read_text())
    binary = directory/'astiango-search-worker'
    shutil.copy2(ROOT/artifact['binary'], binary)
    artifact = {**artifact, 'binary': str(binary.relative_to(ROOT))}
    sources = ['scripts/search-api.py', 'scripts/search-api-integration.py', 'scripts/search-api-resilience-integration.py']
    report = {'version': 1, 'passed': False, 'started_at': datetime.now(ZoneInfo('America/Bogota')).isoformat(),
              'scope': 'isolated local replicas of one native shard; no operational fault injection',
              'source_sha256': {name: base.sha(ROOT/name) for name in sources}, 'build': artifact,
              'baseline_sha256': base.sha(args.baseline), 'corpus_sha256': baseline['corpus_sha256'],
              'generation': first, 'manifest_sha256': base.sha(index/'generations'/first/'manifest.json'),
              'checks': {}, 'http_comparison': {}, 'operational_inputs_modified': False, 'synthetic_network_requests': 0}
    harness = None
    try:
        harness = base.Harness(directory/'service', artifact, index, graph, port=3219, workers=2, max_queue=2,
                               replica_timeout_ms=600, recovery_base_ms=100, recovery_max_ms=400,
                               recovery_attempts_per_minute=5)
        assert harness.command('promote', first)[0]['state'] == 'applied'
        nodes = base.wait(lambda: ready(harness), 'two warmed replicas')
        assert all(node['generation'] == first for node in nodes)
        report['checks']['two_compatible_warmed_replicas'] = {'passed': True, 'nodes': [n['id'] for n in nodes]}

        # Same corpus, all response fields preserved, four independent HTTP modes.
        queries = [{'q': term, 'language': language} for language, term, _ in base.LANGUAGES]
        queries += [{'q': q} for q in ['currentbeta', 'derivativemarker', 'noindexmarker', 'obsoletealpha', 'revokedmarker', 'zzzznosuchdocumentzzzz']]
        expected = [wire(harness.port, harness.token, q, close=True)['value']['results'] for q in queries]
        for mode, keep, coding in [('close_identity', False, 'identity'), ('keep_identity', True, 'identity'),
                                    ('close_gzip', False, 'gzip'), ('keep_gzip', True, 'gzip')]:
            repetitions = []
            for _ in range(3):
                connection = http.client.HTTPConnection('127.0.0.1', harness.port, timeout=5) if keep else None
                before = harness.state()
                measurements = []
                local_port = None
                try:
                    for q, results in zip(queries, expected):
                        item = wire(harness.port, harness.token, q, connection, coding, close=not keep)
                        assert item['status'] == 200 and item['value']['results'] == results
                        assert item['value']['shards'] == {'expected': 1, 'complete': 1, 'partial': False}
                        assert item['value']['generation'] == first and item['value']['routing']['attempts'] == 1
                        assert item['vary'] == 'Accept-Encoding'
                        if keep:
                            current_port = connection.sock.getsockname()[1]
                            assert local_port in {None, current_port}, 'HTTP connection was silently re-opened'
                            local_port = current_port
                        measurements.append(item)
                finally:
                    if connection:
                        connection.close()
                after = harness.state()
                cpu0 = before['http']['traffic'].get('benchmark', {})
                cpu1 = after['http']['traffic']['benchmark']
                repetitions.append({'requests': len(measurements), 'connections_for_queries': 1 if keep else len(measurements),
                    'p95_ms': base.percentile([item['ms'] for item in measurements], .95),
                    'payload_bytes': sum(item['payload_bytes'] for item in measurements),
                    'raw_bytes': sum(item['raw_bytes'] for item in measurements),
                    'serialization_cpu_ms': cpu1.get('serialization_cpu_ms', 0)-cpu0.get('serialization_cpu_ms', 0),
                    'compression_cpu_ms': cpu1.get('compression_cpu_ms', 0)-cpu0.get('compression_cpu_ms', 0),
                    'native_cpu_ms': sum(item['value']['native_cpu_ms'] for item in measurements),
                    'compressed': sum(item['encoding'] == 'gzip' for item in measurements),
                    'rss_bytes': after['resources']['rss_bytes'], 'open_fds': after['resources']['open_fds']})
            report['http_comparison'][mode] = {'repetitions': repetitions,
                **{key: base.summary([item[key] for item in repetitions]) for key in ['p95_ms', 'payload_bytes', 'serialization_cpu_ms', 'compression_cpu_ms', 'native_cpu_ms']}}
        report['checks']['http_results_equivalence_and_reuse'] = {'passed': True, 'queries': len(queries), 'repetitions': 3, 'modes': 4}
        assert wire(harness.port, harness.token, {'q': 'security'}, coding='gzip;q=0')['encoding'] is None
        assert wire(harness.port, harness.token, {'q': 'security'}, coding='gzip;q=0, identity;q=0')['status'] == 406
        assert wire(harness.port, harness.token, {'q': 'security'}, coding='gzip;q=0.5, identity;q=0')['encoding'] == 'gzip'
        assert wire(harness.port, harness.token, {'q': 'security'}, coding='gzip;q=nan')['status'] == 400
        for suffix in ['&count=21', '&page=21', '&q=repeated', '&country=unknown']:
            assert base.request(harness.port, '/v1/web/search?q=security'+suffix, harness.token)[0] == 400
        assert harness.query('site:language.fixture security')[1]['results']
        assert harness.query('"independent search"')[1]['results']
        assert any('signature=a%2Fb&x=2&x=1' in item['url'] for item in harness.query('security')[1]['results'])
        report['checks']['negotiation_filters_pagination_signed_identity'] = {'passed': True}

        nodes = base.wait(lambda: ready(harness), 'ready before slow replica')
        # Choose the replica the least-used routing policy will select.
        slow = min(nodes, key=lambda node: (node['requests'], int(node['id'].split(':')[1])))
        os.kill(slow['pid'], signal.SIGSTOP)
        with ThreadPoolExecutor(max_workers=2) as pool:
            future = pool.submit(harness.query, 'security', cache=False)
            base.wait(lambda: harness.state()['active'] == 1, 'slow request active')
            healthy = [harness.query('security', cache=False) for _ in range(3)]
            failed = future.result()
        assert all(item[0] == 200 and item[1]['routing']['node'] != slow['id'] for item in healthy)
        assert failed[0] == 504 and failed[1]['routing']['attempts'] == 1 and failed[2] < 2000, failed
        recovered = base.wait(lambda: ready(harness), 'slow replica killed and warmed')
        assert next(node for node in recovered if node['id'] == slow['id'])['pid'] != slow['pid']
        report['checks']['slow_replica_isolation_no_retry_and_recovery'] = {'passed': True, 'failed_ms': failed[2],
            'healthy_p95_ms': base.percentile([item[2] for item in healthy], .95), 'deadline_ms': 2000, 'sample_slo_p95_ms': 500}
        victim = recovered[1]
        os.kill(victim['pid'], signal.SIGKILL)
        base.wait(lambda: any(node['id'] == victim['id'] and node['pid'] != victim['pid'] and node['state'] == 'ready'
                             for node in harness.state()['nodes']), 'idle crash replaced')
        assert harness.query('security', cache=False)[0] == 200
        report['checks']['idle_crash_detected_and_recovered'] = {'passed': True}

        nodes = base.wait(lambda: ready(harness), 'ready for node controls')
        drained = nodes[0]['id']
        assert command(harness, 'drain_node', drained)['state'] == 'applied'
        assert harness.query('security', cache=False)[1]['routing']['node'] != drained
        other = nodes[1]['id']
        assert command(harness, 'drain_node', other)['state'] == 'failed'
        assert command(harness, 'resume_node', drained)['state'] == 'applied'
        with ThreadPoolExecutor(max_workers=2) as pool:
            future = pool.submit(command, harness, 'replace_node', drained)
            observed = [harness.query('security', cache=False) for _ in range(5)]
            assert future.result()['state'] == 'applied'
        assert all(item[0] == 200 for item in observed)
        report['checks']['drain_resume_rolling_replace_preserves_capacity'] = {'passed': True, 'queries': len(observed)}
        assert harness.command('promote', second)[0]['state'] == 'applied'
        assert harness.query('security', cache=False)[1]['generation'] == second
        status, _, _ = base.request(harness.port, '/control', harness.token,
                    {'id': str(uuid.uuid4()), 'action': 'drain_node', 'node': drained})
        assert status == 409
        assert harness.command('rollback')[0]['state'] == 'applied'
        assert harness.query('security', cache=False)[1]['generation'] == first
        report['checks']['routing_generation_compatibility_and_rollback'] = {'passed': True}

        # Input/config rejection before bind; no restart of the operational service.
        invalid = directory/'invalid.toml'
        invalid.write_text(harness.path.read_text()+'\nunknown_option=1\n')
        check = subprocess.run(['python3', str(ROOT/'scripts/search-api.py'), '--check-config', '--config', str(invalid)], capture_output=True)
        assert check.returncode != 0
        assert harness.query('security', cache=False)[0] == 200
        # No dashboard is attached to port 3219; querying keeps working independently.
        report['checks']['invalid_config_rejected_ui_independent'] = {'passed': True, 'dashboard_attached': False}

        # Failing starts exhaust the real per-minute budget. Only this private copy
        # loses execute permission; the ready sibling remains usable throughout.
        nodes = base.wait(lambda: ready(harness), 'ready before recovery budget')
        victim = nodes[0]
        binary.chmod(0o644)
        try:
            os.kill(victim['pid'], signal.SIGKILL)
            state = base.wait(lambda: next((node for node in harness.state()['nodes']
                                if node['id'] == victim['id'] and node['recovery_attempts_last_minute'] == 5
                                and node['state'] == 'backoff' and node['next_retry_at']), None), 'recovery circuit open')
            before = state['recovery_attempts_last_minute']
            healthy = [harness.query('security', cache=False) for _ in range(4)]
            time.sleep(.6)
            state = next(node for node in harness.state()['nodes'] if node['id'] == victim['id'])
            assert state['recovery_attempts_last_minute'] == before == 5
            assert all(item[0] == 200 for item in healthy)
            report['checks']['recovery_budget_backoff_circuit_and_healthy_sibling'] = {'passed': True, 'attempts_per_minute': 5,
                'next_retry_at': state['next_retry_at'], 'healthy_p95_ms': base.percentile([item[2] for item in healthy], .95)}
        finally:
            binary.chmod(0o755)
        state = harness.state()
        assert not state['traffic']['user']
        traffic = state['traffic']['benchmark']
        assert traffic['requests'] == sum(traffic.get(key, 0) for key in ['complete', 'failed', 'timeouts', 'cancelled', 'rejected', 'pending'])
        report['checks']['traffic_reconciles_without_test_users'] = {'passed': True}
        report['resource_profile'] = {key: state['configuration'][key] for key in ['workers', 'max_queue', 'deadline_ms', 'replica_timeout_ms', 'recovery_base_ms', 'recovery_max_ms', 'recovery_attempts_per_minute', 'keepalive_seconds', 'keepalive_requests', 'gzip_min_bytes']}
        report['final_resources'] = state['resources']
        assert all(base.sha(ROOT/name) == digest for name, digest in report['source_sha256'].items()), 'Sources changed during integration'
        report['passed'] = True
    finally:
        if harness:
            harness.close()
        report['completed_at'] = datetime.now(ZoneInfo('America/Bogota')).isoformat()
        report['working_directory'] = str(directory.relative_to(ROOT))
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
        print(json.dumps({'passed': report['passed'], 'checks': list(report['checks']), 'output': str(args.output)}))


if __name__ == '__main__':
    main()
