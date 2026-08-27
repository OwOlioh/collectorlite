import { invoke } from "@tauri-apps/api/core";
import type {
  BilibiliProfile,
  BrowserImportRequest,
  CollectionInfo,
  ImportPreview,
  ImportRequest,
  ImportResult,
  ItemFilters,
  QrSession,
  QrStatus,
  Tag,
  TagCategory,
  TagInput,
  VideoItem
} from "../types";
import { convertFileSrc } from "@tauri-apps/api/core";

const inTauri = () => "__TAURI_INTERNALS__" in window;

export const resolveCoverUrl = (coverUrl?: string, coverLocalPath?: string) => {
  if (coverLocalPath && inTauri()) {
    return convertFileSrc(coverLocalPath);
  }
  return coverUrl || "";
};

const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
  if (inTauri()) {
    return invoke<T>(command, args);
  }
  return mockInvoke<T>(command, args);
};

export const api = {
  startQrLogin: () => call<QrSession>("bilibili_start_qr_login"),
  pollQrLogin: (key: string) =>
    call<QrStatus>("bilibili_poll_qr_login", { qrcodeKey: key }),
  getProfile: () => call<BilibiliProfile>("bilibili_profile"),
  logout: () => call<null>("logout"),
  listBilibiliFavorites: () =>
    call<CollectionInfo[]>("list_bilibili_favorites"),
  parsePublicFavoriteUrl: (url: string) =>
    call<CollectionInfo>("parse_public_favorite_url", { url }),
  previewImport: (request: ImportRequest) =>
    call<ImportPreview>("preview_import", { input: request }),
  executeImport: (request: ImportRequest) =>
    call<ImportResult>("execute_import", { input: request }),
  listItems: (filters: ItemFilters) =>
    call<VideoItem[]>("search_items", { filters }),
  deleteItem: (itemId: number) =>
    call<null>("delete_item", { itemId }),
  deleteItems: (itemIds: number[]) =>
    call<number>("delete_items", { itemIds }),
  deleteItemsByTag: (tagId: number) =>
    call<number>("delete_items_by_tag", { tagId }),
  listTags: () => call<Tag[]>("list_tags"),
  listTagCategories: () => call<TagCategory[]>("list_tag_categories"),
  upsertTag: (tag: TagInput) => call<Tag>("upsert_tag", { tag }),
  mergeTags: (sourceTagId: number, targetTagId: number) =>
    call<null>("merge_tags", { sourceTagId, targetTagId }),
  deleteTag: (tagId: number) => call<null>("delete_tag", { tagId }),
  createTagCategory: (name: string, color?: string) =>
    call<TagCategory>("create_tag_category", { name, color }),
  renameTagCategory: (categoryId: number, name: string, color?: string) =>
    call<TagCategory>("rename_tag_category", { categoryId, name, color }),
  deleteTagCategory: (categoryId: number) =>
    call<null>("delete_tag_category", { categoryId }),
  assignTagCategory: (tagId: number, categoryId: number | null) =>
    call<Tag>("assign_tag_category", { tagId, categoryId }),
  updateItemTags: (itemId: number, tagSpecs: TagInput[]) =>
    call<VideoItem>("update_item_tags", { itemId, tagSpecs }),
  updateItemNotes: (itemId: number, notes: string) =>
    call<VideoItem>("update_item_notes", { itemId, notes }),
  importBrowserBookmarks: (request: BrowserImportRequest) =>
    call<ImportResult>("import_browser_bookmarks", {
      htmlContent: request.htmlContent,
      tagSpecs: request.tagSpecs
    }),
  openUrl: (url: string) => call<null>("open_url", { url })
};

let mockTags: Tag[] = [
  { id: 1, namespace: "auto", name: "分区：知识", normalized: "分区：知识", color: "#3b82f6", count: 8 },
  { id: 2, namespace: "auto", name: "UP主：示例创作者", normalized: "up主：示例创作者", color: "#f97316", count: 8 },
  { id: 3, namespace: "manual", name: "值得再看", normalized: "值得再看", color: "#10b981", count: 3 }
];

let mockCategories: TagCategory[] = [];
let nextTagId = 4;
let nextCategoryId = 1;

