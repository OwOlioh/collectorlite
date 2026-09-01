import { invoke } from "@tauri-apps/api/core";
import type {
  BilibiliProfile,
  BrowserImportRequest,
  CollectionInfo,
  ImportPreview,
  ImportRequest,
  ImportResult,
  ItemFilters,
  RecacheResult,
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
  // 回收站
  listTrash: () => call<VideoItem[]>("list_trash", {}),
  restoreItem: (itemId: number) =>
    call<null>("restore_item", { itemId }),
  restoreItems: (itemIds: number[]) =>
    call<number>("restore_items", { itemIds }),
  purgeItem: (itemId: number) =>
    call<null>("purge_item", { itemId }),
  purgeItems: (itemIds: number[]) =>
    call<number>("purge_items", { itemIds }),
  emptyTrash: () => call<number>("empty_trash", {}),
  getTrashCount: () => call<number>("get_trash_count", {}),
  autoPurgeTrash: (retentionDays: number) =>
    call<number>("auto_purge_trash", { retentionDays }),
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
      tagSpecs: request.tagSpecs,
      itemTagAssignments: request.itemTagAssignments
    }),
  openUrl: (url: string) => call<null>("open_url", { url }),
  // 收藏库导出 / 导入
  exportCollection: (itemIds?: number[]) =>
    call<string>("export_collection", { itemIds: itemIds ?? null }),
  importCollection: (payload: string) =>
    call<ImportResult>("import_collection", { payload }),
  saveExportFile: (content: string, suggestedName: string) =>
    call<string>("save_export_file", { content, suggestedName }),
  // 维护：重新缓存封面
  recacheCovers: () => call<RecacheResult>("recache_covers", {}),
  // Zhihu
  zhihuSetCookie: (cookie: string) =>
    call<null>("zhihu_set_cookie", { cookie }),
  zhihuBrowserLogin: (cookie: string) =>
    call<BilibiliProfile>("zhihu_browser_login", { cookie }),
  zhihuLogout: () => call<null>("zhihu_logout"),
  zhihuProfile: () => call<BilibiliProfile>("zhihu_profile"),
  listZhihuCollections: () =>
    call<CollectionInfo[]>("list_zhihu_collections"),
  parseZhihuCollectionUrl: (url: string) =>
    call<CollectionInfo>("parse_zhihu_collection_url", { url }),
  previewZhihuImport: (request: ImportRequest) =>
    call<ImportPreview>("preview_zhihu_import", { input: request }),
  executeZhihuImport: (request: ImportRequest) =>
    call<ImportResult>("execute_zhihu_import", { input: request }),
  // CSDN
  listCsdnCollections: (username: string) =>
    call<CollectionInfo[]>("list_csdn_collections", { username }),
  parseCsdnCollectionUrl: (url: string) =>
    call<CollectionInfo>("parse_csdn_collection_url", { url }),
  previewCsdnImport: (request: ImportRequest) =>
    call<ImportPreview>("preview_csdn_import", { input: request }),
  executeCsdnImport: (request: ImportRequest) =>
    call<ImportResult>("execute_csdn_import", { input: request }),
  // GitHub
  listGithubStars: (username: string) =>
    call<CollectionInfo[]>("list_github_stars", { username }),
  previewGithubImport: (request: ImportRequest) =>
    call<ImportPreview>("preview_github_import", { input: request }),
  executeGithubImport: (request: ImportRequest) =>
    call<ImportResult>("execute_github_import", { input: request }),
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

