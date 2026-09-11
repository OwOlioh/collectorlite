export type TagNamespace = "system" | "auto" | "manual";

export interface Tag {
  id: number;
  namespace: TagNamespace;
  name: string;
  normalized: string;
  color?: string;
  description?: string;
  count?: number;
  categoryId?: number | null;
}

export interface TagInput {
  id?: number;
  namespace: TagNamespace;
  name: string;
  color?: string;
  categoryId?: number | null;
}

export interface TagCategory {
  id: number;
  name: string;
  normalized: string;
  color?: string;
  position: number;
  /** 所属分类组：组内最靠前成员 id；未分组为 null/undefined。 */
  groupId?: number | null;
}

export interface ItemTagAssignment {
  externalId: string;
  tagSpecs: TagInput[];
}

export interface VideoItem {
  id: number;
  source: string;
  externalId: string;
  sourceUrl: string;
  title: string;
  description: string;
  notes?: string;
  coverUrl?: string;
  coverLocalPath?: string;
  authorName?: string;
  authorId?: string;
  partitionName?: string;
  publishedAt?: number;
  duration?: number;
  favoriteTime?: number;
  deletedAt?: number;
  /** 星标置顶：内容置顶显示。 */
  starred?: boolean;
  /** 打星时间（unix 秒）。 */
  starredAt?: number | null;
  /** 同步到 Obsidian 的笔记在 vault 内的相对路径；未同步为 undefined。 */
  obsidianPath?: string;
  tags: Tag[];
}

/** Obsidian 单向联动配置（与 Rust 端 obsidian::ObsidianSettings 对应）。 */
export interface ObsidianSettings {
  enabled: boolean;
  vaultPath: string;
  vaultName: string;
  subdir: string;
}

export interface CollectionInfo {
  source: string;
  id: string;
  title: string;
  owner?: string;
  count: number;
  url?: string;
}

export interface PartitionSuggestion {
  name: string;
  count: number;
  selected: boolean;
}

export interface ImportPreview {
  collection: CollectionInfo;
  items: VideoItem[];
  partitionSuggestions: PartitionSuggestion[];
}

export interface ImportRequest {
  kind: "favorites" | "public_url";
  mediaId?: string;
  url?: string;
  // 前端已解析好的收藏夹信息（下拉选中项 / 公开链接首次解析结果）。
  // 提供时后端跳过重复的 resolve_collection 网络调用，直接复用。
  collection?: CollectionInfo;
  tagSpecs: TagInput[];
  itemTagAssignments: ItemTagAssignment[];
}

export interface BrowserImportRequest {
  htmlContent: string;
  tagSpecs: TagInput[];
  itemTagAssignments: ItemTagAssignment[];
}

export interface ImportResult {
  runId: number;
  total: number;
  imported: number;
  skipped: number;
  failed: number;
  cleanupStatus?: string;
  errors?: string[];
}

export interface BilibiliProfile {
  isLogin: boolean;
  mid?: number;
  name?: string;
  face?: string;
}

export interface RecacheResult {
  total: number;
  cached: number;
  failed: number;
  errors?: string[];
}

/** 封面缓存队列状态：还有多少张没缓存、后台任务是否在跑。 */
export interface CoverCacheStatus {
  pending: number;
  running: boolean;
}

export interface QrSession {
  qrcodeKey: string;
  qrcodeUrl: string;
}

export interface QrStatus {
  code: number;
  message: string;
  profile?: BilibiliProfile;
}

export interface ItemFilters {
  query?: string;
  tagIds: number[];
  tagMode: "and" | "or";
  /** 严格匹配：item 的标签集合必须恰好等于所选标签（既包含所选、又不含其他）。与 tagMode 互斥，开启后忽略 and/or。 */
  strict?: boolean;
  /** 无标签筛选：仅显示未挂任何标签的收藏。与 tagIds 互斥（开启时忽略 tagIds）。 */
  untagged?: boolean;
  sort: "favorite_desc" | "published_desc" | "duration_desc" | "title_asc" | "imported_desc";
  sources: string[];
  trash?: boolean;
}

export type AppView = "library" | "import" | "trash" | "settings";

/** 收藏的打开方式：唤起桌面客户端，或打开网页版。 */
export type OpenTarget = "client" | "browser";

/** 各来源的打开方式偏好。没列出的来源走默认（客户端优先）。 */
export interface OpenPrefs {
  targets: Record<string, OpenTarget>;
}

/** 浏览器扩展「快速入库」用的本地桥状态。port 为 0 表示桥未启动。 */
export interface BridgeInfo {
  port: number;
  running: boolean;
  token: string;
}

/**
 * 网易云增量同步配置。字段名与后端 `SyncSettings` 的 camelCase 保持一致。
 * `waterMarks` = 歌单 id → 上次同步到的加入时间（秒），前端只读、不要手改。
 */
export interface NeteaseSyncSettings {
  enabled: boolean;
  /** 同步间隔（分钟），后端有 5 分钟下限（网易云风控） */
  intervalMinutes: number;
  /** 取消收藏的歌是否自动移入回收站 */
  autoRemoveUnfavorited: boolean;
  /** 参与自动同步的歌单 id，导入过的歌单会自动登记 */
  playlistIds: string[];
  waterMarks: Record<string, number>;
  lastSyncAt: number | null;
}

/** 网易云增量同步结果。skippedReason 非空 = 这一轮根本没跑。 */
export interface NeteaseSyncReport {
  added: number;
  removed: number;
  playlists: number;
  syncedAt: number | null;
  skippedReason: string | null;
  errors: string[];
}
