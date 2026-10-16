export type PublicResult = {
  url: string; title: string; host: string;
  fragments: { text: string; highlighted: boolean }[];
};
export type PublicSearch = {
  version: 1; id: string; generation: string; query: string; page: number;
  results: PublicResult[]; total: { kind: 'exact' | 'approximate' | 'unknown'; value?: number };
  hasMore: boolean | null; durationMs: number; cache: string;
};
export const publicCapabilities = {
  web: true, snippets: true, safeSearch: true, pagination: true,
  images: false, videos: false, news: false, discussions: false,
  autosuggest: false, widgets: false, optics: false, region: false,
  documentLanguageFilter: false,
};
