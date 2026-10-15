#!/usr/bin/env python3
"""One local integration: real WARC -> native generations -> persistent HTTP search.

Exercises seven languages, version/noindex/304 contracts, bounded overload,
disconnect/deadline recovery, promotion/replay/rollback and interrupted publication.
Benchmarks use three repetitions; synthetic inputs never enter operational totals.
"""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from zoneinfo import ZoneInfo
import gzip
import hashlib
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
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def percentile(values, p):
    values = sorted(values)
    return values[min(len(values)-1, max(0, int((len(values)*p+.999999))-1))]


def summary(values):
    median = statistics.median(values)
    return {"median": median, "mad": statistics.median(abs(value-median) for value in values),
            "min": min(values), "max": max(values)}


def run(argv, log):
    with log.open("ab") as output:
        subprocess.run([str(value) for value in argv], cwd=ROOT, env={**os.environ, "RUST_LOG": "error"},
                       stdout=output, stderr=output, check=True, timeout=180)


def request(port, path, token=None, body=None, cache=True):
    headers = {}
    if token:
        headers = {"Authorization": f"Bearer {token}", "X-Search-Traffic": "benchmark"}
    if not cache:
        headers["X-Search-Cache"] = "off"
    if body is not None:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", headers=headers,
          data=json.dumps(body).encode() if body is not None else None)
    start = time.perf_counter()
    try:
        response = urllib.request.urlopen(req, timeout=8)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        value = json.load(response)
        timing = response.headers.get("Server-Timing", "")
        if timing.startswith("http_serialization;dur="):
            value["_http_serialization_ms"] = float(timing.split("=", 1)[1].split(",", 1)[0])
        return response.status, value, (time.perf_counter()-start)*1000


def wait(predicate, description, seconds=30):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            last = predicate()
            if last:
                return last
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(.08)
    raise AssertionError(f"Timed out: {description}; last={last}")


def legacy(url, title, words):
    html = f"<html><head><title>{title}</title></head><body><main><h1>{title}</h1><p>{words}</p></main></body></html>".encode()
    request_record = f"WARC/1.0\r\nWARC-Type: request\r\nWARC-Date: 2026-10-14T00:00:00Z\r\nWARC-Target-URI: {url}\r\nContent-Length: 0\r\n\r\n\r\n\r\n".encode()
    response_record = f"WARC/1.0\r\nWARC-Type: response\r\nWARC-Identified-Payload-Type: text/html\r\nContent-Length: {len(html)+4}\r\n\r\n\r\n\r\n".encode()+html+b"\r\n\r\n"
    metadata = b"fetchTimeMs: 1"
    return request_record+response_record+f"WARC/1.0\r\nWARC-Type: metadata\r\nContent-Length: {len(metadata)}\r\n\r\n".encode()+metadata+b"\r\n\r\n"


