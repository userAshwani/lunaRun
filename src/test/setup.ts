import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import "@/i18n";

// Auto-cleanup only happens when Vitest globals are enabled; we keep them off.
afterEach(cleanup);
