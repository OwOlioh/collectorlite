import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, inTauri } from "../lib/api";
import { useToast } from "./Toast";

interface CoverCacheListenerProps {
  /** 一轮封面缓存结束后触发：让收藏库重新拉列表，好把新落地的本地封面显示出来。 */
  onCoversCached?: () => void;
}

interface CoverCacheProgress {
  total: number;
  done: number;
  cached: number;
  failed: number;
  running: boolean;
}

const COVER_CACHE_EVENT = "cover-cache://progress";

/**
 * 监听后台封面缓存任务。
 *
 * 导入时数据先落库、封面交给后台补，所以这里负责把后台的进展告诉用户：
 *  - 启动后发现队列里还有上次没缓存完的 → 提示「正在继续缓存」；
 *  - 一轮结束 → 提示结果并触发收藏库刷新（本地封面要重新拉列表才显示）。
 *
 * 必须挂在 ToastProvider 内部才能用 useToast，所以做成独立组件。
 */
export function CoverCacheListener({ onCoversCached }: CoverCacheListenerProps) {
  const { toast } = useToast();
  // 标记「本轮是否已经提醒过用户」。开始提示分两种来源：
  //  - 导入触发的由 ImportPage 自己提示（它知道导入了多少条）；
  //  - 启动续传的由下面第二个 effect 提示。
  // 这里只负责收尾，避免同一个进度被提示两遍。
  const activeRef = useRef(false);

  useEffect(() => {
    if (!inTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;

    void listen<CoverCacheProgress>(COVER_CACHE_EVENT, (event) => {
      const p = event.payload;
      if (!p) return;

      if (p.running) {
        activeRef.current = true;
        return;
      }

      // 一轮结束
      if (!activeRef.current) return;
      activeRef.current = false;
      if (p.failed > 0) {
        toast(
          "info",
          `封面缓存完成：成功 ${p.cached} 张，${p.failed} 张失败（下次启动会自动重试）`
        );
      } else if (p.cached > 0) {
        toast("success", `封面缓存完成：${p.cached} 张已存到本地`);
      }
      onCoversCached?.();
    })
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {
        /* 事件通道不可用时静默降级：缓存本身不依赖前端 */
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [toast, onCoversCached]);

  // 启动时查一次：上次没缓存完的封面会由 Rust 端自动续传，这里只负责告诉用户
  useEffect(() => {
    if (!inTauri()) return;
    let cancelled = false;
    void api
      .coverCacheStatus()
      .then((status) => {
        if (cancelled || status.pending <= 0) return;
        toast(
          "info",
          `还有 ${status.pending} 张封面未缓存，正在后台继续（关闭应用也不会丢，下次启动会接着缓存）`
        );
        // 后台任务已经在跑（Rust 端启动时会自动拉起），标记本轮进行中，等结束再提示
        activeRef.current = true;
      })
      .catch(() => {
        /* 查不到就算了 */
      });
    return () => {
      cancelled = true;
    };
  }, [toast]);

  return null;
}
