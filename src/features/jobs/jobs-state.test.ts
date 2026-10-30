import { describe, expect, it } from "vitest";
import type { JobInfo } from "@/bindings/JobInfo";
import { applyProgress, percent } from "./jobs-state";

const job = (overrides: Partial<JobInfo> = {}): JobInfo => ({
  id: "job-1",
  key: "php@8.3.14",
  label: "Install PHP 8.3.14",
  state: "running",
  phase: "download",
  bytesDone: 10,
  bytesTotal: 100,
  message: null,
  error: null,
  ...overrides,
});

describe("applyProgress", () => {
  const event = { jobId: "job-1", phase: "extract", bytesDone: 50, bytesTotal: 200, message: "x" };

  it("updates only the matching running job", () => {
    const other = job({ id: "job-2", bytesDone: 7 });
    const result = applyProgress([job(), other], event);
    expect(result?.[0]).toMatchObject({
      phase: "extract",
      bytesDone: 50,
      bytesTotal: 200,
      message: "x",
    });
    expect(result?.[1]).toEqual(other);
  });

  it("does not mutate the cached list", () => {
    const list = [job()];
    applyProgress(list, event);
    expect(list[0]?.bytesDone).toBe(10);
  });

  it("ignores a late event for a job that already finished", () => {
    const finished = job({ state: "succeeded", bytesDone: 100 });
    expect(applyProgress([finished], event)?.[0]).toEqual(finished);
  });

  it("returns undefined for an unknown job or an empty cache, so the caller refetches", () => {
    expect(applyProgress([job()], { ...event, jobId: "job-9" })).toBeUndefined();
    expect(applyProgress(undefined, event)).toBeUndefined();
    expect(applyProgress([], event)).toBeUndefined();
  });
});

describe("percent", () => {
  it("is a whole number between 0 and 100", () => {
    expect(percent(0, 200)).toBe(0);
    expect(percent(50, 200)).toBe(25);
    expect(percent(199, 200)).toBe(99);
    expect(percent(200, 200)).toBe(100);
    expect(percent(500, 200)).toBe(100);
    expect(percent(-5, 200)).toBe(0);
  });

  it("is null when the total is unknown or zero", () => {
    expect(percent(50, null)).toBeNull();
    expect(percent(50, 0)).toBeNull();
  });
});
