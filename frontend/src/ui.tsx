import { useId, type ComponentProps, type ReactNode } from "react";
import { Icon } from "@mdi/react";
import { mdiInformationOutline } from "@mdi/js";

export function Button({
  primary,
  className = "",
  ...props
}: ComponentProps<"button"> & { primary?: boolean }) {
  return (
    <button
      type="button"
      className={`button ${primary ? "button-primary" : ""} ${className}`}
      {...props}
    />
  );
}
export function IconButton({
  path,
  label,
  className = "",
  ...props
}: Omit<ComponentProps<"button">, "children"> & {
  path: string;
  label: string;
}) {
  return (
    <button
      type="button"
      className={`icon-button ${className}`}
      aria-label={label}
      title={label}
      {...props}
    >
      <Icon path={path} aria-hidden="true" />
    </button>
  );
}
export function Help({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <>
      <IconButton
        path={mdiInformationOutline}
        label={`Help: ${label}`}
        popoverTarget={id}
      />
      <span
        id={id}
        popover="auto"
        role="note"
        aria-label={label}
        className="help-popover"
      >
        {children}
      </span>
    </>
  );
}
export function Empty({ children }: { children: ReactNode }) {
  return <div className="empty">{children}</div>;
}
