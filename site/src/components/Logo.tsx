// The WadSpaces lockup and mark (dark only, as WadSpaces is).
import clsx from "clsx";

export function Logo({ className }: { className?: string }) {
  return (
    <span className={clsx("relative inline-block", className)}>
      <img src="/brand/wadspaces-logo-dark.svg" alt="WAD SPACES" className="block h-full w-auto" draggable={false} />
    </span>
  );
}

export function LogoMark({ className }: { className?: string }) {
  return (
    <span className={clsx("relative inline-block", className)}>
      <img src="/brand/wadspaces-icon-dark-transparent.svg" alt="WAD SPACES" className="block size-full" draggable={false} />
    </span>
  );
}
