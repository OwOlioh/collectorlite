import { useEffect, useRef } from "react";
import { api } from "../lib/api";
import { useToast } from "./Toast";
import {
  buildBackupFileName,
  getBackupSettings,
  isBackupDue,
  setBackupSettings
} from "../lib/backup";

const CHECK_INTERVAL_MS = 60 * 60 * 1000; // 每小时复查一次是否到期

/**
 * 自动备份常驻调度器（无 UI，挂在 ToastProvider 内以便弹提示）。
 * 应用启动时检查一次，此后每小时复查：启用且距上次成功备份 ≥ 3 天 → 立即备份整库。
 * 失败静默，下一轮自动重试，不打断用户。
 */
export function AutoBackupRunner() {
  const { toast } = useToast();
  const runningRef = useRef(false);
  const lastToastAtRef = useRef(0);

  useEffect(() => {
    let disposed = false;
    const attempt = async () => {
      if (runningRef.current) return;
      const settings = getBackupSettings();
      if (!isBackupDue(settings)) return;
      runningRef.current = true;
      try {
        const path = await api.backupNow(settings.folder, buildBackupFileName());
        if (disposed) return;
        setBackupSettings({ ...getBackupSettings(), lastRunAt: Date.now() });
        const now = Date.now();
        if (now - lastToastAtRef.current > 60_000) {
          lastToastAtRef.current = now;
          toast("info", `已完成自动备份：${path}`);
        }
      } catch (error) {
        console.warn("自动备份失败，将稍后重试：", error);
      } finally {
        runningRef.current = false;
      }
    };

    void attempt();
    const timer = window.setInterval(() => void attempt(), CHECK_INTERVAL_MS);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
    // toast 引用由 ToastProvider 保证稳定；effect 只在挂载时注册一次调度器
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return null;
}
