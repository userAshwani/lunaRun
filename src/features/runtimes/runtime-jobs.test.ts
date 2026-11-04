import { describe, expect, it } from "vitest";
import { runtimeJob } from "@/test/runtimes-fixtures";
import { jobKey, jobKind, latestRuntimeJobs, parseJobKey } from "./runtime-jobs";

describe("job keys", () => {
  it("round-trips a runtime and a version", () => {
    expect(jobKey("php", "8.3.27")).toBe("php@8.3.27");
    expect(parseJobKey("php@8.3.27")).toEqual({ runtime: "php", version: "8.3.27" });
    expect(parseJobKey("mysql@8.4.6")).toEqual({ runtime: "mysql", version: "8.4.6" });
  });

  it("refuses keys that are not about one runtime version", () => {
    for (const key of ["download", "@1.0", "php@", ""]) expect(parseJobKey(key)).toBeNull();
  });
});

describe("jobKind", () => {
  it("tells an install from an uninstall by the label the backend gives it", () => {
    expect(jobKind(runtimeJob("php", "8.3.27", { label: "Install PHP 8.3.27" }))).toBe("install");
    expect(jobKind(runtimeJob("php", "8.3.27", { label: "Uninstall php 8.3.27" }))).toBe(
      "uninstall",
    );
  });

  it("ignores jobs that are not runtime jobs", () => {
    expect(jobKind(runtimeJob("php", "8.3.27", { label: "Compact the cache" }))).toBeNull();
    expect(jobKind(runtimeJob("php", "8.3.27", { key: "cache-compact" }))).toBeNull();
  });
});

describe("latestRuntimeJobs", () => {
  it("keeps only the newest job of each version, which the list gives first", () => {
    const newest = runtimeJob("php", "8.3.27", { id: "new", state: "succeeded" });
    const older = runtimeJob("php", "8.3.27", { id: "old", state: "failed" });
    const other = runtimeJob("node", "24.1.0", { id: "node-job" });
    const foreign = runtimeJob("php", "8.3.27", { id: "x", key: "cache", label: "Compact" });

    const latest = latestRuntimeJobs([newest, other, foreign, older]);

    expect([...latest.keys()].sort()).toEqual(["node@24.1.0", "php@8.3.27"]);
    expect(latest.get("php@8.3.27")?.id).toBe("new");
  });
});
