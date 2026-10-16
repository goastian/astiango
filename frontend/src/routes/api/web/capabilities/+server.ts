import { json } from '@sveltejs/kit';
import { publicCapabilities } from '$lib/public-search';
export const GET = () => json({version:1,capabilities:publicCapabilities,limits:{queryBytes:4096,queryTerms:16,pageSize:10,maxPage:21,deadlineMs:3000,responseBytes:524288},documentLanguageFilter:false},{headers:{'Cache-Control':'no-store'}});