const mockTrash: VideoItem[] = [];

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
    case "execute_import": {
      // 尊重白名单：仅导入 itemTagAssignments 中的项（与真实后端一致）
      const assignments = (args?.input as { itemTagAssignments?: unknown[] } | undefined)
        ?.itemTagAssignments as unknown[] | undefined;
      const count = assignments && assignments.length > 0 ? assignments.length : mockItems.length;
      return {
        runId: 1,
        total: count,
        imported: count,
        skipped: 0,
        failed: 0,
        cleanupStatus: "pending",
        errors: []
      } as T;
    }
    case "search_items":
      return mockItems.map((item) => ({
        ...item,
        tags: item.tags.map((tag) => ({ ...tag }))
      })) as T;
    case "delete_item": {
      const itemId = Number(args?.itemId);
      const index = mockItems.findIndex((item) => item.id === itemId);
      if (index >= 0) {
        const [item] = mockItems.splice(index, 1);
        mockTrash.push({ ...item, deletedAt: Math.floor(Date.now() / 1000) });
      }
      return null as T;
    }
    case "delete_items": {
      const itemIds = (args?.itemIds as number[] | undefined) ?? [];
      const ids = new Set(itemIds.map(Number));
      for (let index = mockItems.length - 1; index >= 0; index -= 1) {
        if (ids.has(mockItems[index].id)) {
          const [item] = mockItems.splice(index, 1);
          mockTrash.push({ ...item, deletedAt: Math.floor(Date.now() / 1000) });
        }
      }
      return itemIds.length as T;
    }
    case "delete_items_by_tag": {
      const tagId = Number(args?.tagId);
      const before = mockItems.length;
      for (let index = mockItems.length - 1; index >= 0; index -= 1) {
        if (mockItems[index].tags.some((tag) => tag.id === tagId)) {
          const [item] = mockItems.splice(index, 1);
          mockTrash.push({ ...item, deletedAt: Math.floor(Date.now() / 1000) });
        }
      }
      return (before - mockItems.length) as T;
    }
    case "list_trash":
      return mockTrash.map((item) => ({ ...item })) as T;
    case "restore_item": {
      const itemId = Number(args?.itemId);
      const index = mockTrash.findIndex((item) => item.id === itemId);
      if (index >= 0) {
        const [item] = mockTrash.splice(index, 1);
        const restored = { ...item };
        delete restored.deletedAt;
        mockItems.push(restored);
      }
      return null as T;
    }
    case "restore_items": {
      const itemIds = (args?.itemIds as number[] | undefined) ?? [];
      const ids = new Set(itemIds.map(Number));
      for (let index = mockTrash.length - 1; index >= 0; index -= 1) {
        if (ids.has(mockTrash[index].id)) {
          const [item] = mockTrash.splice(index, 1);
          const restored = { ...item };
          delete restored.deletedAt;
          mockItems.push(restored);
        }
      }
      return itemIds.length as T;
    }
    case "purge_item": {
      const itemId = Number(args?.itemId);
      const index = mockTrash.findIndex((item) => item.id === itemId);
      if (index >= 0) mockTrash.splice(index, 1);
      return null as T;
    }
    case "purge_items": {
      const itemIds = (args?.itemIds as number[] | undefined) ?? [];
      const ids = new Set(itemIds.map(Number));
      for (let index = mockTrash.length - 1; index >= 0; index -= 1) {
        if (ids.has(mockTrash[index].id)) mockTrash.splice(index, 1);
      }
      return itemIds.length as T;
    }
    case "empty_trash": {
      const count = mockTrash.length;
      mockTrash.length = 0;
      return count as T;
    }
    case "get_trash_count":
      return mockTrash.length as T;
    case "auto_purge_trash": {
      const retentionDays = Number(args?.retentionDays ?? 7);
      const threshold = Math.floor(Date.now() / 1000) - retentionDays * 86400;
      for (let index = mockTrash.length - 1; index >= 0; index -= 1) {
        if ((mockTrash[index].deletedAt ?? 0) < threshold) mockTrash.splice(index, 1);
      }
      return 0 as T;
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
    case "list_csdn_collections":
      return [
        {
          source: "csdn",
          id: "4050350",
          title: "默认收藏夹",
          owner: "LOVEmy134611",
          count: 1023,
          url: undefined
        }
      ] as T;
    case "parse_csdn_collection_url": {
      const url = String(args?.url ?? "");
      return {
        source: "csdn",
        id: "12345",
        title: url ? "CSDN 收藏夹" : "CSDN 收藏夹",
        owner: "testuser",
        count: 10,
        url
      } as T;
    }
    case "preview_csdn_import":
      return {
        collection: {
          source: "csdn",
          id: "4050350",
          title: "默认收藏夹",
          owner: "LOVEmy134611",
          count: 2,
          url: undefined
        },
        items: [
          {
            id: -1,
            source: "csdn",
            externalId: "164078212",
            sourceUrl: "https://blog.csdn.net/2401_83830408/article/details/164078212",
            title: "RAG 实战教程（四）：GraphRAG 查询实战",
            description: "",
            notes: undefined,
            coverUrl: undefined,
            coverLocalPath: undefined,
            authorName: "IvanCodes",
            authorId: "2401_83830408",
            partitionName: undefined,
            publishedAt: 1787801938,
            duration: undefined,
            favoriteTime: 1787801938,
            tags: []
          },
          {
            id: -2,
            source: "csdn",
            externalId: "164052452",
            sourceUrl: "https://blog.csdn.net/2301_76297596/article/details/164052452",
            title: "把音乐播放器放进 NAS：极空间部署 R3PLAYX",
            description: "",
            notes: undefined,
            coverUrl: undefined,
            coverLocalPath: undefined,
            authorName: "星辰邢哥",
            authorId: "2301_76297596",
            partitionName: undefined,
            publishedAt: 1787798756,
            duration: undefined,
            favoriteTime: 1787798756,
            tags: []
          }
        ],
        partitionSuggestions: []
      } as T;
    case "execute_csdn_import": {
      // 尊重白名单：仅导入 itemTagAssignments 中的项
      const assignments = (args?.input as { itemTagAssignments?: unknown[] } | undefined)
        ?.itemTagAssignments as unknown[] | undefined;
      const count = assignments && assignments.length > 0 ? assignments.length : 2;
      return {
        runId: 1,
        total: count,
        imported: count,
        skipped: 0,
        failed: 0,
        cleanupStatus: undefined,
        errors: []
      } as T;
    }
    case "list_github_stars":
      return [
        {
          source: "github",
          id: "starred",
          title: "OwOlioh's Stars",
          owner: "OwOlioh",
          count: 30,
          url: undefined
        }
      ] as T;
    case "preview_github_import":
      return {
        collection: {
          source: "github",
          id: "starred",
          title: "OwOlioh's Stars",
          owner: "OwOlioh",
          count: 3,
          url: undefined
        },
        items: [
          {
            id: -1,
            source: "github",
            externalId: "1074250582",
            sourceUrl: "https://github.com/CuteLeaf/Firefly",
            title: "CuteLeaf/Firefly",
            description: "Fresh and aesthetic Astro blog theme template.",
            notes: undefined,
            coverUrl: "https://avatars.githubusercontent.com/u/43440669?v=4",
            coverLocalPath: undefined,
            authorName: "CuteLeaf",
            authorId: undefined,
            partitionName: "Astro",
            duration: undefined,
            tags: []
          },
          {
            id: -2,
            source: "github",
            externalId: "1091679870",
            sourceUrl: "https://github.com/w-Steve/BUAA-Physics-Labs",
            title: "w-Steve/BUAA-Physics-Labs",
            description: "BUAA Physics Labs",
            notes: undefined,
            coverUrl: "https://avatars.githubusercontent.com/u/41997389?v=4",
            coverLocalPath: undefined,
            authorName: "w-Steve",
            authorId: undefined,
            partitionName: "HTML",
            duration: undefined,
            tags: []
          }
        ],
        partitionSuggestions: []
      } as T;
    case "execute_github_import": {
      // 尊重白名单：仅导入 itemTagAssignments 中的项
      const assignments = (args?.input as { itemTagAssignments?: unknown[] } | undefined)
        ?.itemTagAssignments as unknown[] | undefined;
      const count = assignments && assignments.length > 0 ? assignments.length : 2;
      return {
        runId: 1,
        total: count,
        imported: count,
        skipped: 0,
        failed: 0,
        errors: []
      } as T;
    }
    case "export_collection": {
      const exportObj = {
        formatVersion: 1,
        exportedAt: Math.floor(Date.now() / 1000),
        app: "bilibili_collector",
        items: mockItems.map((item) => ({
          source: item.source,
          externalId: item.externalId,
          sourceUrl: item.sourceUrl,
          title: item.title,
          description: item.description,
          coverUrl: item.coverUrl,
          authorName: item.authorName,
          authorId: item.authorId,
          partitionName: item.partitionName,
          publishedAt: item.publishedAt,
          duration: item.duration,
          favoriteTime: item.favoriteTime,
          notes: item.notes ?? "",
          extra: {},
          tags: item.tags.map((t) => ({
            namespace: t.namespace,
            name: t.name,
            color: t.color,
            category: t.categoryId ? "示例分类" : undefined
          }))
        }))
      };
      return JSON.stringify(exportObj) as T;
    }
    case "import_collection":
      return {
        runId: 1,
        total: 1,
        imported: 1,
        skipped: 0,
        failed: 0,
        errors: []
      } as T;
    case "recache_covers":
      return {
        total: 2,
        cached: 2,
        failed: 0,
        errors: []
      } as T;
    case "save_export_file":
      // 浏览器 mock 环境没有系统对话框，退化为触发下载并返回一个示意路径
      return "已保存（示例路径）" as T;
    // ── 知乎 mock（浏览器模式开发时能跑通完整导入流程） ──
    case "zhihu_set_cookie":
      return null as T;
    case "zhihu_logout":
      return null as T;
    case "zhihu_profile":
      return { isLogin: true, name: "示例知乎用户", face: null, mid: null } as T;
    case "zhihu_browser_login":
      return { isLogin: true, name: "示例知乎用户", face: null, mid: null } as T;
    case "list_zhihu_collections":
      return [
        {
          source: "zhihu",
          id: "128593041",
          title: "默认收藏夹",
          owner: "示例知乎用户",
          count: 2,
          url: undefined
        }
      ] as T;
    case "parse_zhihu_collection_url": {
      const url = String(args?.url ?? "");
      return {
        source: "zhihu",
        id: "128593041",
        title: url ? "知乎收藏夹" : "知乎收藏夹",
        owner: "示例知乎用户",
        count: 2,
        url
      } as T;
    }
    case "preview_zhihu_import":
      return {
        collection: {
          source: "zhihu",
          id: "128593041",
          title: "默认收藏夹",
          owner: "示例知乎用户",
          count: 2,
          url: undefined
        },
        items: [
          {
            id: -1,
            source: "zhihu",
            externalId: "zhihu-a1",
            sourceUrl: "https://www.zhihu.com/question/1/answer/1001",
            title: "如何系统地建立知识管理体系？",
            description: "",
            notes: undefined,
            coverUrl: undefined,
            coverLocalPath: undefined,
            authorName: "知乎答主A",
            authorId: undefined,
            partitionName: undefined,
            publishedAt: 1787700000,
            duration: undefined,
            favoriteTime: 1787700000,
            tags: []
          },
          {
            id: -2,
            source: "zhihu",
            externalId: "zhihu-a2",
            sourceUrl: "https://www.zhihu.com/pin/2002",
            title: "关于阅读与笔记的一些实践",
            description: "",
            notes: undefined,
            coverUrl: undefined,
            coverLocalPath: undefined,
            authorName: "知乎答主B",
            authorId: undefined,
            partitionName: undefined,
            publishedAt: 1787600000,
            duration: undefined,
            favoriteTime: 1787600000,
            tags: []
          }
        ],
        partitionSuggestions: []
      } as T;
    case "execute_zhihu_import": {
      // 尊重白名单：仅导入 itemTagAssignments 中的项
      const assignments = (args?.input as { itemTagAssignments?: unknown[] } | undefined)
        ?.itemTagAssignments as unknown[] | undefined;
      const count = assignments && assignments.length > 0 ? assignments.length : 2;
      return {
        runId: 1,
        total: count,
        imported: count,
        skipped: 0,
        failed: 0,
        errors: []
      } as T;
    }
    // ── 浏览器书签 mock ──
    case "import_browser_bookmarks": {
      const assignments = (args?.itemTagAssignments as unknown[] | undefined) ?? [];
      const html = String(args?.htmlContent ?? "");
      const estimated = html.match(/<a\s/gi)?.length ?? 0;
      const count = assignments.length > 0 ? assignments.length : estimated;
      return {
        runId: 1,
        total: count,
        imported: count,
        skipped: 0,
        failed: 0,
        errors: []
      } as T;
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
