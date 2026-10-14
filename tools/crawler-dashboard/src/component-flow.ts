export interface SignalBinding {generation:string; scope:string; algorithm:string; nodes:number; edges:number; manifest_sha256:string; contract_sha256:string; adopted?:boolean}
export interface ComponentFlow {
  version:number; updated_at:string;
  counts:Record<string,number>; rates:Record<string,number>; backlog:Record<string,number>;
  states:Record<string,{alive:boolean;pid:number;phase:string;rss_bytes:number;cpu_percent:number;open_fds:number;stage?:string}>;
  history:Array<{at:string;captures:number;index_records:number;graph_records:number;documents:number;edges:number}>;
  open_segment:{id:string;records:number;age_seconds:number;rotation_seconds:number;estimated_seconds_to_seal:number}|null;
  signals:{generation:string;manifest_sha256:string;signals:{algorithm:string;scope:string;nodes:number;edges:number;iterations:number;cpu_ms:number;elapsed_ms:number;reserved_bytes:number;absolute_error_bound:number;legacy_collisions:number;top:Array<{node:string;score:number;rank:number}>}}|null;
  indexer_adoption:SignalBinding|null; adoption_error:string|null;
}
export function recentRates(history:ComponentFlow['history'], key:'captures'|'index_records'|'graph_records') {
  return history.map((point,i)=>({at:point.at,value:i?Math.max(0,point[key]-history[i-1][key])*60000/Math.max(1,Date.parse(point.at)-Date.parse(history[i-1].at)):0}));
}
