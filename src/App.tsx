import { Moon } from "lucide-react";
import { Button } from "@/components/ui/button";

// Placeholder shell. The sidebar layout, routing and i18n are separate M0.1 tasks.
function App() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-4">
      <Moon className="size-10 text-primary" aria-hidden />
      <h1 className="text-3xl font-semibold">lunaRun</h1>
      <p className="text-muted-foreground">Multi-version PHP &amp; Laravel dev environment.</p>
      <Button disabled>Coming soon</Button>
    </main>
  );
}

export default App;
