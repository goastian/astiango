#!/usr/bin/env python3
"""Observe the real local stack together without injecting captures or users."""
from datetime import datetime
from zoneinfo import ZoneInfo
import argparse
import json
from pathlib import Path
import subprocess
import time
import urllib.parse
import urllib.request

ROOT=Path(__file__).resolve().parents[1]


def read(path):
    value=json.loads((ROOT/path).read_text())
    return value.get("snapshot",value)


def get(url, token=None):
    request=urllib.request.Request(url,headers={"Authorization":f"Bearer {token}","X-Search-Traffic":"probe"} if token else {})
    with urllib.request.urlopen(request,timeout=5) as response:
        return response.status,json.load(response)


def sample():
    states={}
    for component,path in [("crawler","data/crawler-local/status.json"),("indexer","data/indexer-shared/status.json"),("webgraph","data/webgraph-ingestion/status.json")]:
        state=read(Path(path));pid=state["pid"]
        states[component]={"pid":pid,"alive":Path(f"/proc/{pid}").exists(),"phase":state["phase"],"reason":state.get("reason"),"updated_at":state["updated_at"]}
    flow=read(Path("data/control-center/status.json"))
    status,search=get("http://127.0.0.1:3210/status")
    snapshot=search["snapshot"]
    assert status==200 and snapshot["ready"],snapshot["reason"]
    states["search"]={"pid":snapshot["pid"],"alive":True,"ready":snapshot["ready"],"generation":snapshot["generation"]["id"],"documents":snapshot["generation"]["documents"],"last_probe":snapshot.get("last_probe")}
    assert get("http://127.0.0.1:3101/healthz")[0]==200
    assert all(v["alive"] for v in states.values())
    return {"at":datetime.now(ZoneInfo("America/Bogota")).isoformat(),"states":states,"counts":flow["counts"],"backlog":flow["backlog"],"index_adoption":flow.get("indexer_adoption"),"search_resources":snapshot["resources"],"traffic":snapshot["traffic"]}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds",type=int,default=60)
    parser.add_argument("--output",type=Path,default=ROOT/"docs/search-api-coexistencia-2026-10-15.json")
    args=parser.parse_args()
    assert 10<=args.seconds<=120
    deadline=time.monotonic()+30
    while True:
        try:
            if get("http://127.0.0.1:3210/readyz")[0]==200:
                break
        except OSError:
            pass
        if time.monotonic()>deadline:
            raise AssertionError("Search/API did not become ready after startup")
        time.sleep(.2)
    initial=sample();measurements=[initial]
    token=(ROOT/"data/search-api/control-token").read_text().strip()
    status,query=get("http://127.0.0.1:3210/v1/web/search?"+urllib.parse.urlencode({"q":"privacy"}),token)
    assert status==200 and query["reader"]=="LocalSearcher" and query["results"]
    for _ in range(args.seconds//5):
        time.sleep(5)
        measurements.append(sample())
    final=measurements[-1]
    deltas={key:final["counts"][key]-initial["counts"][key] for key in initial["counts"]}
    assert all(deltas[key]>0 for key in ["captures","index_records","graph_records"]),deltas
    assert all(initial["states"][key]["pid"]==final["states"][key]["pid"] for key in initial["states"])
    total=final["traffic"]["user"]
    assert total.get("requests",0)==sum(total.get(key,0) for key in ["complete","failed","timeouts","cancelled","rejected","pending"]),total
    services=subprocess.check_output(["systemctl","--user","list-units","--all","--no-pager","--plain","astiango*"],text=True)
    report={"version":1,"passed":True,"seconds":args.seconds,"injected_inputs":0,"classified_queries":"probe",
            "deltas":deltas,"query":{"generation":query["generation"],"results":len(query["results"]),"wall_ms":query["wall_ms"],"reader":query["reader"]},
            "measurements":measurements,"services":services,"database_reset":False,"payments_enabled":False}
    args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(json.dumps({"passed":True,"deltas":deltas,"output":str(args.output)}))


if __name__=="__main__":main()
