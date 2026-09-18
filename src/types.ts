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

/** 智能匹配标签的单条命中溯源：标签在某条收藏的某个字段里命中了哪段文字。 */
export interface SmartTagMatch {
  /** 命中项 id */
  itemId: number;
  /** 命中项标题，用于在 UI 中标识是哪条收藏 */
  itemTitle: string;
  /** 命中的字段 key（title / description / authorName / partitionName / source） */
  field: string;
  /** 字段中文名（标题 / 描述 / 作者 / 分区 / 来源） */
  fieldLabel: string;
  /** 命中词之前的上下文（已按窗口截断，必要时带前导 …） */
  before: string;
  /** 实际命中的子串（原始大小写） */
  hit: string;
  /** 命中词之后的上下文（已按窗口截断，必要时带尾随 …） */
  after: string;
}

/** 智能匹配标签的候选：一个已有标签 + 它命中的选中项与逐条溯源。 */
export interface SmartTagCandidate {
  tag: Tag;
  /** 命中的选中项 id 集合（仅含尚未挂载该标签的项，保证幂等）。 */
  matchedItemIds: number[];
  /** 逐条命中溯源，供 UI 展开核对依据。 */
  matches: SmartTagMatch[];
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

export type AppView = "library" | "import" | "trash" | "settings" | "stats" | "duplicates";

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

// ── 速记浮窗（P1） ──────────────────────────────────────────────────────────

/** 从网易云窗口标题解析出的曲目。 */
export interface NowPlayingTrack {
  title: string;
  artist: string;
  rawTitle: string;
}

/** 当前播放状态。`track` 为 null 时看 `hint` 判断具体原因。 */
export interface NowPlayingState {
  track: NowPlayingTrack | null;
  hint: string | null;
}

/** 真实播放进度快照（A 方案后台线程累计）。`elapsedMs` 为该曲真实已播毫秒。 */
// NowPlayingProgress 已移除：时间戳改手动时间轴（见方案文档），不再走后端进度快照。

/** 曲目反查结果。resolved=false 表示没匹配到正式条目，但仍可记批注。 */
export interface TrackResolveResult {
  resolved: boolean;
  songId: string | null;
  title: string;
  artist: string;
  coverUrl: string | null;
  duration: number | null;
  inLibrary: boolean;
  itemId: number | null;
}

/** 速记一键入库的请求。 */
export interface QuickCaptureRequest {
  title: string;
  artist: string;
  songId: string | null;
  coverUrl: string | null;
  duration: number | null;
  note: string;
  tags: string[];
}

/** 速记一键入库的结果。 */
export interface QuickCaptureResult {
  itemId: number;
  created: boolean;
  unresolved: boolean;
}

// ── 收藏统计（数据可视化） ──────────────────────────────────────────────

export interface SourceCount {
  source: string;
  count: number;
}

export interface TagCountStat {
  name: string;
  color?: string;
  count: number;
}

export interface MonthCount {
  month: string;
  count: number;
}

export interface CollectionStats {
  total: number;
  starredCount: number;
  untaggedCount: number;
  bySource: SourceCount[];
  byTag: TagCountStat[];
  byMonth: MonthCount[];
}

// ── 跨源重复项检测 ──────────────────────────────────────────────────

export interface DuplicateItemPreview {
  id: number;
  source: string;
  externalId: string;
  sourceUrl: string;
  title: string;
  coverUrl?: string;
  favoriteTime?: number;
}

export interface DuplicateGroup {
  key: string;
  matchType: string;
  items: DuplicateItemPreview[];
}
