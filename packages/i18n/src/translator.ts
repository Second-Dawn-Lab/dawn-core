export type Dictionary = Record<string, string>;
export type Vars = Record<string, string | number>;
export type Translator = (key: string, vars?: Vars) => string;

/** Turn a preference ("system", "de", "de-AT") into one of the supported locales. */
export function resolveLocale(pref: string, supported: string[], fallback = "en"): string {
  const wanted = pref === "system" ? (typeof navigator === "undefined" ? fallback : navigator.language) : pref;
  const base = wanted.toLowerCase().split("-")[0];
  return supported.find((s) => s === base) ?? fallback;
}

/** Build a `t(key, vars)` function. Missing keys fall back to the fallback locale, then the key. */
export function createTranslator(
  dictionaries: Record<string, Dictionary>,
  locale: string,
  fallback = "en",
): Translator {
  return (key, vars) => {
    const template = dictionaries[locale]?.[key] ?? dictionaries[fallback]?.[key] ?? key;
    return vars
      ? template.replace(/\{(\w+)\}/g, (match, name: string) => String(vars[name] ?? match))
      : template;
  };
}
