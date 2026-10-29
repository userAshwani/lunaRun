import { Moon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";

// Placeholder shell. The sidebar layout and routing are separate M0.1 tasks.
function App() {
  const { t } = useTranslation();
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-4">
      <Moon className="size-10 text-primary" aria-hidden />
      <h1 className="text-3xl font-semibold">{t("app.name")}</h1>
      <p className="text-muted-foreground">{t("app.tagline")}</p>
      <Button disabled>{t("app.comingSoon")}</Button>
    </main>
  );
}

export default App;