const mockItems: VideoItem[] = [
  {
    id: 1,
    source: "bilibili",
    externalId: "BV1TEST0001",
    sourceUrl: "https://www.bilibili.com/video/BV1TEST0001",
    title: "如何建立自己的知识管理系统",
    description: "这是一条用于前端预览的示例视频元数据。",
    coverUrl: "https://i0.hdslb.com/bfs/archive/0e90a1ca7e25a1e7c0c9cb93a9d2b0cc3baee101.jpg",
    authorName: "示例创作者",
    authorId: "10001",
    partitionName: "知识",
    publishedAt: 1754300000,
    duration: 842,
    favoriteTime: 1754700000,
    tags: [mockTags[0], mockTags[1], mockTags[2]]
  },
  {
    id: 2,
    source: "bilibili",
    externalId: "BV1TEST0002",
    sourceUrl: "https://www.bilibili.com/video/BV1TEST0002",
    title: "标签筛选与检索工作流",
    description: "第二条示例视频。",
    coverUrl: "https://i0.hdslb.com/bfs/archive/4ff20d834cb8882dae18aab00df119f1f95bbd76.jpg",
    authorName: "示例创作者",
    authorId: "10001",
    partitionName: "科技",
    publishedAt: 1754000000,
    duration: 1260,
    favoriteTime: 1754800000,
    tags: [mockTags[0], mockTags[1]]
  }
];

const mockCollections: CollectionInfo[] = [
  {
    source: "bilibili",
    id: "123456789",
    title: "默认收藏夹",
    owner: "示例用户",
    count: 42,
    url: "https://space.bilibili.com/10001/favlist?fid=123456789"
  },
  {
    source: "bilibili",
    id: "987654321",
    title: "知识库",
    owner: "示例用户",
    count: 8,
    url: "https://space.bilibili.com/10001/favlist?fid=987654321"
  }
];

