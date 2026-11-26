// Notifications shown at the bottom of the preview (docs/ux.md, "Notifications").
// At most three at once; each disappears after a few seconds, longer when it
// offers an action.

export type ToastTone = "success" | "info" | "error";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  tone: ToastTone;
  message: string;
  details?: string;
  action?: ToastAction;
}

const MAXIMUM_VISIBLE_TOASTS = 3;
const PLAIN_TOAST_DURATION_IN_MILLISECONDS = 5_000;
const TOAST_WITH_ACTION_DURATION_IN_MILLISECONDS = 8_000;
const ERROR_TOAST_DURATION_IN_MILLISECONDS = 10_000;

class Toasts {
  visible = $state<Toast[]>([]);
  private nextId = 1;

  show(toast: Omit<Toast, "id">): void {
    const id = this.nextId++;
    this.visible = [...this.visible, { ...toast, id }].slice(-MAXIMUM_VISIBLE_TOASTS);
    const duration =
      toast.tone === "error"
        ? ERROR_TOAST_DURATION_IN_MILLISECONDS
        : toast.action
          ? TOAST_WITH_ACTION_DURATION_IN_MILLISECONDS
          : PLAIN_TOAST_DURATION_IN_MILLISECONDS;
    window.setTimeout(() => this.dismiss(id), duration);
  }

  dismiss(id: number): void {
    this.visible = this.visible.filter((toast) => toast.id !== id);
  }
}

export const toasts = new Toasts();
