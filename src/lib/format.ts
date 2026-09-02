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

/// 根据来源与作者 id 拼出原作者个人空间链接；不支持的来源或缺失 id 时返回 null。
/// 各平台个人空间 URL 模板：
/// - bilibili -> https://space.bilibili.com/{mid}
/// - zhihu    -> https://www.zhihu.com/people/{id}
/// - csdn     -> https://blog.csdn.net/{id}
/// - github   -> https://github.com/{id}
export function authorProfileUrl(
  source: string | undefined,
  authorId: string | undefined,
): string | null {
  if (!source || !authorId) return null;
  const id = authorId.trim();
  if (!id) return null;
  switch (source) {
    case "bilibili":
      return `https://space.bilibili.com/${id}`;
    case "zhihu":
      return `https://www.zhihu.com/people/${id}`;
    case "csdn":
      return `https://blog.csdn.net/${id}`;
    case "github":
      return `https://github.com/${id}`;
    default:
      return null;
  }
}
