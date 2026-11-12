import {
  CircleAlert,
  CircleCheck,
  CircleStop,
  LoaderCircle,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ServiceState } from "@/bindings/ServiceState";
import { Badge } from "@/components/ui/badge";

type Variant = "neutral" | "info" | "success" | "warning" | "destructive";

/** Every state pairs a colour with an icon and a word, so none is told apart by colour alone. */
const LOOK: Readonly<
  Record<ServiceState, { icon: LucideIcon; variant: Variant; spins?: boolean }>
> = {
  running: { icon: CircleCheck, variant: "success" },
  starting: { icon: LoaderCircle, variant: "info", spins: true },
  stopping: { icon: LoaderCircle, variant: "warning", spins: true },
  stopped: { icon: CircleStop, variant: "neutral" },
  crashed: { icon: TriangleAlert, variant: "destructive" },
  error: { icon: CircleAlert, variant: "destructive" },
};

export function ServiceStateBadge({ state }: { state: ServiceState }) {
  const { t } = useTranslation();
  const { icon: Icon, variant, spins } = LOOK[state];
  return (
    <Badge variant={variant}>
      <Icon
        className={spins ? "size-3 animate-spin motion-reduce:animate-none" : "size-3"}
        aria-hidden
      />
      {t(`services.state.${state}`)}
    </Badge>
  );
}