def derivative_fixture(corpus):
    raw = gzip.decompress((corpus / "contract-a.warc.gz").read_bytes())
    position, metadata = 0, None
    while position < len(raw):
        split = raw.index(b"\r\n\r\n", position)
        headers = dict(line.split(":", 1) for line in raw[position:split].decode().split("\r\n")[1:])
        length = int(headers["Content-Length"])
        body = raw[split+4:split+4+length]
        position = split+4+length+4
        if headers.get("WARC-Type", "").strip() == "metadata":
            metadata = json.loads(body)
            break
    assert metadata
    text = "derivativemarker extracted text from a controlled offline JavaScript fixture. " * 40
    payload = ('<html><title>Derived fixture</title><body><script>document.body.textContent = '+json.dumps(text)+';</script></body></html>').encode()
    identity = str(uuid.uuid5(uuid.NAMESPACE_URL, "https://derived.fixture/document"))
    metadata.update(fetch_id=identity, requested_url="https://derived.fixture/document", payload_sha256=hashlib.sha256(payload).hexdigest(), payload_bytes=len(payload), decoded_bytes=len(payload))
    metadata["response"].update(url="https://derived.fixture/document", status=200, received_at="2026-10-14T00:01:00Z")
    metadata["response"]["headers"] = [{"name":"content-type","value_hex":b"text/html; charset=utf-8".hex()}]
    metadata["robots"]["no_index"] = False
    metadata["revalidation"] = None
    def digest(data):return "sha256:"+base64.b32encode(hashlib.sha256(data).digest()).decode()
    def record(kind, rid, body, extra):
        header=f"WARC/1.1\r\nWARC-Type: {kind}\r\nWARC-Record-ID: <urn:uuid:{rid}>\r\nWARC-Date: 2026-10-14T00:01:00Z\r\nContent-Length: {len(body)}\r\nWARC-Target-URI: https://derived.fixture/document\r\nAstian-Capture-Version: 1\r\n{extra}\r\n"
        return gzip.compress(header.encode()+body+b"\r\n\r\n",compresslevel=3,mtime=0)
    http=f"HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {len(payload)}\r\n\r\n".encode()+payload
    capture=record("response",identity,http,f"Content-Type: application/http; msgtype=response\r\nWARC-Identified-Payload-Type: text/html\r\nWARC-Payload-Digest: {digest(payload)}\r\nWARC-Block-Digest: {digest(http)}\r\n")
    encoded=json.dumps(metadata,separators=(",",":")).encode()
    capture+=record("metadata",str(uuid.uuid5(uuid.NAMESPACE_URL,identity)),encoded,f"Content-Type: application/json\r\nWARC-Concurrent-To: <urn:uuid:{identity}>\r\nWARC-Block-Digest: {digest(encoded)}\r\n")
    (corpus/"derived-source.warc.gz").write_bytes(capture)
    doc={"version":1,"id":identity+":offline-js-text-v1","source":metadata,"kind":"offline-js-text-v1",
         "derived_sha256":hashlib.sha256(text.encode()).hexdigest(),"text":text,"title":"Derived fixture","indexing_allowed":True}
    (corpus/"derivative.jsonl").write_text(json.dumps(doc)+"\n")


LANGUAGES = [("en", "security", "Security and reliable independent search"),
             ("de", "Sicherheit", "Sicherheit und zuverlässige unabhängige Suche"),
             ("pt", "segurança", "Segurança e pesquisa independente confiável"),
             ("es", "seguridad", "Seguridad y búsqueda independiente fiable"),
             ("it", "sicurezza", "Sicurezza e ricerca indipendente affidabile"),
             ("fr", "sécurité", "Sécurité et recherche indépendante fiable"),
             ("zh-CN", "安全", "安全与可靠的独立搜索")]


class Harness:
    def __init__(self, directory, artifact, index_root, graph_root, port=3218, deadline=2000, failpoint=None, **options):
        self.directory, self.port = directory, port
        directory.mkdir(parents=True, exist_ok=True)
        config = {"host": "127.0.0.1", "port": port, "state_directory": str(directory.relative_to(ROOT)),
                  "index_root": str(index_root.relative_to(ROOT)), "graph_root": str(graph_root.relative_to(ROOT)),
                  "binary": artifact["binary"], "binary_sha256": artifact["sha256"], "workers": 1,
                  "max_queue": 1, "max_connections": 16, "deadline_ms": deadline, "load_timeout_seconds": 30,
                  "max_qps": 1000, "cache_bytes": 2*1024*1024, "cache_entries": 128, "cache_seconds": 60,
                  "poll_seconds": 1, "memory_guard_bytes": 384*1024*1024, "cpu_index": 4,
                  "auto_promote": False, "scope": "isolated_integration"}
        config.update(options)
        self.path = directory / "profile.toml"
        self.path.write_text("\n".join(f"{key}={json.dumps(value)}" if not isinstance(value, bool) else f"{key}={str(value).lower()}" for key, value in config.items())+"\n")
        self.log = (directory / "service.log").open("ab")
        environment = {**os.environ}
        if failpoint:
            environment["SEARCH_FAILPOINT"] = failpoint
        self.process = subprocess.Popen(["python3", str(ROOT / "scripts/search-api.py"), "--config", str(self.path)],
             cwd=ROOT, stdout=self.log, stderr=self.log, start_new_session=True, env=environment)
        wait(lambda: request(port, "/healthz")[0] == 200, "API health")
        self.token = (directory / "control-token").read_text().strip()

    def state(self):
        return request(self.port, "/status")[1]["snapshot"]

    def query(self, q, language="en", cache=True):
        return request(self.port, "/v1/web/search?"+urllib.parse.urlencode({"q": q, "language": language}), self.token, cache=cache)

    def command(self, action, generation=None, identity=None, await_result=True):
        body = {"id": identity or str(uuid.uuid4()), "action": action}
        if generation:
            body["generation"] = generation
        status, result, _ = request(self.port, "/control", self.token, body)
        assert status == 202, (status, result)
        if await_result:
            result = wait(lambda: next((row for row in self.state()["commands"] if row["id"]==body["id"] and row["state"] in {"applied","failed"}), None), action)
        return result, body

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.log.close()


