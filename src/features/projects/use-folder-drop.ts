import { useEffect, useRef, useState } from "react";
import { listenForFolderDrops } from "@/lib/native";

/**
 * Listen for folders dropped on the window while the page is open. Returns whether something is being dragged over
 * the window right now, so the page can show where to drop it.
 */
export function useFolderDrop(onDrop: (paths: string[]) => void): boolean {
  const [hovering, setHovering] = useState(false);
  // The newest callback without resubscribing on every render.
  const latest = useRef(onDrop);
  useEffect(() => {
    latest.current = onDrop;
  });

  useEffect(() => {
    // An object, not a plain boolean: it is set from the cleanup function while the async setup runs.
    const lifecycle = { disposed: false };
    let unsubscribe: (() => void) | undefined;
    void listenForFolderDrops({
      onHover: setHovering,
      onDrop: (paths) => {
        latest.current(paths);
      },
    }).then((off) => {
      if (lifecycle.disposed) off();
      else unsubscribe = off;
    });
    return () => {
      lifecycle.disposed = true;
      unsubscribe?.();
    };
  }, []);

  return hovering;
}
