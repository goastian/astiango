import { goto } from '$app/navigation';
import { getContext, setContext, onMount } from 'svelte';
import { derived, writable, get } from 'svelte/store';
import { defaultLocale, loadLocale, formatLocales, languageParameter, languageStorageKey, resolveLocale, translate, type Locale } from '../../../packages/i18n';

export type I18n = ReturnType<typeof createI18n>;
const contextKey = Symbol('astiango.i18n');
function createI18n(initial: Locale) {
  const locale = writable(initial);
  const t = derived(locale, language => (message: string, parameters?: Record<string, unknown>) => translate(language, message, parameters));
  const formatLocale = derived(locale, language => formatLocales[language]);
  let generation = 0;
  async function restoreLocale(value: unknown) {
    const request = ++generation;
    const language = resolveLocale(value);
    await loadLocale(language);
    if (request !== generation) return;
    locale.set(language);
  }
  async function setLocale(value: string) {
    const request = ++generation;
    const language = resolveLocale(value);
    await loadLocale(language);
    if (request !== generation) return;
    locale.set(language);
    try { localStorage.setItem(languageStorageKey, language); } catch { /* Browser storage can be unavailable. */ }
    const url = new URL(window.location.href);
    url.searchParams.set(languageParameter, language);
    void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
  }
  return { locale, t, formatLocale, setLocale, restoreLocale };
}
export function initI18n(initial: Locale = defaultLocale) {
  const i18n = createI18n(initial);
  setContext(contextKey, i18n);
  onMount(() => {
    const explicit = new URL(window.location.href).searchParams.get(languageParameter);
    let saved: string | null = null;
    try { saved = localStorage.getItem(languageStorageKey); } catch { /* Use the default language. */ }
    const restored = resolveLocale(explicit ?? saved ?? initial);
    void i18n.restoreLocale(restored);
    if (!explicit && saved && restored !== initial) i18n.setLocale(restored);
    const unsubscribe = i18n.locale.subscribe(language => { document.documentElement.lang = language; });
    const synchronize = () => {
      const language = resolveLocale(new URL(window.location.href).searchParams.get(languageParameter) ?? get(i18n.locale));
      void i18n.restoreLocale(language);
    };
    window.addEventListener('popstate', synchronize);
    const storage = (event: StorageEvent) => { if (event.key === languageStorageKey) void i18n.restoreLocale(event.newValue); };
    window.addEventListener('storage', storage);
    return () => { unsubscribe(); window.removeEventListener('popstate', synchronize); window.removeEventListener('storage', storage); };
  });
  return i18n;
}
export function getI18n() { return getContext<I18n>(contextKey); }
