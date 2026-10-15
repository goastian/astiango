#!/usr/bin/env python3
"""Bounded projection of the running ingestion pipeline; fetch workers never wait for it."""
import argparse
from collections import deque
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import signal
import time
import tomllib

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('helpers',ROOT/'scripts/indexer-local.py')
h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
class Projection:
    def __init__(self,adopt=False):
        self.adopt=adopt;self.cached={};self.history=deque(maxlen=300);self.ids=None
        self.directory=ROOT/'data/control-center';self.directory.mkdir(parents=True,exist_ok=True)
        self.running=True;self.last_adoption=None;self.adoption_error=None;self.cpu_previous={};self.open_scan_at=0;self.open_path=None
    def read(self,path,limit=1048576):
        info=path.stat();key=(info.st_ino,info.st_size,info.st_mtime_ns)
        previous=self.cached.get(str(path))
        if previous and previous[0]==key:return previous[1]
        value=h.limited_json(path,limit)
        self.cached[str(path)]=(key,value)
        if len(self.cached)>32:self.cached.pop(next(iter(self.cached)))
        return value
    def adopt_graph(self,state):
        generation=state['generation'];directory=ROOT/'data/webgraph-ingestion/graph/generations'/generation
        if self.last_adoption==generation:return
        manifest=self.read(directory/'manifest.json')
        if manifest.get('version')!=2 or manifest.get('state')!='ready' or not manifest.get('signals'):
            raise ValueError('Signal adoption requires a ready precision-verified generation')
        if h.checksum(directory/'manifest.json')!=state['manifest_sha256']:
            raise ValueError('Selected graph manifest changed')
        config_path=ROOT/'configs/indexer/shared-local.toml';text=config_path.read_text();config=tomllib.loads(text)
        if 'ingestion' not in config:raise ValueError('Signal adoption requires shared indexing')
        expected=str(directory.relative_to(ROOT));old=config['ingestion'].get('graph_generation')
        if old!=expected:
            lines=text.splitlines();section=False;found=False
            for i,line in enumerate(lines):
                if line.strip().startswith('['):section=line.strip()=='[ingestion]'
                if section and line.lstrip().startswith('graph_generation'):
                    lines[i]='graph_generation='+json.dumps(expected);found=True
            if not found:
                index=lines.index('[ingestion]')+1;lines.insert(index,'graph_generation='+json.dumps(expected))
            candidate='\n'.join(lines)+'\n';tomllib.loads(candidate)
            temporary=config_path.with_suffix('.signals.tmp')
            with temporary.open('w') as output:output.write(candidate);output.flush();os.fsync(output.fileno())
            temporary.replace(config_path)
            audit_path=self.directory/'actions.jsonl'
            if audit_path.exists() and audit_path.stat().st_size>4*1024*1024:
                for index in range(3,0,-1):
                    old=audit_path.with_suffix(f'.jsonl.{index}');new=audit_path.with_suffix(f'.jsonl.{index+1}')
                    if old.exists():old.replace(new)
                audit_path.replace(audit_path.with_suffix('.jsonl.1'))
            with audit_path.open('a') as audit:
                audit.write(json.dumps({'at':h.timestamp(),'action':'pin_index_generation_signals','graph_generation':generation,
                                       'manifest_sha256':state['manifest_sha256'],'previous':old})+'\n');audit.flush();os.fsync(audit.fileno())
        self.last_adoption=generation;self.adoption_error=None
    def project(self):
        now=time.time();snapshots={}
        for name,path in [('crawler','data/crawler-local/status.json'),('indexer','data/indexer-shared/status.json'),('webgraph','data/webgraph-ingestion/status.json')]:
            try:
                value=self.read(ROOT/path,16777216 if name=='crawler' else 262144);value=value.get('snapshot',value)
                snapshots[name]=value
            except (OSError,ValueError):snapshots[name]={}
        c,i,g=[snapshots[n] for n in ['crawler','indexer','webgraph']]
        ids=tuple(s.get('run_id') for s in [c,i,g])
        if self.ids!=ids:self.history.clear();self.ids=ids
        opened=None;spool=ROOT/'data/crawler-local/warc/spool'
        current=(c.get('warc') or {}).get('current_file')
        if current and (c.get('spool') or {}).get('segments_open'):
            try:
                descriptor=self.read(spool/'state'/str(Path(current).with_suffix('').with_suffix('.json')),65536)
                if descriptor.get('state')=='open':opened=descriptor
            except (OSError,ValueError):pass
            if not opened:
                if now-self.open_scan_at>10:
                    self.open_scan_at=now;self.open_path=None
                    for index,path in enumerate((spool/'state').glob('*.json')):
                        if index>=4096:break
                        try:
                            if self.read(path,65536).get('state')=='open':self.open_path=path;break
                        except (OSError,ValueError):pass
                if self.open_path:
                    try:
                        candidate=self.read(self.open_path,65536)
                        if candidate.get('state')=='open':opened=candidate
                    except (OSError,ValueError):pass
        configuration=tomllib.loads((ROOT/'configs/crawler/local.toml').read_text())
        open_records=(opened or {}).get('records',0)
        created=(opened or {}).get('created_at')
        if isinstance(created,str):created=datetime.fromisoformat(created.replace('Z','+00:00')).timestamp()
        if not isinstance(created,(int,float)):created=now
        counts={'captures':(c.get('spool') or {}).get('records_durable',0),
                'index_records':(i.get('totals') or {}).get('source_records_completed',0),
                'graph_records':(g.get('totals') or {}).get('source_records',0),
                'documents':(i.get('totals') or {}).get('indexed_documents',0),
                'edges':(g.get('totals') or {}).get('stored_edges',0)}
        sample={'at':h.timestamp(),**counts}
        self.history.append(sample);samples=list(self.history)
        minute=[s for s in samples if datetime.fromisoformat(s['at']).timestamp()>=now-60]
        first=minute[0];seconds=max(.001,now-datetime.fromisoformat(first['at']).timestamp())
        rates={key:max(0,sample[key]-first[key])*60/seconds for key in counts}
        signals=None;adoption=None
        try:
            signals=self.read(ROOT/'data/webgraph-ingestion/graph/signals-current.json',262144)
            selected=self.read(ROOT/'data/webgraph-ingestion/graph/graph-selector.json',65536)
            if signals.get('generation')!=selected.get('active') or signals.get('manifest_sha256')!=selected.get('manifest_sha256'):signals=None
            elif self.adopt and signals.get('signals'):self.adopt_graph(signals)
        except (OSError,ValueError) as error:
            if signals:self.adoption_error=str(error)[:512]
        try:adoption=self.read(ROOT/'data/indexer-shared/index/generations/status.json',262144).get('signals_adoption')
        except (OSError,ValueError):pass
        states={}
        for name,s in snapshots.items():
            worker=s.get('worker') or {};resources=s.get('resources') or {}
            try:age=now-datetime.fromisoformat(s['updated_at'].replace('Z','+00:00')).timestamp()
            except (KeyError,ValueError):age=float('inf')
            parent=h.process_sample(s.get('pid',0)) or {}
            worker_pid=worker.get('pid');process=h.process_sample(worker_pid) if worker_pid else None
            measured=process or parent
            cpu=parent.get('cpu_seconds',0)+(process or {}).get('cpu_seconds',0)
            old=self.cpu_previous.get(name)
            cpu_percent=max(0,(cpu-old[1])*100/max(.001,now-old[0])) if old else 0
            self.cpu_previous[name]=(now,cpu)
            states[name]={'pid':s.get('pid'),'run_id':s.get('run_id'),'alive':bool(s.get('pid') and h.process_sample(s['pid']) and age<=10),
                          'phase':s.get('phase','unavailable'),'reason':s.get('reason',resources.get('reason')),
                          'rss_bytes':measured.get('rss_bytes',resources.get('rss_bytes',0)),
                          'cpu_percent':resources.get('process_cpu_percent',cpu_percent),
                          'open_fds':measured.get('open_fds',resources.get('open_fds',0)),
                          'input':worker.get('input'),'stage':worker.get('stage',(s.get('runtime') or {}).get('stage'))}
        backlog={'indexer':max(0,counts['captures']-open_records-counts['index_records']),
                 'webgraph':max(0,counts['captures']-open_records-counts['graph_records'])}
        data={'version':1,'updated_at':sample['at'],'counts':counts,'rates':rates,'states':states,'history':samples,
              'backlog':backlog,'open_segment':{'id':(opened or {}).get('id'),'records':open_records,
                  'age_seconds':max(0,now-created),'rotation_seconds':configuration['spool']['rotate_seconds'],
                  'estimated_seconds_to_seal':max(0,configuration['spool']['rotate_seconds']-(now-created))} if opened else None,
              'signals':signals,'indexer_adoption':adoption,'adoption_error':self.adoption_error,
              'scope':'retained captures and terminal input records; current documents/edges are separate','read_only_http':True}
        for path in [self.directory/'status.json',ROOT/'tools/crawler-dashboard/dist/component-flow.json']:
            try:h.atomic_json(path,data,durable=False)
            except OSError as error:print('Observation output unavailable:',error,flush=True)
    def run(self):
        signal.signal(signal.SIGTERM,lambda *_:setattr(self,'running',False))
        signal.signal(signal.SIGINT,lambda *_:setattr(self,'running',False))
        with (self.directory/'observer.lock').open('w') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            while self.running:
                try:self.project()
                except (OSError,ValueError,KeyError) as error:print('Projection unavailable:',error,flush=True)
                time.sleep(2)
if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--adopt-signals',action='store_true');args=parser.parse_args()
    Projection(args.adopt_signals).run()
