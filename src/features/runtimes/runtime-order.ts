import type { RuntimeCatalog } from "@/bindings/RuntimeCatalog";

// The runtimes people reach for first. The backend lists runtimes by name, which would put Caddy first.
const PREFERRED = ["php", "node", "composer", "mysql", "mariadb", "caddy"];

/** The main runtimes first, in a fixed order, then any other by name. */
export function orderRuntimes(runtimes: readonly RuntimeCatalog[]): RuntimeCatalog[] {
  const rank = (id: string) => {
    const index = PREFERRED.indexOf(id);
    return index === -1 ? PREFERRED.length : index;
  };
  return [...runtimes].sort(
    (a, b) => rank(a.id) - rank(b.id) || a.displayName.localeCompare(b.displayName),
  );
}
