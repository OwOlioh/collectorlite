import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { inTauri } from "../lib/api";
import { useToast } from "./Toast";
import type { NeteaseSyncReport } from "../types";

interface NeteaseSyncListenerProps {
  /** 后台同步改动了库内容时触发：让收藏库重新拉列表。 */
  onSynced?: () => void;
}

const NETEASE_SYNC_EVENT = "netease://sync";

/**
 * 监听网易云后台自动同步的结果。
 *
 * 只在**确实有变化**时才提示并刷新列表 —— 绝大多数轮次都是「0 新增 0 移除」，
 * 每 15 分钟弹一次「同步完成」纯属打扰。手动点「立即同步」由设置页自己提示。
 *
 * 必须挂在 ToastProvider 内部才能用 useToast，所以做成独立组件。
 */
export function NeteaseSyncListener({ onSynced }: NeteaseSyncListenerProps) {
  const { toast } = useToast();

  useEffect(() => {
    if (!inTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;

    void listen<NeteaseSyncReport>(NETEASE_SYNC_EVENT, (event) => {
      const report = event.payload;
      if (!report) return;
      const removedHint =
        report.removed > 0 ? `，${report.removed} 首已取消收藏并移入回收站` : "";
      toast("success", `网易云同步：新增 ${report.added} 首${removedHint}`);
      if (report.errors.length > 0) {
        toast("error", report.errors.slice(0, 3).join("; "));
      }
      onSynced?.();
    })
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {
        /* 事件通道不可用时静默降级：同步本身不依赖前端 */
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [toast, onSynced]);

  return null;
}
