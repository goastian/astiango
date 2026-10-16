import { json } from '@sveltejs/kit';
import { realSearch, SearchFailure, trafficFor } from '$lib/server/public-search';
export const GET = async ({ url, request }) => {
  try { return json(await realSearch(url.searchParams,request.signal,trafficFor(request)),{headers:{'Cache-Control':'no-store'}}); }
  catch(error) { return json({code:error instanceof SearchFailure?error.code:'api_unavailable'},{status:error instanceof SearchFailure?error.status:503,headers:{'Cache-Control':'no-store'}}); }
};
