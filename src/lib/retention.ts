// 回收站保留天数设置（本地持久化，默认 7 天，已与用户确认）。
export const DEFAULT_RETENTION_DAYS = 7;
export const RETENTION_OPTIONS = [7, 15, 30] as const;

const STORAGE_KEY = "bilibili_collector.retention_days";

export function getRetentionDays(): number {
  const raw = Number(localStorage.getItem(STORAGE_KEY));
  if ((RETENTION_OPTIONS as readonly number[]).includes(raw)) return raw;
  return DEFAULT_RETENTION_DAYS;
}

export function setRetentionDays(days: number): void {
  localStorage.setItem(STORAGE_KEY, String(days));
}
