export type GenerationStorage = {
  version: number;
  updated_at: number;
  state: string;
  error: string | null;
  archive_quota_bytes: number;
  components: Record<string, {
    working_bytes: number;
    working_quota_bytes: number;
    archived_generations: number;
    archive_bytes: number;
    recoverable_bytes: number;
    pinned_generations: number;
  }>;
};
