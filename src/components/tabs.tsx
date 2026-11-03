import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";

import { cn } from "@/lib/utils";

export interface TabItem {
  id: string;
  label: string;
  /** A small count or hint next to the label. */
  trailing?: ReactNode;
}

interface TabsProps {
  items: readonly TabItem[];
  value: string;
  onChange: (id: string) => void;
  /** Names the tab list for screen readers. */
  label: string;
  /** The panel that belongs to the selected tab. */
  children: ReactNode;
}

/** Tabs with the keyboard behaviour screen readers expect: arrow keys move, Home/End jump, one tab stop. */
export function Tabs({ items, value, onChange, label, children }: TabsProps) {
  const base = useId();
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const tabId = (id: string) => `${base}-tab-${id}`;
  const panelId = `${base}-panel`;

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const index = items.findIndex((item) => item.id === value);
    const last = items.length - 1;
    let next: number;
    switch (event.key) {
      case "ArrowRight":
        next = index >= last ? 0 : index + 1;
        break;
      case "ArrowLeft":
        next = index <= 0 ? last : index - 1;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = last;
        break;
      default:
        return;
    }
    event.preventDefault();
    const target = items[next];
    if (!target) return;
    onChange(target.id);
    refs.current.get(target.id)?.focus();
  }

  return (
    <div>
      <div
        role="tablist"
        aria-label={label}
        onKeyDown={onKeyDown}
        // overflow-y is clipped on purpose: the selected tab overlaps the border by 1px, which would add a vertical scrollbar.
        className="mb-6 flex gap-1 overflow-x-auto overflow-y-hidden border-b [scrollbar-width:thin]"
      >
        {items.map((item) => {
          const selected = item.id === value;
          return (
            <button
              key={item.id}
              ref={(node) => {
                if (node) refs.current.set(item.id, node);
                else refs.current.delete(item.id);
              }}
              type="button"
              role="tab"
              id={tabId(item.id)}
              aria-selected={selected}
              aria-controls={panelId}
              tabIndex={selected ? 0 : -1}
              onClick={() => {
                onChange(item.id);
              }}
              className={cn(
                "-mb-px flex items-center gap-2 border-b-2 px-3 py-2 text-sm font-medium whitespace-nowrap outline-none focus-visible:rounded-t-md focus-visible:ring-[3px] focus-visible:ring-ring/50",
                selected
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground",
              )}
            >
              {item.label}
              {item.trailing}
            </button>
          );
        })}
      </div>
      <div role="tabpanel" id={panelId} aria-labelledby={tabId(value)}>
        {children}
      </div>
    </div>
  );
}
