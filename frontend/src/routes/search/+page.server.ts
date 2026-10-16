import { redirect } from '@sveltejs/kit';
import { realSearch, SearchFailure, trafficFor } from '$lib/server/public-search';
import type { PageServerLoad } from './$types';
export const load: PageServerLoad = async ({ url, request, setHeaders }) => {
  setHeaders({'Cache-Control':'no-store'});
  if(!(url.searchParams.get('q')??'').trim()) redirect(303,'/');
  const query=url.searchParams.get('q')??'';
  try { return { query, result:await realSearch(url.searchParams,request.signal,trafficFor(request)), failure:null }; }
  catch(error) { return {query,result:null,failure:{code:error instanceof SearchFailure?error.code:'api_unavailable',status:error instanceof SearchFailure?error.status:503}}; }
};
