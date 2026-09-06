// 自动备份设置（本地持久化，与回收站保留期同一套 localStorage 模式）。
// 启用后每 BACKUP_INTERVAL_MS 自动把整库（含标签与分类）导出一份 JSON 到所选文件夹。

export interface BackupSettings {
  enabled: boolean;
  /** 备份文件夹绝对路径 */
  folder: string;
  /** 上次成功备份的毫秒时间戳；null = 尚未备份过 */
  lastRunAt: number | null;
}

export const BACKUP_INTERVAL_MS = 3 * 24 * 60 * 60 * 1000; // 每 3 天

const STORAGE_KEY = "bilibili_collector.backup_settings";

const DEFAULTS: BackupSettings = {
  enabled: false,
  folder: "",
  lastRunAt: null
};

export function getBackupSettings(): BackupSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw) as Partial<BackupSettings>;
    return {
      enabled: parsed.enabled === true,
      folder: typeof parsed.folder === "string" ? parsed.folder : "",
      lastRunAt: typeof parsed.lastRunAt === "number" ? parsed.lastRunAt : null
    };
  } catch {
    return { ...DEFAULTS };
  }
}

export function setBackupSettings(settings: BackupSettings): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

/** 生成带本地时间的备份文件名，如 bili-collector-backup-20260906-153000.json */
export function buildBackupFileName(date = new Date()): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  const stamp =
    `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}` +
    `-${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`;
  return `bili-collector-backup-${stamp}.json`;
}

/** 当前设置是否「应该到期执行一次备份」（启用 + 已选文件夹 + 距上次 ≥ 3 天） */
export function isBackupDue(settings: BackupSettings, now = Date.now()): boolean {
  if (!settings.enabled || !settings.folder) return false;
  if (settings.lastRunAt === null) return true;
  return now - settings.lastRunAt >= BACKUP_INTERVAL_MS;
}

export function formatBackupTime(timestamp: number | null): string {
  if (timestamp === null) return "从未";
  const date = new Date(timestamp);
  const pad = (value: number) => String(value).padStart(2, "0");
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}`
  );
}
