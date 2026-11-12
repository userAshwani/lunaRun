import { describe, expect, it } from "vitest";
import { serviceJob } from "@/test/services-fixtures";
import { latestServiceJobs, serviceJobAction, serviceOfJob } from "./service-jobs";

describe("serviceOfJob", () => {
  it("reads the service from a service job's key", () => {
    expect(serviceOfJob({ key: "service:mysql@8.4" })).toBe("mysql@8.4");
    expect(serviceOfJob({ key: "service:php-fcgi@8.3.36" })).toBe("php-fcgi@8.3.36");
    expect(serviceOfJob({ key: "service:caddy" })).toBe("caddy");
  });

  it("is null for a job of another kind or with no service", () => {
    expect(serviceOfJob({ key: "php@8.4.2" })).toBeNull();
    expect(serviceOfJob({ key: "service:" })).toBeNull();
    expect(serviceOfJob({ key: "" })).toBeNull();
  });
});

describe("serviceJobAction", () => {
  it("tells start, stop and restart apart by the label", () => {
    expect(serviceJobAction(serviceJob("caddy", "Start"))).toBe("start");
    expect(serviceJobAction(serviceJob("caddy", "Stop"))).toBe("stop");
    expect(serviceJobAction(serviceJob("caddy", "Restart"))).toBe("restart");
  });

  it("is null for a service key with an unknown label, and for another kind of job", () => {
    expect(serviceJobAction({ ...serviceJob("caddy", "Start"), label: "Dance caddy" })).toBeNull();
    expect(
      serviceJobAction({ ...serviceJob("caddy", "Start"), key: "php@8.4.2", label: "Start php" }),
    ).toBeNull();
  });
});

describe("latestServiceJobs", () => {
  it("keeps the first (newest) job of each service", () => {
    const newest = serviceJob("caddy", "Restart", { id: "new" });
    const older = serviceJob("caddy", "Start", { id: "old", state: "succeeded" });
    const other = serviceJob("mysql@8.4", "Start", { id: "db" });

    const latest = latestServiceJobs([newest, other, older]);

    expect([...latest.keys()]).toEqual(["caddy", "mysql@8.4"]);
    expect(latest.get("caddy")?.id).toBe("new");
  });

  it("leaves out jobs that are not about services", () => {
    const install = {
      ...serviceJob("caddy", "Start"),
      key: "caddy@2.9.0",
      label: "Install Caddy 2.9.0",
    };
    expect(latestServiceJobs([install]).size).toBe(0);
  });
});
