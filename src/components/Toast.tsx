import {
  createContext,
  useCallback,
  useContext,
  useState,
  type ReactNode
} from "react";
import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";

type ToastType = "success" | "error" | "info";

interface ToastAction {
  label: string;
  onClick: () => void;
}

interface ToastItem {
  id: number;
  type: ToastType;
  message: string;
  action?: ToastAction;
}

interface ToastOptions {
  action?: ToastAction;
  duration?: number;
}

interface ToastContextValue {
  toast: (type: ToastType, message: string, opts?: ToastOptions) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

let counter = 0;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);

  const remove = useCallback((id: number) => {
    setItems((current) => current.filter((item) => item.id !== id));
  }, []);

  const toast = useCallback<ToastContextValue["toast"]>(
    (type, message, opts) => {
      const id = ++counter;
      const duration = opts?.duration ?? 3200;
      setItems((current) => [
        ...current,
        { id, type, message, action: opts?.action }
      ]);
      if (duration > 0) {
        window.setTimeout(() => remove(id), duration);
      }
    },
    [remove]
  );

  return (
    <ToastContext.Provider value={{ toast }}>
      {children}
      <div className="toast-viewport" role="region" aria-label="通知">
        {items.map((item) => (
          <div key={item.id} className={`toast toast-${item.type}`} role="status">
            <span className="toast-icon">
              {item.type === "success" ? (
                <CheckCircle2 size={18} />
              ) : item.type === "error" ? (
                <AlertCircle size={18} />
              ) : (
                <Info size={18} />
              )}
            </span>
            <span className="toast-message">{item.message}</span>
            {item.action && (
              <button
                className="toast-action"
                type="button"
                onClick={() => {
                  item.action?.onClick();
                  remove(item.id);
                }}
              >
                {item.action.label}
              </button>
            )}
            <button
              className="toast-close icon-button"
              type="button"
              onClick={() => remove(item.id)}
              aria-label="关闭"
            >
              <X size={14} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within a ToastProvider");
  return ctx;
}
