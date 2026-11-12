import { Info } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ServiceRow } from "@/bindings/ServiceRow";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableScroll,
} from "@/components/ui/table";
import { formatPorts, serviceName } from "./service-format";
import { ServiceStateBadge } from "./service-state-badge";

/** The second line under a service's name: its version, or what kind of PHP it is. */
function useDetail() {
  const { t } = useTranslation();
  return (row: ServiceRow): string => {
    if (row.kind === "php") {
      // On Windows a pool is several `php-cgi` workers; elsewhere it is one PHP-FPM.
      return row.id.startsWith("php-fcgi@")
        ? t("services.detail.workers", { count: row.total })
        : t("services.detail.fpm");
    }
    return t("services.detail.version", { version: row.version });
  };
}

/** What is worth saying about a service beyond its state, each as one plain sentence. */
function useNotes() {
  const { t } = useTranslation();
  return (row: ServiceRow): string[] => {
    const notes: string[] = [];
    // Only when it is not everything: "Running" already says that all of them are.
    if (row.total > 1 && row.state !== "stopped" && row.running !== row.total) {
      notes.push(t("services.detail.runningOf", { running: row.running, total: row.total }));
    }
    // Why it is not running, in the backend's own words.
    if (row.state === "error" || row.state === "crashed") {
      const failed = row.members.find((member) => member.error)?.error;
      if (failed) notes.push(failed.message);
    }
    if (row.members.some((member) => member.gaveUp)) notes.push(t("services.note.gaveUp"));
    if (row.members.some((member) => member.killed)) notes.push(t("services.note.killed"));
    if (row.needsSetup && row.state === "stopped") notes.push(t("services.note.needsSetup"));
    return notes;
  };
}

export function ServicesTable({ rows }: { rows: readonly ServiceRow[] }) {
  const { t } = useTranslation();
  const detail = useDetail();
  const notes = useNotes();

  return (
    // This table is the page, not one card among several: it takes the height it needs and the page scrolls.
    <TableScroll label={t("services.tableLabel")} className="max-h-none">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead>{t("services.columns.service")}</TableHead>
            <TableHead>{t("services.columns.ports")}</TableHead>
            <TableHead>{t("services.columns.status")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row) => (
            <TableRow key={row.id}>
              <TableCell className="align-top">
                {/* Never broken across lines: the long notes in the status column wrap instead. */}
                <p className="font-medium whitespace-nowrap">{serviceName(row)}</p>
                <p className="text-[13px] whitespace-nowrap text-muted-foreground tabular-nums">
                  {detail(row)}
                </p>
              </TableCell>
              <TableCell className="align-top whitespace-nowrap tabular-nums">
                {row.ports.length > 0 ? (
                  formatPorts(row.ports)
                ) : (
                  <span className="text-muted-foreground" title={t("services.ports.noneHint")}>
                    {t("services.ports.none")}
                  </span>
                )}
              </TableCell>
              <TableCell className="align-top">
                <ServiceStateBadge state={row.state} />
                {notes(row).map((note) => (
                  <p key={note} className="mt-1 max-w-md text-[13px] text-muted-foreground">
                    {note}
                  </p>
                ))}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableScroll>
  );
}

/** A line of small print under the table, with an icon so it reads as a note and not as data. */
export function ServicesNote({ children }: { children: string }) {
  return (
    <p className="flex items-start gap-2 text-[13px] text-muted-foreground">
      <Info className="mt-0.5 size-4 shrink-0" aria-hidden />
      <span>{children}</span>
    </p>
  );
}
