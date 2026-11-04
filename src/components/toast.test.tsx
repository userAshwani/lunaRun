import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastProvider } from "./toast";
import { useToast } from "./toast-context";

function Trigger({ title, tone }: { title: string; tone?: "success" | "info" }) {
  const toast = useToast();
  return (
    <button
      onClick={() => {
        toast.show({ title, tone });
      }}
    >
      show {title}
    </button>
  );
}

describe("toasts", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows a confirmation in a live region and removes it after a few seconds", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(
      <ToastProvider>
        <Trigger title="PHP 8.3.27 installed." />
      </ToastProvider>,
    );

    await user.click(screen.getByRole("button", { name: /show/ }));
    expect(screen.getByRole("region", { name: "Notifications" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("PHP 8.3.27 installed.");

    act(() => {
      vi.advanceTimersByTime(6100);
    });
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("can be dismissed by hand", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(
      <ToastProvider>
        <Trigger title="Done" tone="info" />
      </ToastProvider>,
    );

    await user.click(screen.getByRole("button", { name: /show/ }));
    await user.click(screen.getByRole("button", { name: "Dismiss notification" }));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("keeps only the latest few so they never pile up", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(
      <ToastProvider>
        {["one", "two", "three", "four", "five"].map((title) => (
          <Trigger key={title} title={title} />
        ))}
      </ToastProvider>,
    );

    for (const button of screen.getAllByRole("button", { name: /show/ })) {
      await user.click(button);
    }

    expect(screen.getAllByRole("status").map((item) => item.textContent)).toEqual([
      "two",
      "three",
      "four",
      "five",
    ]);
  });

  it("refuses to be used outside its provider", () => {
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => render(<Trigger title="x" />)).toThrow(/ToastProvider/);
    quiet.mockRestore();
  });
});
