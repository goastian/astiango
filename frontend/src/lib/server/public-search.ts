import { env } from '$env/dynamic/private';
import { randomUUID, timingSafeEqual } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { PublicSearch, PublicResult } from '$lib/public-search';
import { resolveLocale } from '../../../../packages/i18n';
import { recordSearch } from './search-observation';

const token = (() => {
  try { return readFileSync(resolve(env.SEARCH_PROBE_TOKEN_FILE || '../data/search-api/control-token'),'utf8').trim(); }
  catch { return ''; }
})();
export function trafficFor(request: Request) {
  const expected=Buffer.from(token), actual=Buffer.from(request.headers.get('x-frontend-probe')??'');
  return expected.length>0 && expected.length===actual.length && timingSafeEqual(expected,actual) ? 'probe' : 'user';
}

export class SearchFailure extends Error {
  constructor(public status: number, public code: string) { super(code); }
}
export async function realSearch(params: URLSearchParams, signal: AbortSignal, traffic = 'user'): Promise<PublicSearch> {
  const id = randomUUID(), start = performance.now();
  const query = params.get('q') ?? '';
  const page = Number(params.get('p') ?? '1');
  let status = 200, outcome = 'complete', metadata: Record<string, any> = {};
  try {
    if (['q','p','ss','ui_lang','type','optic','gl'].some(name=>params.getAll(name).length>1)) throw new SearchFailure(400,'invalid_query');
    if (!query.trim() || query.trim().split(/\s+/u).length>16 || new TextEncoder().encode(query).length>4096 || !Number.isInteger(page) || page<1 || page>21) throw new SearchFailure(400,'invalid_query');
    if ((params.get('type') && params.get('type')!=='web') || params.get('optic') || params.get('gl')) throw new SearchFailure(400,'unsupported_option');
    const origin = new URL(env.SEARCH_API_ORIGIN || 'http://127.0.0.1:3210');
    if (!['http:','https:'].includes(origin.protocol) || origin.username || origin.password || origin.pathname!=='/') throw new SearchFailure(503,'invalid_api_configuration');
    const target = new URL('/v1/web/search',origin);
    target.search = new URLSearchParams({ q:query, count:'10', page:String(page-1), safe_search:params.get('ss') ?? 'false', language:resolveLocale(params.get('ui_lang')) }).toString();
    const headers: Record<string,string> = {};
    // A local integration runner may label probes; normal browsers cannot choose traffic.
    if (traffic === 'probe' && token) {
      headers.Authorization=`Bearer ${token}`;
      headers['X-Search-Traffic']='probe';
    }
    const response = await fetch(target,{signal:AbortSignal.any([signal,AbortSignal.timeout(3000)]),headers});
    const reader=response.body?.getReader();
    if(!reader) throw new SearchFailure(502,'invalid_response');
    const chunks: Uint8Array[]=[]; let bytes=0;
    try { while(true) { const part=await reader.read(); if(part.done)break; bytes+=part.value.byteLength;
      if(bytes>512*1024) throw new SearchFailure(502,'response_budget'); chunks.push(part.value); }
    } finally { await reader.cancel().catch(()=>{}); }
    const raw=Buffer.concat(chunks);
    let payload;
    try { payload=JSON.parse(raw.toString('utf8')); } catch { throw new SearchFailure(502,'invalid_response'); }
    if(!response.ok) throw new SearchFailure(response.status,typeof payload.code==='string'?payload.code:'api_unavailable');
    if(payload.version!==1 || typeof payload.id!=='string' || typeof payload.wall_ms!=='number' || !Number.isFinite(payload.wall_ms) || payload.wall_ms<0 || typeof payload.native_cpu_ms!=='number' || !Number.isFinite(payload.native_cpu_ms) || payload.native_cpu_ms<0 || typeof payload.stages_ms!=='object' || payload.stages_ms===null || Object.values(payload.stages_ms).some(value=>typeof value!=='number' || !Number.isFinite(value) || value<0) || typeof payload.generation!=='string' || !Array.isArray(payload.results) || payload.results.length>20 || payload.shards?.partial) throw new SearchFailure(502,'invalid_response');
    const results: PublicResult[]=payload.results.map((item: any)=>{
      if(typeof item.url!=='string' || typeof item.title!=='string' || typeof item.snippet!=='string') throw new SearchFailure(502,'invalid_response');
      let url; try { url=new URL(item.url); } catch { throw new SearchFailure(502,'invalid_result_url'); }
      if(!['http:','https:'].includes(url.protocol) || url.username || url.password) throw new SearchFailure(502,'invalid_result_url');
      const fragments=item.engine_snippet?.fragments;
      if(fragments!==undefined && (!Array.isArray(fragments) || fragments.length>2048 || fragments.some((f:any)=>typeof f.text!=='string' || !['normal','highlighted'].includes(f.kind)))) throw new SearchFailure(502,'invalid_response');
      return {url:item.url,host:url.host,title:item.title || item.url,
        fragments:fragments?.length ? fragments.map((f:any)=>({text:f.text,highlighted:f.kind==='highlighted'})) : [{text:item.snippet,highlighted:false}]};
    });
    const count=payload.num_hits;
    const total:PublicSearch['total']=count && ['exact','approximate'].includes(count._type) && Number.isSafeInteger(count.value) && count.value>=0 ? {kind:count._type,value:count.value} : {kind:'unknown'};
    metadata={engineRequestId:payload.id,generation:payload.generation,nativeCpuMs:payload.native_cpu_ms,allocatedBytes:payload.allocated_bytes,responseBytes:bytes,results:results.length,cache:payload.cache,stagesMs:payload.stages_ms};
    return {version:1,id,generation:payload.generation,query,page,results,total,
      hasMore:page>=21 ? false : (typeof payload.has_more==='boolean' ? payload.has_more : total.kind==='exact' ? (total.value??0)>page*10 : null),
      durationMs:payload.wall_ms,cache:payload.cache};
  } catch(error) {
    status=error instanceof SearchFailure ? error.status : signal.aborted ? 499 : error instanceof Error && ['TimeoutError','AbortError'].includes(error.name)?504:503;
    outcome=error instanceof SearchFailure ? error.code : status===499?'cancelled':status===504?'deadline':'api_unavailable';
    throw new SearchFailure(status,outcome);
  } finally {
    void recordSearch({id,query,page,traffic,status,outcome,apiMs:performance.now()-start,...metadata}).catch(()=>{});
  }
}
