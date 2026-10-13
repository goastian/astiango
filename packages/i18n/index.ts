import en from './messages/en.json';

export const languages = [
  { code: 'en', name: 'English' },
  { code: 'de', name: 'Deutsch' },
  { code: 'pt', name: 'Português' },
  { code: 'es', name: 'Español' },
  { code: 'it', name: 'Italiano' },
  { code: 'fr', name: 'Français' },
  { code: 'zh-CN', name: '简体中文' },
] as const;
export type Locale = (typeof languages)[number]['code'];
export const defaultLocale: Locale = 'en';
export const languageStorageKey = 'astiango.ui-language';
export const languageParameter = 'ui_lang';
const catalogs: Partial<Record<Locale, Record<string, string>>> = { en };
const loaders = {
  de: () => import('./messages/de.json'), pt: () => import('./messages/pt.json'),
  es: () => import('./messages/es.json'), it: () => import('./messages/it.json'),
  fr: () => import('./messages/fr.json'), 'zh-CN': () => import('./messages/zh-CN.json'),
};
const pending = new Map<Locale, Promise<void>>();
export function loadLocale(locale: Locale): Promise<void> {
  if (catalogs[locale]) return Promise.resolve();
  const existing = pending.get(locale);
  if (existing) return existing;
  const loading = loaders[locale as keyof typeof loaders]().then(module => { catalogs[locale] = module.default; }).finally(() => { pending.delete(locale); });
  pending.set(locale, loading);
  return loading;
}
export const formatLocales: Record<Locale, string> = {
  en: 'en-US', de: 'de-DE', pt: 'pt-PT', es: 'es-CO', it: 'it-IT', fr: 'fr-FR', 'zh-CN': 'zh-CN',
};
export function resolveLocale(value: unknown): Locale {
  if (typeof value !== 'string') return defaultLocale;
  const code = value.replace('_', '-').toLowerCase();
  if (code === 'zh' || code === 'zh-cn' || code === 'zh-hans') return 'zh-CN';
  return languages.find(language => language.code === code.split('-')[0])?.code ?? defaultLocale;
}
export function translate(locale: Locale, message: string, parameters: Record<string, unknown> = {}): string {
  const key = message.replace(/\s+/g, ' ').trim();
  const template = catalogs[locale]?.[key] ?? catalogs.en?.[key] ?? message;
  return template.replace(/\{(\w+)\}/g, (token, name: string) =>
    Object.hasOwn(parameters, name) ? String(parameters[name] ?? '') : token,
  );
}
export function localizedHref(href: string, locale: Locale): string {
  if (!href.startsWith('/') || href.startsWith('//')) return href;
  const url = new URL(href, 'https://astiango.invalid');
  url.searchParams.set(languageParameter, locale);
  return `${url.pathname}${url.search}${url.hash}`;
}
