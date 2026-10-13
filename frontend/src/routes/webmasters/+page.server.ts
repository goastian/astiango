import { languageParameter, resolveLocale } from '../../../../packages/i18n';
import { loadArticle } from '$lib/server/articles';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ url }) => {
  const { md } = await loadArticle('webmasters', resolveLocale(url.searchParams.get(languageParameter)));
  return { md, globals: { title: 'Webmasters' } };
};
