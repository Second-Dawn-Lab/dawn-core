import type { ReactNode } from "react";

const paths: Record<string, ReactNode> = {
  menu: <path d="M4 6h16M4 12h16M4 18h16" />,
  copy: (
    <>
      <rect x="9" y="9" width="12" height="12" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </>
  ),
  close: <path d="M18 6 6 18M6 6l12 12" />,
  swap: <path d="M16 3l4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16" />,
  undo: <path d="M3 7v6h6M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6.7 3L3 13" />,
  redo: <path d="M21 7v6h-6M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6.7 3L21 13" />,
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m21 21-4.3-4.3" />
    </>
  ),
  check: <path d="M20 6 9 17l-5-5" />,
  download: <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3" />,
  settings: <path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" />,
  trash: <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6" />,
  image: (
    <>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <circle cx="9" cy="9" r="2" />
      <path d="m21 15-5-5L5 21" />
    </>
  ),
  file: <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6" />,
  text: <path d="M4 7V4h16v3M9 20h6M12 4v16" />,
  clock: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 6v6l4 2" />
    </>
  ),
  chevron: <path d="m6 9 6 6 6-6" />,
  shield: (
    <>
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      <path d="m9 12 2 2 4-4" />
    </>
  ),
  alert: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 8v5M12 16.5v.5" />
    </>
  ),
  globe: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M2 12h20M12 2a15 15 0 0 1 0 20M12 2a15 15 0 0 0 0 20" />
    </>
  ),
  keyboard: (
    <>
      <rect x="2" y="5" width="20" height="14" rx="2" />
      <path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M7 15h10" />
    </>
  ),
  accessibility: (
    <>
      <circle cx="12" cy="4.5" r="2" />
      <path d="M5 8l7 1.5L19 8M12 9.5V15l-3 6M12 15l3 6" />
    </>
  ),
  folder: <path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  package: (
    <>
      <path d="M21 8 12 3 3 8v8l9 5 9-5z" />
      <path d="m3 8 9 5 9-5M12 13v8" />
    </>
  ),
  wrench: <path d="M14.7 6.3a4 4 0 0 0-5 5L3 18l3 3 6.7-6.7a4 4 0 0 0 5-5l-2.5 2.5-2.5-.5-.5-2.5z" />,
  info: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M12 16v-5M12 7.5v.01" />
    </>
  ),
  pin: <path d="M12 17v5M9 3h6l-1 6 4 4v2H6v-2l4-4z" />,
  external: <path d="M14 3h7v7M21 3l-9 9M10 5H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5" />,
  more: (
    <>
      <circle cx="5" cy="12" r="1.2" />
      <circle cx="12" cy="12" r="1.2" />
      <circle cx="19" cy="12" r="1.2" />
    </>
  ),
  replace: <path d="M4 7h12M12 3l4 4-4 4M20 17H8M12 13l-4 4 4 4" />,
  doc: (
    <>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6" />
      <path d="M8 13h8M8 17h6" />
    </>
  ),
  sheet: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M3 10h18M3 15h18M9 4v16M15 4v16" />
    </>
  ),
  slides: (
    <>
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M12 16v4M8 20h8M10 8.5v3l3-1.5z" />
    </>
  ),
  pdf: (
    <>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6" />
      <path d="M8 17c1.5-1 2.5-3 3-6 .3 2 1.5 4 4 4.5-2 .2-4.500.7-7 1.500z" />
    </>
  ),
  pencil: <path d="M17 3a2.8 2.8 0 0 1 4 4L8 20l-5 1 1-5z" />,
  plus: <path d="M12 5v14M5 12h14" />,
  pause: <path d="M8 5v14M16 5v14" />,
  play: <path d="M7 4.500v15l12-7.500z" />,
  scan: <path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M4 12h16" />,
};

export type IconName = keyof typeof paths;

export function Icon(props: { name: IconName; size?: number; className?: string }) {
  const { name, size = 18, className } = props;
  return (
    <svg
      className={`dw-icon ${className ?? ""}`}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}
