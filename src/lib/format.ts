export function formatDuration(seconds?: number): string {
  if (!seconds) return "未知";
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins}:${String(secs).padStart(2, "0")}`;
}

export function formatDate(timestamp?: number): string {
  if (!timestamp) return "未知";
  return new Date(timestamp * 1000).toLocaleDateString("zh-CN");
}
