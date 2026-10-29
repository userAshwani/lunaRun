import { describe, expect, it } from "vitest";
import i18n, { defaultNamespace } from "./index";
import { en } from "./locales/en";

/** Collect every leaf key as a dotted path. */
function leafKeys(obj: object, prefix = ""): string[] {
  return Object.entries(obj).flatMap(([key, value]: [string, unknown]) =>
    typeof value === "object" && value !== null
      ? leafKeys(value, `${prefix}${key}.`)
      : [`${prefix}${key}`],
  );
}

describe("i18n", () => {
  it("has a non-empty string for every English key", () => {
    for (const key of leafKeys(en)) {
      const value: unknown = i18n.getResource("en", defaultNamespace, key);
      expect(typeof value, key).toBe("string");
      expect((value as string).length, key).toBeGreaterThan(0);
    }
  });

  it("falls back to English for an unknown locale", async () => {
    await i18n.changeLanguage("xx");
    expect(i18n.t("app.name")).toBe("lunaRun");
    await i18n.changeLanguage("en");
  });
});