async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  await new Promise((resolve) => setTimeout(resolve, 180));
  switch (command) {
    case "bilibili_start_qr_login":
      return {
        qrcodeKey: "mock-qrcode-key",
        qrcodeUrl: "https://www.bilibili.com"
      } as T;
    case "bilibili_poll_qr_login":
      return { code: 0, message: "登录成功", profile: mockProfile } as T;
    case "bilibili_profile":
      return mockProfile as T;
    case "logout":
      return null as T;
    case "list_bilibili_favorites":
      return mockCollections as T;
    case "parse_public_favorite_url": {
      const url = String(args?.url ?? "");
      return {
        source: "bilibili",
        id: "123456789",
        title: url ? "公开收藏夹" : "公开收藏夹",
        owner: "公开用户",
        count: mockItems.length,
        url
      } as T;
    }
    case "preview_import":
      return {
        collection: mockCollections[1],
        items: mockItems,
        partitionSuggestions: [
          { name: "知识", count: 1, selected: true },
          { name: "科技", count: 1, selected: true }
        ]
      } as T;
    case "execute_import":
      return {
        runId: 1,
        total: mockItems.length,
        imported: mockItems.length,
        skipped: 0,
        failed: 0,
        cleanupStatus: "pending",
        errors: []
      } as T;
    case "search_items":
      return mockItems.map((item) => ({
        ...item,
        tags: item.tags.map((tag) => ({ ...tag }))
      })) as T;
    case "delete_item": {
      const itemId = Number(args?.itemId);
      const index = mockItems.findIndex((item) => item.id === itemId);
      if (index >= 0) mockItems.splice(index, 1);
      return null as T;
    }
    case "delete_items": {
      const itemIds = (args?.itemIds as number[] | undefined) ?? [];
      const ids = new Set(itemIds.map(Number));
      for (let index = mockItems.length - 1; index >= 0; index -= 1) {
        if (ids.has(mockItems[index].id)) mockItems.splice(index, 1);
      }
      return itemIds.length as T;
    }
    case "delete_items_by_tag": {
      const tagId = Number(args?.tagId);
      const before = mockItems.length;
      for (let index = mockItems.length - 1; index >= 0; index -= 1) {
        if (mockItems[index].tags.some((tag) => tag.id === tagId)) {
          mockItems.splice(index, 1);
        }
      }
      return (before - mockItems.length) as T;
    }
    case "list_tags":
      return mockTags.map((tag) => ({ ...tag })) as T;
    case "list_tag_categories":
      return mockCategories.map((category) => ({ ...category })) as T;
    case "upsert_tag": {
      const input = args?.tag as TagInput | undefined;
      const name = String(input?.name ?? "").trim();
      const normalized = name.toLowerCase();
      const existing = input?.id
        ? mockTags.find((tag) => tag.id === input.id)
        : mockTags.find((tag) => tag.normalized.toLowerCase() === normalized);
      if (existing) {
        existing.name = name || existing.name;
        existing.normalized = normalized || existing.normalized;
        if (input?.color) existing.color = input.color;
        if (input?.categoryId !== undefined) existing.categoryId = input.categoryId;
        return { ...existing } as T;
      }
      const tag: Tag = {
        id: nextTagId++,
        namespace: input?.namespace || "manual",
        name,
        normalized,
        color: input?.color || mockTagColor(name),
        categoryId: input?.categoryId ?? null
      };
      mockTags.push(tag);
      return { ...tag } as T;
    }
    case "merge_tags":
      return null as T;
    case "delete_tag": {
      const tagId = Number(args?.tagId);
      mockTags = mockTags.filter((tag) => tag.id !== tagId);
      mockItems.forEach((item) => {
        item.tags = item.tags.filter((tag) => tag.id !== tagId);
      });
      return null as T;
    }
    case "delete_tag_category": {
      const categoryId = Number(args?.categoryId);
      mockCategories = mockCategories.filter((category) => category.id !== categoryId);
      mockTags.forEach((tag) => {
        if (tag.categoryId === categoryId) tag.categoryId = null;
      });
      return null as T;
    }
    case "assign_tag_category": {
      const tagId = Number(args?.tagId);
      const categoryId =
        args?.categoryId === null || args?.categoryId === undefined
          ? null
          : Number(args.categoryId);
      const tag = mockTags.find((item) => item.id === tagId);
      if (tag) tag.categoryId = categoryId;
      return tag ? ({ ...tag } as T) : (null as T);
    }
    case "update_item_tags": {
      const itemId = Number(args?.itemId);
      const tagSpecs = (args?.tagSpecs as TagInput[] | undefined) ?? [];
      const item = mockItems.find((video) => video.id === itemId);
      if (!item) return null as T;
      item.tags = tagSpecs.map((spec) => {
        const name = String(spec.name ?? "").trim();
        const normalized = name.toLowerCase();
        const existing = spec.id
          ? mockTags.find((tag) => tag.id === spec.id)
          : mockTags.find((tag) => tag.normalized.toLowerCase() === normalized);
        if (existing) return existing;
        const tag: Tag = {
          id: nextTagId++,
          namespace: spec.namespace || "manual",
          name,
          normalized,
          color: spec.color || mockTagColor(name),
          categoryId: spec.categoryId ?? null
        };
        mockTags.push(tag);
        return tag;
      });
      return {
        ...item,
        tags: item.tags.map((tag) => ({ ...tag }))
      } as T;
    }
    case "update_item_notes": {
      const itemId = Number(args?.itemId);
      const notes = String(args?.notes ?? "");
      const item = mockItems.find((video) => video.id === itemId);
      if (!item) return null as T;
      item.notes = notes;
      return {
        ...item,
        tags: item.tags.map((tag) => ({ ...tag }))
      } as T;
    }
    case "open_url":
      return null as T;
    case "create_tag_category": {
      const name = String(args?.name ?? "").trim();
      const category: TagCategory = {
        id: nextCategoryId++,
        name,
        normalized: name.toLowerCase(),
        color: String(args?.color ?? "#64748b"),
        position: 0
      };
      mockCategories.push(category);
      return { ...category } as T;
    }
    case "rename_tag_category": {
      const categoryId = Number(args?.categoryId);
      const name = String(args?.name ?? "").trim();
      const category = mockCategories.find((item) => item.id === categoryId);
      if (category) {
        category.name = name || category.name;
        category.normalized = (name || category.name).toLowerCase();
        if (args?.color) category.color = String(args.color);
      }
      return category ? ({ ...category } as T) : (null as T);
    }
    default:
      return null as T;
  }
}

function mockTagColor(name: string) {
  const colors = ["#64748b", "#0f766e", "#b45309", "#7c3aed", "#be123c"];
  const hash = name
    .toLowerCase()
    .split("")
    .reduce((total, character) => total + character.charCodeAt(0), 0);
  return colors[hash % colors.length];
}

const mockProfile: BilibiliProfile = {
  isLogin: true,
  mid: 10001,
  name: "示例用户",
  face: ""
};