def native_config(directory, corpus, cohort, files):
    path = directory / f"{cohort}.toml"
    root = directory / cohort / "index"
    text = f'''output_path={json.dumps(str(root))}
host_centrality_store_path={json.dumps(str(corpus/'centrality/harmonic'))}
batch_size=16
[warc_source]
type="Local"
folder={json.dumps(str(corpus))}
names={json.dumps(files)}
[consumer]
record_bytes=41943040
decoded_bytes=33554432
header_bytes=65536
batch_bytes=67108864
workers=1
stack_bytes=8388608
[ingestion]
writer_bytes=25165824
memory_budget_bytes=536870912
cache_bytes=4194304
prepared_bytes=67108864
commit_records=64
max_segments=8
merge_temporary_bytes=1073741824
max_output_bytes=2147483648
disk_reserve_bytes=134217728
publish_generations=true
derivatives={json.dumps([str(corpus/'derivative.jsonl')])}
'''
    path.write_text(text)
    return path, root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/search-api-s00-s04-pruebas-2026-10-15.json")
    args = parser.parse_args()
    base = ROOT / "data/search-integration" / str(uuid.uuid4())
    base.mkdir(parents=True)
    artifact = json.loads((ROOT / "data/artifacts/search/latest.json").read_text())
    index_artifact = json.loads((ROOT / "data/artifacts/indexer/latest.json").read_text())
    native = ROOT / index_artifact["binary"]
    corpus = ROOT / "data/search-integration/corpus"
    if not (corpus / "corpus.json").exists():
        run([native, "--make-corpus", corpus], base / "corpus.log")
    raw = b"WARC/1.0\r\nWARC-Type: warcinfo\r\nContent-Length: 0\r\n\r\n\r\n\r\n"
    for language, term, title in LANGUAGES:
        words = (f"{term} {title}. This document explains useful independent search indexing, privacy, reliability, published generations and verifiable content. ")*60
        raw += legacy(f"https://language.fixture/{language}?signature=a%2Fb&x=2&x=1",title,words)
    multilingual = corpus / "multilingual.warc.gz"
    multilingual.write_bytes(gzip.compress(raw, compresslevel=3, mtime=0))
    derivative_fixture(corpus)
    corpus_hashes = {path.name: sha(path) for path in corpus.glob("*.warc.gz")}
    queries = [(term, language) for language, term, _ in LANGUAGES]
    queries += [("currentbeta","en"),("derivativemarker","en"),("noindexmarker","en"),("obsoletealpha","en"),("revokedmarker","en"),("zzzznosuchdocumentzzzz","en")]
    report = {"version":1,"passed":False,"scope":"isolated_local_native_integration", "started_at":datetime.now(ZoneInfo("America/Bogota")).isoformat(),
              "synthetic_network_requests":0,"operational_inputs_modified":False,"build":artifact,
              "indexer_build":index_artifact["build_id"],"corpus_sha256":corpus_hashes,
              "query_sha256":hashlib.sha256(json.dumps(queries,ensure_ascii=False).encode()).hexdigest(),
              "resource_profile":{"workers":1,"max_queue":1,"cpu_affinity_count":1,"cache_bytes":2097152},
              "hardware":{"cpu_affinity_available":len(os.sched_getaffinity(0)),"platform":os.uname().machine},
              "cold_scope":"new process/readers; OS page cache is not flushed or claimed cold", "external_ml":"not available; default ranking plus saved real-HTML centrality fixture", "cohorts":{},"checks":{}}
    report["source_sha256"]={name:sha(ROOT/name) for name in ["scripts/search-api.py", "scripts/search-api-integration.py", "scripts/ingestion-retention.py"]}
    harness = None
    try:
        roots = {}
        for cohort in ["small", "medium"]:
            files=[f"{cohort}.warc.gz","multilingual.warc.gz","contract-a.warc.gz","contract-b.warc.gz","derived-source.warc.gz"]
            config, root = native_config(base, corpus, cohort, files)
            run([native, config], base / f"{cohort}-ingestion.log")
            selected = json.loads((root / "generation-selector.json").read_text())
            roots[cohort] = (root, config, selected["active"])
            manifest_path=root / "generations" / selected["active"] / "manifest.json"
            cohort_report={"generation":selected["active"],"manifest_sha256":sha(manifest_path),"config_sha256":sha(config),
                           "documents":json.loads(manifest_path.read_text())["documents"],"repetitions":[]}
            for repetition in range(3):
                directory=base / f"{cohort}-service-{repetition}"
                harness=Harness(directory,artifact,root,base / "graph",port=3218)
                assert request(3218,"/readyz")[0]==503
                action,_=harness.command("promote",selected["active"])
                assert action["state"]=="applied",action
                baseline,candidate,cpu,allocated,stages=[],[],[],[],{}
                expected={}
                for query,language in queries:
                    status,value,elapsed=harness.query(query,language,cache=False)
                    assert status==200,(status,value)
                    assert value["generation"]==selected["active"] and value["reader"]=="LocalSearcher"
                    expected[(query,language)]=value["results"]
                    if query in {"noindexmarker","obsoletealpha","revokedmarker","zzzznosuchdocumentzzzz"}:assert not value["results"],(query,value)
                    else:assert value["results"],(query,value)
                    baseline.append(elapsed);cpu.append(value["native_cpu_ms"])
                    allocated.append(value["allocated_bytes"])
                    for stage,measurement in {**value["stages_ms"],"http_serialization":value.get("_http_serialization_ms",0)}.items():
                        stages.setdefault(stage,[]).append(measurement)
                    harness.query(query,language,cache=True)
                for query,language in queries:
                    status,value,elapsed=harness.query(query,language,cache=True)
                    assert status==200 and value["cache"]=="hit" and value["results"]==expected[(query,language)]
                    candidate.append(elapsed)
                state=harness.state()
                assert state["totals"]["requests"]==0,"Benchmarks leaked into user traffic"
                assert state["cache_bytes"]<=2097152 and len(state["nodes"])==1
                cohort_report["repetitions"].append({"baseline_p50_ms":percentile(baseline,.5),"baseline_p95_ms":percentile(baseline,.95),
                    "baseline_p99_ms":percentile(baseline,.99),"candidate_p95_ms":percentile(candidate,.95),
                    "native_cpu_ms_per_query":statistics.mean(cpu),"baseline_qps":1000*len(baseline)/sum(baseline),
                    "allocated_bytes_per_query":statistics.mean(allocated),
                    "rss_bytes":state["resources"]["rss_bytes"],"open_fds":state["resources"]["open_fds"],
                    "process_io":{key:state["resources"].get(key) for key in ["read_bytes","written_bytes","read_chars","written_chars"]},
                    "load_ms":state["nodes"][0]["load_ms"],"samples":len(baseline),"stages_ms":{name:statistics.mean(values) for name,values in stages.items()}})
                cohort_report["repetitions"][-1]["gateway_config_sha256"]=sha(harness.path)
                harness.close();harness=None
            repetitions=cohort_report["repetitions"]
            cohort_report["baseline_p95_ms"]=summary([r["baseline_p95_ms"] for r in repetitions])
            cohort_report["candidate_p95_ms"]=summary([r["candidate_p95_ms"] for r in repetitions])
            cohort_report["p95_reduction_percent"]=(1-cohort_report["candidate_p95_ms"]["median"]/cohort_report["baseline_p95_ms"]["median"])*100
            cohort_report["p99_scope"]="Observed sample percentile, insufficient for a stable tail SLO claim"
            assert cohort_report["p95_reduction_percent"]>=20,cohort_report
            report["cohorts"][cohort]=cohort_report
        # One adverse input contract, without adding another performance claim.
        adverse_config,adverse_root=native_config(base,corpus,"adversarial",["large.warc.gz","contract-a.warc.gz","contract-b.warc.gz"])
        run([native,adverse_config],base/"adversarial.log")
        adverse_selector=json.loads((adverse_root/"generation-selector.json").read_text())
        valid_selector=(adverse_root/"generation-selector.json").read_bytes()
        adverse_config.write_text(adverse_config.read_text().replace('"contract-b.warc.gz"]','"contract-b.warc.gz", "corrupt.warc.gz"]'))
        failed=subprocess.run([str(native),str(adverse_config)],cwd=ROOT,env={**os.environ,"RUST_LOG":"error"},capture_output=True,text=True,timeout=60)
        assert failed.returncode!=0,"Corrupt input was silently accepted"
        assert (adverse_root/"generation-selector.json").read_bytes()==valid_selector
        audit=json.loads(subprocess.check_output([str(native),str(adverse_config),"--audit"],cwd=ROOT,text=True,env={**os.environ,"RUST_LOG":"error"}))
        assert any(row["state"]["outcome"]=="quarantined" for row in audit["sources"]),audit["sources"]
        report["checks"]["large_admitted_and_corruption_quarantined"]={"passed":True,"generation":adverse_selector["active"],"documents":audit["documents"],"source_outcomes":[r["state"]["outcome"] for r in audit["sources"]]}
        root,config,first=roots["small"]
        # A second actual native generation supports meaningful promotion/rollback.
        text=config.read_text().replace('"derived-source.warc.gz"]','"derived-source.warc.gz", "medium.warc.gz"]')
        config.write_text(text)
        run([native,config],base / "generation-change.log")
        second=json.loads((root / "generation-selector.json").read_text())["active"]
        assert second!=first
        harness=Harness(base/"control-service",artifact,root,base/"graph",port=3218,deadline=350)
        result,body=harness.command("promote",first)
        assert result["state"]=="applied"
        for suffix in ["&count=21","&page=21","&country=unknown","&q=repeated"]:
            assert request(3218,"/v1/web/search?q=security"+suffix,harness.token)[0]==400
        assert request(3218,"/v1/web/search?q="+"x"*4097,harness.token)[0]==400
        assert request(3218,"/v1/web/search?q="+urllib.parse.quote("word "*17),harness.token)[0]==400
        assert harness.query("site:language.fixture security")[1]["results"]
        assert harness.query('"independent search"')[1]["results"]
        first_page=request(3218,"/v1/web/search?q=search&count=1&page=0",harness.token)[1]
        second_page=request(3218,"/v1/web/search?q=search&count=1&page=1",harness.token)[1]
        assert len(first_page["results"])==len(second_page["results"])==1
        assert first_page["results"][0]["url"]!=second_page["results"][0]["url"]
        language_result=harness.query("security")[1]
        assert any("signature=a%2Fb&x=2&x=1" in page["url"] for page in language_result["results"])
        report["checks"]["query_limits_filters_pagination_and_identity"]={"passed":True}
        pid=harness.state()["nodes"][0]["pid"]
        os.kill(pid,signal.SIGSTOP)
        with ThreadPoolExecutor(max_workers=4) as pool:
            futures=[pool.submit(harness.query,"security",cache=False) for _ in range(4)]
            statuses=[future.result()[0] for future in futures]
        assert 429 in statuses and 504 in statuses,statuses
        wait(lambda:harness.state()["ready"],"reader recovery")
        wait(lambda:harness.query("security",cache=False)[0]==200,"query after timeout")
        report["checks"]["bounded_overload_deadline_recovery"]={"passed":True,"statuses":statuses}
        pid=harness.state()["nodes"][0]["pid"]
        os.kill(pid,signal.SIGSTOP)
        client=socket.create_connection(("127.0.0.1",3218))
        wire=f"GET /v1/web/search?q=security HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {harness.token}\r\nX-Search-Traffic: benchmark\r\nX-Search-Cache: off\r\n\r\n"
        client.sendall(wire.encode())
        wait(lambda:harness.state()["active"]==1,"admitted disconnect query")
        client.close()
        wait(lambda:harness.state()["traffic"]["benchmark"].get("cancelled",0)>0,"cancel propagated")
        wait(lambda:harness.query("security",cache=False)[0]==200,"query after disconnect")
        report["checks"]["disconnect_cancels_native_work"]={"passed":True}
        result,body=harness.command("promote",second)
        assert result["state"]=="applied",result
        before=harness.state()["nodes"][0]["pid"]
        status,replay,_=request(3218,"/control",harness.token,body)
        assert status==202 and replay["state"]=="applied" and harness.state()["nodes"][0]["pid"]==before
        assert request(3218,"/control",None,{"id":str(uuid.uuid4()),"action":"drain"})[0]==403
        conflict={**body,"action":"warm"}
        assert request(3218,"/control",harness.token,conflict)[0]==409
        assert harness.command("rollback")[0]["state"]=="applied"
        assert harness.query("security")[1]["generation"]==first
        pins=json.loads((harness.directory/"pins.json").read_text())
        assert {first,second}.issubset(set(pins["indexer"]))
        report["checks"]["promotion_rollback_replay_and_pins"]={"passed":True,"first":first,"second":second}
        # Verify the actual retention reader consumes the serving pin contract.
        retention_index=base/"retention-index.toml";retention_graph=base/"retention-graph.toml"
        retention_index.write_text(f'state_directory={json.dumps(str((root.parent).relative_to(ROOT)))}\nmax_output_bytes=2147483648\n')
        retention_graph.write_text(f'state_directory={json.dumps(str(base.relative_to(ROOT)))}\n[graph]\nmax_output_bytes=2147483648\n')
        retention_profile=base/"retention.toml"
        retention_profile.write_text(f'''archive_directory={json.dumps(str((base/'history').relative_to(ROOT)))}
archive_quota_bytes=2147483648
disk_reserve_bytes=134217728
keep_generations=2
minimum_age_seconds=0
high_watermark=0.6
target_watermark=0.4
max_generation_bytes=1073741824
indexer_config={json.dumps(str(retention_index.relative_to(ROOT)))}
webgraph_config={json.dumps(str(retention_graph.relative_to(ROOT)))}
serving_pins={json.dumps(str((harness.directory/'pins.json').relative_to(ROOT)))}
''')
        spec=importlib.util.spec_from_file_location("integration_retention",ROOT/"scripts/ingestion-retention.py")
        retention_module=importlib.util.module_from_spec(spec);spec.loader.exec_module(retention_module)
        retention=retention_module.Retention(retention_profile)
        protected=retention.pinned()["indexer"]
        assert {first,second}.issubset(protected)
        assert not any(path.name in {first,second} for path in retention.candidates("indexer",protected))
        scratch=root/"generations"/str(uuid.uuid4());scratch.mkdir();(scratch/"config.json").write_text("{}");(scratch/"temporary.bin").write_bytes(b"uncommitted")
        os.utime(scratch,(time.time()-700,time.time()-700))
        cleanup=retention.prune_incomplete("indexer")
        assert cleanup["removed"]==1 and cleanup["published_generations_removed"]==0 and (root/"ledger.redb").is_file()
        report["checks"]["retention_honors_serving_pins_and_only_prunes_scratch"]={"passed":True,"protected":len(protected)}
        invalid=str(uuid.uuid4());bad=root/"generations"/invalid;bad.mkdir()
        manifest=json.loads((root/"generations"/second/"manifest.json").read_text());manifest.update(version=999,id=invalid)
        (bad/"manifest.json").write_text(json.dumps(manifest))
        result,_=harness.command("promote",invalid)
        assert result["state"]=="failed" and harness.query("security")[1]["generation"]==first
        report["checks"]["unknown_version_retains_valid_reader"]={"passed":True}
        control_directory=harness.directory;harness.close();harness=None
        harness=Harness(control_directory,artifact,root,base/"graph",port=3218,failpoint="after_selector")
        wait(lambda:harness.state()["ready"],"recovery before fault")
        harness.command("promote",second,await_result=False)
        wait(lambda:harness.process.poll() is not None,"publication fault exits")
        harness.close();harness=None
        harness=Harness(control_directory,artifact,root,base/"graph",port=3218)
        wait(lambda:harness.state()["ready"],"publication repair")
        status,value,_=harness.query("security")
        assert status==200 and value["generation"]==second
        assert any(row["state"]=="interrupted" for row in harness.state()["commands"])
        report["checks"]["interrupted_publication_recovers_selector"]={"passed":True,"served":second}
        report["checks"]["seven_languages_noindex_versions_304_and_signed_urls"]={"passed":True,"languages":[r[0] for r in LANGUAGES]}
        assert all(sha(ROOT/name)==value for name,value in report["source_sha256"].items()),"Implementation changed during the benchmark"
        report["passed"]=True
    finally:
        if harness:
            harness.close()
        report["completed_at"]=datetime.now(ZoneInfo("America/Bogota")).isoformat()
        report["working_directory"]=str(base.relative_to(ROOT))
        args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
        print(json.dumps({"passed":report["passed"],"output":str(args.output),"checks":list(report["checks"])}))


if __name__=="__main__":
    main()
