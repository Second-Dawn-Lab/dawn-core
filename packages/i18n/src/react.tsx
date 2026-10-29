import { createContext, useContext, useMemo, type ReactNode } from "react";
import { createTranslator, type Dictionary, type Translator } from "./translator";

interface I18nValue {
  locale: string;
  t: Translator;
}

const I18nContext = createContext<I18nValue>({ locale: "en", t: (key) => key });

export function I18nProvider(props: {
  locale: string;
  dictionaries: Record<string, Dictionary>;
  children: ReactNode;
}) {
  const { locale, dictionaries, children } = props;
  const value = useMemo(
    () => ({ locale, t: createTranslator(dictionaries, locale) }),
    [locale, dictionaries],
  );
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  return useContext(I18nContext);
}
