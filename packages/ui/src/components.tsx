import {
  useEffect,
  useRef,
  type ButtonHTMLAttributes,
  type ReactNode,
} from "react";
import { Icon, type IconName } from "./icons";

const cx = (...parts: (string | false | undefined)[]) => parts.filter(Boolean).join(" ");

// --- Buttons ------------------------------------------------------------

export function Button(
  props: ButtonHTMLAttributes<HTMLButtonElement> & {
    variant?: "default" | "primary" | "ghost" | "danger";
    icon?: IconName;
  },
) {
  const { variant = "default", icon, children, className, ...rest } = props;
  return (
    <button
      type="button"
      className={cx("dw-btn", variant !== "default" && `dw-btn--${variant}`, className)}
      {...rest}
    >
      {icon && <Icon name={icon} size={16} />}
      {children}
    </button>
  );
}

export function IconButton(
  props: ButtonHTMLAttributes<HTMLButtonElement> & { icon: IconName; label: string },
) {
  const { icon, label, className, ...rest } = props;
  return (
    <button
      type="button"
      className={cx("dw-iconbtn", className)}
      aria-label={label}
      title={label}
      {...rest}
    >
      <Icon name={icon} size={17} />
    </button>
  );
}

export function Chip(props: {
  tone?: "neutral" | "brand" | "success" | "warn";
  icon?: IconName;
  title?: string;
  children: ReactNode;
}) {
  const { tone = "neutral", icon, title, children } = props;
  return (
    <span className={cx("dw-chip", tone !== "neutral" && `dw-chip--${tone}`)} title={title}>
      {icon && <Icon name={icon} size={13} />}
      {children}
    </span>
  );
}

/** `value` and `max` in the same unit; omit `max` for an indeterminate bar. */
export function ProgressBar(props: { value: number; max?: number; label: string }) {
  const percent = props.max ? Math.min(100, Math.round((props.value / props.max) * 100)) : undefined;
  return (
    <div
      className={cx("dw-progress", percent === undefined && "dw-progress--busy")}
      role="progressbar"
      aria-label={props.label}
      aria-valuenow={percent}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div className="dw-progress__bar" style={percent === undefined ? undefined : { width: `${percent}%` }} />
    </div>
  );
}

export function Spinner() {
  return <span className="dw-spinner" role="status" aria-label="Loading" />;
}

// --- Form controls ------------------------------------------------------

export function Switch(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      aria-label={props.label}
      disabled={props.disabled}
      className="dw-switch"
      onClick={() => props.onChange(!props.checked)}
    />
  );
}

export function Checkbox(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <label className={cx("dw-check", props.disabled && "dw-check--disabled")}>
      <input
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      <span>{props.children}</span>
    </label>
  );
}

export interface Option<T extends string> {
  value: T;
  label: string;
  description?: string;
  disabled?: boolean;
}

export function RadioGroup<T extends string>(props: {
  name: string;
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className="dw-radios" role="radiogroup">
      {props.options.map((o) => {
        const disabled = props.disabled || o.disabled;
        return (
          <label
            key={o.value}
            className={cx(
              "dw-radio",
              props.value === o.value && "dw-radio--on",
              disabled && "dw-radio--disabled",
            )}
          >
            <input
              type="radio"
              name={props.name}
              checked={props.value === o.value}
              disabled={disabled}
              onChange={() => props.onChange(o.value)}
            />
            <span>
              {o.label}
              {o.description && <small>{o.description}</small>}
            </span>
          </label>
        );
      })}
    </div>
  );
}

