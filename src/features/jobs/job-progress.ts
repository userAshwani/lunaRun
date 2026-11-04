import { useTranslation } from "react-i18next";
import type { JobInfo } from "@/bindings/JobInfo";
import { formatBytes } from "@/lib/format";
import { percent } from "./jobs-state";

const KNOWN_PHASES = [
  "starting",
  "preparing",
  "downloading",
  "verifying",
  "extracting",
  "finishing",
  "removing",
] as const;
type KnownPhase = (typeof KNOWN_PHASES)[number];

function isKnownPhase(phase: string): phase is KnownPhase {
  return (KNOWN_PHASES as readonly string[]).includes(phase);
}

export interface JobProgressText {
  /** "Downloading". A phase this build does not know (from a newer backend) is shown as it is. */
  phase: string;
  /** "12.3 MB of 18 MB", "3 MB so far", or `null` before any byte moved. */
  bytes: string | null;
  /** 0 to 100, or `null` when the total is unknown. */
  value: number | null;
}

/** Turns a running job's numbers into the words and percentage every progress display uses. */
export function useJobProgressText(): (job: JobInfo) => JobProgressText {
  const { t } = useTranslation();
  return (job) => {
    const bytes =
      job.bytesTotal !== null
        ? t("jobs.bytes.of", {
            done: formatBytes(job.bytesDone),
            total: formatBytes(job.bytesTotal),
          })
        : job.bytesDone > 0
          ? t("jobs.bytes.done", { done: formatBytes(job.bytesDone) })
          : null;
    return {
      phase: isKnownPhase(job.phase) ? t(`jobs.phase.${job.phase}`) : job.phase,
      bytes,
      value: percent(job.bytesDone, job.bytesTotal),
    };
  };
}
