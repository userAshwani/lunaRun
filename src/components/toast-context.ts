import { createContext, useContext } from "react";

export interface ToastInput {
  /** One short sentence, for example "PHP 8.3.27 installed". */
  title: string;
  /** `success` for something that worked, `info` for a neutral note such as a cancel. */
  tone?: "success" | "info";
}

export interface ToastApi {
  show: (toast: ToastInput) => void;
}

export const ToastContext = createContext<ToastApi | null>(null);

export function useToast(): ToastApi {
  const api = useContext(ToastContext);
  if (!api) throw new Error("useToast must be used inside <ToastProvider>");
  return api;
}