export function Select<T extends string>(props: {
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <select
      className="dw-select"
      aria-label={props.label}
      value={props.value}
      disabled={props.disabled}
      onChange={(e) => props.onChange(e.target.value as T)}
    >
      {props.options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

// --- Overlays -----------------------------------------------------------

export function Dialog(props: {
  open: boolean;
  title: string;
  onClose: () => void;
  children?: ReactNode;
  footer?: ReactNode;
}) {
  const { open, title, onClose } = props;
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="dw-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dw-dialog" role="dialog" aria-modal="true" aria-label={title}>
        <h2>{title}</h2>
        {props.children}
        {props.footer && <div className="dw-dialog__footer">{props.footer}</div>}
      </div>
    </div>
  );
}

/** A trigger plus a floating panel that closes on outside click or Escape. */
export function Popover(props: {
  open: boolean;
  onClose: () => void;
  trigger: ReactNode;
  children: ReactNode;
  align?: "start" | "end" | "center";
}) {
  const { open, onClose, align = "start" } = props;
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);
  return (
    <div className="dw-popover-wrap" ref={ref}>
      {props.trigger}
      {open && <div className={cx("dw-popover", `dw-popover--${align}`)}>{props.children}</div>}
    </div>
  );
}

// --- Navigation and layout ---------------------------------------------

export interface NavItem<T extends string> {
  id: T;
  label: string;
  icon: IconName;
}

export function Nav<T extends string>(props: {
  items: NavItem<T>[];
  value: T;
  onChange: (id: T) => void;
  variant?: "bar" | "list";
  label: string;
}) {
  const { variant = "list" } = props;
  return (
    <nav className={cx("dw-nav", `dw-nav--${variant}`)} aria-label={props.label}>
      {props.items.map((item) => (
        <button
          key={item.id}
          type="button"
          className="dw-nav__item"
          aria-current={item.id === props.value ? "page" : undefined}
          onClick={() => props.onChange(item.id)}
        >
          <Icon name={item.icon} size={17} />
          {item.label}
        </button>
      ))}
    </nav>
  );
}

export interface MenuEntry {
  label: string;
  icon?: IconName;
  hint?: string;
  disabled?: boolean;
  /** Draw a divider above this entry. */
  separator?: boolean;
  onSelect: () => void;
}

/** A button that opens a small list of actions. */
export function Menu(props: {
  label: string;
  icon: IconName;
  entries: MenuEntry[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { open, onOpenChange } = props;
  return (
    <Popover
      open={open}
      onClose={() => onOpenChange(false)}
      align="start"
      trigger={
        <IconButton
          icon={props.icon}
          label={props.label}
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => onOpenChange(!open)}
        />
      }
    >
      <div className="dw-menu" role="menu">
        {props.entries.map((entry) => (
          <div key={entry.label}>
            {entry.separator && <hr className="dw-menu__sep" />}
            <button
              type="button"
              role="menuitem"
              className="dw-menu__item"
              disabled={entry.disabled}
              onClick={() => {
                onOpenChange(false);
                entry.onSelect();
              }}
            >
              {entry.icon && <Icon name={entry.icon} size={16} />}
              <span>{entry.label}</span>
              {entry.hint && <small>{entry.hint}</small>}
            </button>
          </div>
        ))}
      </div>
    </Popover>
  );
}

export function Section(props: { title?: string; description?: string; children: ReactNode }) {
  return (
    <section className="dw-section">
      {props.title && <h3 className="dw-section__title">{props.title}</h3>}
      {props.description && <p className="dw-section__desc">{props.description}</p>}
      <div className="dw-section__body">{props.children}</div>
    </section>
  );
}

export function Row(props: {
  title: string;
  description?: string;
  badge?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="dw-row">
      <div className="dw-row__text">
        <div className="dw-row__title">
          {props.title}
          {props.badge}
        </div>
        {props.description && <div className="dw-row__desc">{props.description}</div>}
      </div>
      {props.children}
    </div>
  );
}

export function EmptyState(props: {
  icon: IconName;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="dw-empty">
      <div className="dw-empty__icon">
        <Icon name={props.icon} size={28} />
      </div>
      <h3>{props.title}</h3>
      {props.children && <p>{props.children}</p>}
      {props.action}
    </div>
  );
}
