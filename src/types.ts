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
  tags: Tag[];
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
  sort: "favorite_desc" | "published_desc" | "duration_desc" | "title_asc" | "imported_desc";
  sources: string[];
  trash?: boolean;
}

export type AppView = "library" | "import" | "trash" | "settings";
