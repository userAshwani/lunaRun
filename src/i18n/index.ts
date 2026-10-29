import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { en } from "./locales/en";

export const defaultNamespace = "translation";

// Resources are bundled, so init is synchronous and nothing suspends on first render.
void i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  defaultNS: defaultNamespace,
  resources: { en: { [defaultNamespace]: en } },
  interpolation: { escapeValue: false }, // React already escapes
  returnNull: false,
});

export default i18n;
