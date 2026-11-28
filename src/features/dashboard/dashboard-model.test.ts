import { describe, expect, it } from "vitest";
import type { ErrorCode } from "@/bindings/ErrorCode";
import type { JobInfo } from "@/bindings/JobInfo";
import type { LunaError } from "@/bindings/LunaError";
import { project, siteEntry } from "@/test/projects-fixtures";
import { caddyRow, mysqlRow, phpPool, row, status } from "@/test/services-fixtures";
import {
  QUICK_PROJECTS,
  activeServices,
  currentProblems,
  jobPage,
  quickProjects,
  type Problem,
} from "./dashboard-model";

const err = (code: ErrorCode, message: string): LunaError => ({ code, message });

function job(id: string, key: string, state: JobInfo["state"], label = `Task ${id}`): JobInfo {
  return {
    id,
    key,
    label,
    state,
    phase: "",
    bytesDone: 0,
    bytesTotal: null,
    message: null,
    error: state === "failed" ? err("IO", `${id} broke`) : null,
  };
}

const kinds = (problems: Problem[]) =>
  problems.map((p) =>
    p.kind === "service" ? `service ${p.row.id}` : p.kind === "job" ? `job ${p.job.id}` : "sites",
  );

describe("quickProjects", () => {
  it("lists the served projects first, then the ones changed last, then by name", () => {
    const old = project("old", { name: "Old", updatedAt: 100 });
    const fresh = project("fresh", { name: "Fresh", updatedAt: 300 });
    const servedOld = project("served", { name: "Served", updatedAt: 50 });
    const alpha = project("alpha", { name: "Alpha", updatedAt: 100 });
    const sites = [siteEntry("served", { served: true }), siteEntry("fresh")];
    expect(quickProjects([old, fresh, servedOld, alpha], sites).map((p) => p.id)).toEqual([
      "served",
      "fresh",
      "alpha",
      "old",
    ]);
  });

  it("shows five at most, and works before the sites are read", () => {
    const many = Array.from({ length: 7 }, (_, n) => project(`p${String(n)}`, { updatedAt: n }));
    const shown = quickProjects(many, undefined);
    expect(shown).toHaveLength(QUICK_PROJECTS);
    expect(shown[0]?.id).toBe("p6");
  });
});

describe("activeServices", () => {
  it("keeps what runs or is on its way up or down; a stopped or broken service is not listed", () => {
    const rows = [
      row("a", { kind: "web", runtime: "caddy", version: "1", state: "running" }),
      row("b", { kind: "web", runtime: "caddy", version: "1", state: "stopping" }),
      row("c", { kind: "web", runtime: "caddy", version: "1", state: "crashed" }),
      row("d", { kind: "web", runtime: "caddy", version: "1", state: "error" }),
      caddyRow,
      phpPool,
    ];
    expect(activeServices(rows).map((r) => r.id)).toEqual(["a", "b", phpPool.id]);
  });
});

describe("currentProblems", () => {
  it("is empty when everything is healthy", () => {
    expect(currentProblems([caddyRow, mysqlRow], [], null, new Set())).toEqual([]);
  });

  it("names a crashed service, one whose restarts ran out, and one that stopped with an error", () => {
    const error = err("SERVICE_START_FAILED", "mysqld exited");
    const crashed = {
      ...caddyRow,
      state: "crashed" as const,
      members: [status("caddy", { state: "crashed" })],
    };
    const gaveUp = {
      ...mysqlRow,
      state: "crashed" as const,
      members: [status("mysql@8.4", { state: "crashed", gaveUp: true, error })],
    };
    // A Windows pool with one worker in error: the row is told once, with that worker's error.
    const pool = {
      ...phpPool,
      members: [
        status("php-fcgi@8.3.36-w1", { state: "running" }),
        status("php-fcgi@8.3.36-w2", { state: "error", error }),
      ],
    };
    const problems = currentProblems([crashed, gaveUp, pool], [], null, new Set());
    expect(problems).toEqual([
      { kind: "service", row: crashed, crashed: true, gaveUp: false, error: null },
      { kind: "service", row: gaveUp, crashed: true, gaveUp: true, error },
      { kind: "service", row: pool, crashed: false, gaveUp: false, error },
    ]);
  });

  it("counts a failed task only while it is the last attempt of its key", () => {
    const jobs = [
      // Running first, then finished newest first, as the backend lists them.
      job("retrying", "node@24.1.0", "running"),
      job("fixed", "php@8.3.14", "succeeded"),
      job("old-failure", "php@8.3.14", "failed"),
      job("still", "mysql@8.4.13", "failed"),
      job("earlier", "node@24.1.0", "failed"),
      job("cancelled", "caddy@2.11.7", "cancelled"),
    ];
    expect(kinds(currentProblems([], jobs, null, new Set()))).toEqual(["job still"]);
  });

  it("tells a service's failed start once, by the service, and a listed project's failed start by its row", () => {
    const error = err("SERVICE_START_FAILED", "no");
    const broken = {
      ...phpPool,
      members: [status("php-fcgi@8.3.36-w1", { state: "error", error })],
    };
    const jobs = [
      job("pool", "service:php-fcgi@8.3.36", "failed"),
      job("caddy", "service:caddy", "failed"),
      job("shop", "project:shop", "failed"),
      job("blog", "project:blog", "failed"),
    ];
    expect(kinds(currentProblems([broken, caddyRow], jobs, null, new Set(["shop"])))).toEqual([
      `service ${phpPool.id}`,
      "job caddy",
      "job blog",
    ]);
    const blog = currentProblems([], jobs.slice(3), null, new Set())[0];
    expect(blog).toMatchObject({ kind: "job", projectId: "blog" });
  });

  it("adds the web server refusing the sites last", () => {
    const refused = err("RELOAD_FAILED", "Caddyfile:3 bad");
    expect(kinds(currentProblems([], [job("x", "php@1", "failed")], refused, new Set()))).toEqual([
      "job x",
      "sites",
    ]);
  });
});

describe("jobPage", () => {
  it("sends each failed task to the page where it is fixed", () => {
    expect(jobPage(job("1", "project:shop", "failed"))).toBe("/projects/shop");
    expect(jobPage(job("2", "service:mysql@8.4", "failed"))).toBe("/services");
    expect(jobPage(job("3", "php@8.3.14", "failed"))).toBe("/runtimes");
  });
});
