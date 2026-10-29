export type Theme = "light" | "dark" | "system";

/** Apply a theme preference to the document ("system" follows the OS). */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
}
