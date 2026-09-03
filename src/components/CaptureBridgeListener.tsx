import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { inTauri } from "../lib/api";
import { useToast } from "./Toast";

interface CaptureBridgeListenerProps {
  /** 收到入库事件后触发：刷新标签池、回收站计数与收藏库列表。 */
  onCaptured?: () => void;
}

interface CapturePayload {
  title?: string;
}

/**
 * 浏览器扩展通过本地桥写入条目后，Rust 端会广播 `capture://saved`。
 * 这里只负责「提示 + 触发刷新」，不参与写入——桥是直连数据库的，
 * 即使应用窗口没打开也已经入库成功。
 *
 * 必须挂在 ToastProvider 内部才能用 useToast，所以做成独立组件。
 */
export function CaptureBridgeListener({ onCaptured }: CaptureBridgeListenerProps) {
  const { toast } = useToast();

  useEffect(() => {
    if (!inTauri()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;

    void listen<CapturePayload>("capture://saved", (event) => {
      const title = event.payload?.title?.trim();
      toast("success", title ? `已收藏：${title}` : "已通过浏览器扩展收藏");
      onCaptured?.();
    })
      .then((fn) => {
        // 组件在 listen 完成前就卸载了，立刻解绑避免泄漏
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {
        /* 事件通道不可用时静默降级：入库本身不依赖前端 */
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [toast, onCaptured]);

  return null;
}
