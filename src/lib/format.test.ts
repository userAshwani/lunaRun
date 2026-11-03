import { describe, expect, it } from "vitest";
import { formatBytes, formatIsoDate, formatUnixDate } from "./format";

describe("formatBytes", () => {
  it("uses the right unit and sensible precision", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatBytes(120 * 1024 * 1024)).toBe("120 MB");
    expect(formatBytes(201_760_169)).toBe("192 MB");
    expect(formatBytes(3.5 * 1024 ** 3)).toBe("3.5 GB");
  });

  it("never returns nonsense for bad input", () => {
    expect(formatBytes(-5)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe("0 B");
    expect(formatBytes(1024 ** 6)).toBe("1048576 TB");
  });
});

describe("formatIsoDate", () => {
  it("keeps the calendar day whatever the time zone", () => {
    expect(formatIsoDate("2027-12-31", "en")).toBe("Dec 31, 2027");
    expect(formatIsoDate("2026-01-01", "en")).toBe("Jan 1, 2026");
  });

  it("returns text that is not a date unchanged", () => {
    expect(formatIsoDate("soon", "en")).toBe("soon");
  });
});

describe("formatUnixDate", () => {
  it("formats seconds since 1970", () => {
    expect(formatUnixDate(1_767_225_600 + 12 * 3600, "en")).toBe("Jan 1, 2026");
  });
});
