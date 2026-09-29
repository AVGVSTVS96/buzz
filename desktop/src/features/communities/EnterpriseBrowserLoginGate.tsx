import { ShieldCheck } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";

type EnterpriseBrowserLoginGateProps = {
  communityName: string;
  error?: string | null;
  isPending?: boolean;
  onCancel: () => void;
  onContinue: () => void;
};

export function EnterpriseBrowserLoginGate({
  communityName,
  error,
  isPending = false,
  onCancel,
  onContinue,
}: EnterpriseBrowserLoginGateProps) {
  const systemColorScheme = useSystemColorScheme();
  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
      data-testid="enterprise-browser-login-gate"
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[520px] flex-col items-center text-center">
        <div className="flex size-12 items-center justify-center rounded-full bg-primary/10 text-primary">
          <ShieldCheck aria-hidden="true" className="size-6" />
        </div>
        <h1 className="mt-5 text-3xl font-semibold tracking-tight">
          Sign in with your company account
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          {isPending
            ? "Finish sign-in in your browser, then return to Buzz. If the browser flow was closed or is stuck, cancel here to recover."
            : `${communityName} requires enterprise sign-in. Buzz will open your system browser so your identity provider can handle SSO, passkeys, and device-trust checks outside the app.`}
        </p>
        {error ? (
          <p
            className="mt-4 text-sm text-destructive"
            data-testid="enterprise-browser-login-error"
          >
            {error}
          </p>
        ) : null}
        <div className="mt-8 flex w-full max-w-[320px] flex-col gap-3">
          <Button
            className="h-10 w-full"
            data-testid="enterprise-browser-login-continue"
            disabled={isPending}
            onClick={onContinue}
            type="button"
          >
            {isPending ? "Waiting for browser sign-in…" : "Continue in browser"}
          </Button>
          <Button
            className="h-10 w-full"
            data-testid="enterprise-browser-login-cancel"
            onClick={onCancel}
            type="button"
            variant="secondary"
          >
            {isPending ? "Cancel sign-in" : "Cancel"}
          </Button>
        </div>
      </div>
    </div>
  );
}
