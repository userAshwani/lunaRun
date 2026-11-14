// The two things the webview cannot do by itself: ask the OS for a folder, and hear that a folder was dropped on the
// window. Kept in one place so pages and tests mock one module, and so nothing here runs in a plain browser.
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

/** The native folder picker. `null` when the user closes it without choosing. */
export async function pickFolder(title: string): Promise<string | null> {
  const chosen = await open({ directory: true, multiple: false, title });
  return typeof chosen === "string" ? chosen : null;
}

export interface FolderDropHandlers {
  /** Something is being dragged over the window (`true`) or left it (`false`). */
  onHover: (over: boolean) => void;
  /** Paths were dropped. They may be files: the backend refuses anything that is not a folder. */
  onDrop: (paths: string[]) => void;
}

/** Listen for drag and drop on the window. Returns the function that stops listening. */
export async function listenForFolderDrops(handlers: FolderDropHandlers): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type === "enter" || payload.type === "over") {
      handlers.onHover(true);
    } else if (payload.type === "leave") {
      handlers.onHover(false);
    } else {
      handlers.onHover(false);
      handlers.onDrop(payload.paths);
    }
  });
}
