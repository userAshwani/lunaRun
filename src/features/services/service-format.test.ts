import { describe, expect, it } from "vitest";
import { formatPorts, serviceName } from "./service-format";

describe("formatPorts", () => {
  it("shows one port as it is and nothing for none", () => {
    expect(formatPorts([3307])).toBe("3307");
    expect(formatPorts([])).toBe("");
  });

  it("joins a run of consecutive ports into a range", () => {
    expect(formatPorts([9100, 9101, 9102, 9103])).toBe("9100–9103");
    expect(formatPorts([9100, 9101])).toBe("9100–9101");
  });

  it("keeps ports that are not consecutive apart, in order", () => {
    expect(formatPorts([8105, 8100])).toBe("8100, 8105");
    expect(formatPorts([9100, 9101, 9103])).toBe("9100–9101, 9103");
  });

  it("ignores a port given twice", () => {
    expect(formatPorts([3307, 3307])).toBe("3307");
  });
});

describe("serviceName", () => {
  it("names a database by its line, PHP by its exact version, the others by their product", () => {
    expect(serviceName({ runtime: "mysql", version: "8.4.13", line: "8.4" })).toBe("MySQL 8.4");
    expect(serviceName({ runtime: "mariadb", version: "12.3.3", line: "12.3" })).toBe(
      "MariaDB 12.3",
    );
    expect(serviceName({ runtime: "php", version: "8.3.36", line: null })).toBe("PHP 8.3.36");
    expect(serviceName({ runtime: "caddy", version: "2.11.7", line: null })).toBe("Caddy");
  });

  it("falls back to the runtime id for one it does not know", () => {
    expect(serviceName({ runtime: "redis", version: "7.2.0", line: null })).toBe("redis");
  });
});
