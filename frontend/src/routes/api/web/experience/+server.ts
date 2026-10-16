import { json } from '@sveltejs/kit';
import { recordExperience } from '$lib/server/search-observation';
export const POST = async ({ request, url }) => {
  if(request.headers.get('origin')!==url.origin) return new Response(null,{status:403});
  const reader=request.body?.getReader(); let source=''; let bytes=0;
  if(!reader) return new Response(null,{status:400});
  try { while(true){const part=await reader.read();if(part.done)break;bytes+=part.value.byteLength;if(bytes>4096)return new Response(null,{status:413});source+=new TextDecoder().decode(part.value);}
    const value=JSON.parse(source);if(typeof value.id!=='string')return new Response(null,{status:400});
    recordExperience(value.id,value); return json({accepted:true});
  } catch { return new Response(null,{status:400}); } finally { await reader.cancel().catch(()=>{}); }
};
