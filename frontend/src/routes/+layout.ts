import { PUBLIC_API_BASE } from '$env/static/public';
import { setGlobalApiBase } from '$lib/api';
import { loadLocale } from '../../../packages/i18n';
import { globals } from '$lib/globals';
import type { LayoutLoad } from './$types';

setGlobalApiBase(PUBLIC_API_BASE || '');

export const load: LayoutLoad = async ({ data }) => {
  await loadLocale(data.uiLocale);
  return { ...data, globals: await globals() };
};
