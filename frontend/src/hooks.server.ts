import '$lib/server/search-observation';
import type { Handle } from '@sveltejs/kit';
import { languageParameter, resolveLocale } from '../../packages/i18n';

export const handle: Handle = ({ event, resolve }) => {
  const locale = resolveLocale(event.url.searchParams.get(languageParameter));
  return resolve(event, {
    transformPageChunk: ({ html }) => html.replace('%astiango.locale%', locale),
  });
};
