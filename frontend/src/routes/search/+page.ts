import type { PageLoad } from './$types';
export const load: PageLoad = ({ data }) => ({...data,globals:{title:`${data.query} – AstianGO`}});
