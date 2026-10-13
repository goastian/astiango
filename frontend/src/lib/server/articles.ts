import { Marked } from 'marked';
import { localizedHref, type Locale } from '../../../../packages/i18n';

const articles = import.meta.glob<string>('../articles/**/*.md', { query: '?raw', import: 'default', eager: true });
export async function loadArticle(article: 'about' | 'webmasters' | 'privacy-and-happy-lawyers', locale: Locale) {
  const source = articles[`../articles/${article}/${locale}.md`];
  if (source === undefined) throw new Error(`Missing article translation: ${article}/${locale}`);
  const localized = source.replace(/\]\((\/[^)]*)\)/g, (_, href: string) => `](${localizedHref(href, locale)})`);
  return { md: await new Marked().parse(localized) };
}
