import { RouterProvider } from "react-router";
import { router } from "@/app/router";
import { ThemeProvider } from "@/app/theme-provider";

function App() {
  return (
    <ThemeProvider>
      <RouterProvider router={router} />
    </ThemeProvider>
  );
}

export default App;
