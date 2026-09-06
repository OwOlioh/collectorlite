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

/** 浏览器扩展「快速入库」用的本地桥状态。port 为 0 表示桥未启动。 */
export interface BridgeInfo {
  port: number;
  running: boolean;
  token: string;
}
