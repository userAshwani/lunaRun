import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "react-router";
import { router } from "@/app/router";
import { ThemeProvider } from "@/app/theme-provider";
import { ToastProvider } from "@/components/toast";
import { createQueryClient } from "@/lib/query-client";

const queryClient = createQueryClient();

function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <ToastProvider>
          <RouterProvider router={router} />
        </ToastProvider>
      </ThemeProvider>
    </QueryClientProvider>
  );
}

export default App;
