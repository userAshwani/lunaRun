import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import { ConfirmDialog } from "./confirm-dialog";

function open(props: Partial<ComponentProps<typeof ConfirmDialog>> = {}) {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <ConfirmDialog
      title="Uninstall PHP 8.3.9?"
      description="This removes the program files."
      confirmLabel="Uninstall"
      cancelLabel="Keep it"
      onConfirm={onConfirm}
      onCancel={onCancel}
      {...props}
    >
      <p>Extra detail</p>
    </ConfirmDialog>,
  );
  return { onConfirm, onCancel };
}

describe("ConfirmDialog", () => {
  it("is a named modal dialog with its description and content", () => {
    open();
    const dialog = screen.getByRole("dialog", { name: "Uninstall PHP 8.3.9?" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveAccessibleDescription("This removes the program files.");
    expect(dialog).toHaveTextContent("Extra detail");
  });

  it("starts on the safe choice", () => {
    open();
    expect(screen.getByRole("button", { name: "Keep it" })).toHaveFocus();
  });

  it("confirms and cancels through its buttons", async () => {
    const { onConfirm, onCancel } = open();
    await userEvent.click(screen.getByRole("button", { name: "Uninstall" }));
    expect(onConfirm).toHaveBeenCalledOnce();
    await userEvent.click(screen.getByRole("button", { name: "Keep it" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("cancels on Escape and on a click outside the panel, not on a click inside", async () => {
    const { onCancel } = open();
    await userEvent.click(screen.getByText("Extra detail"));
    expect(onCancel).not.toHaveBeenCalled();

    await userEvent.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);

    const backdrop = screen.getByRole("dialog").parentElement as HTMLElement;
    await userEvent.click(backdrop);
    expect(onCancel).toHaveBeenCalledTimes(2);
  });

  it("stays put while the action is under way", async () => {
    const { onConfirm, onCancel } = open({ busy: true });
    expect(screen.getByRole("button", { name: "Uninstall" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Keep it" })).toBeDisabled();

    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("dialog").parentElement as HTMLElement);
    expect(onCancel).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("can keep the confirm button off", () => {
    open({ confirmDisabled: true, destructive: true });
    expect(screen.getByRole("button", { name: "Uninstall" })).toBeDisabled();
  });
});
