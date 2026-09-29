import { create } from "zustand";
import { ApiError } from "../lib/api";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
}

interface ToastStore {
  toasts: Toast[];
  show: (text: string, kind?: Toast["kind"]) => void;
  dismiss: (id: number) => void;
}

let nextId = 1;

export const useToasts = create<ToastStore>((set, get) => ({
  toasts: [],
  show: (text, kind = "info") => {
    const id = nextId++;
    set({ toasts: [...get().toasts, { id, text, kind }] });
    setTimeout(() => get().dismiss(id), kind === "error" ? 6000 : 3000);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
}));

export function toast(text: string): void {
  useToasts.getState().show(text, "info");
}

/** Show an error from any failed command. Hibernate errors get a clear hint. */
export function toastError(e: unknown): void {
  const msg =
    e instanceof ApiError
      ? e.code === "hibernating"
        ? "This needs Standard mode. Switch from the mode badge."
        : e.message
      : String(e);
  useToasts.getState().show(msg, "error");
}
