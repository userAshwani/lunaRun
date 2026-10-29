import type { defaultNamespace } from "./index";
import type { Translation } from "./locales/en";

// Makes `t("...")` keys type-checked against the English file.
declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: typeof defaultNamespace;
    resources: { [defaultNamespace]: Translation };
  }
}
