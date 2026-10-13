import { languageParameter, resolveLocale, loadLocale } from '../../../packages/i18n';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = async ({ url }) => {
  const uiLocale = resolveLocale(url.searchParams.get(languageParameter));
  await loadLocale(uiLocale);
  return { uiLocale };
};
