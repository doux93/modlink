/**
 * Naive UI GlobalThemeOverrides — 与 tokens.css 保持一致。
 * 这里用字面量而非引用 tokens.css，因为 Naive UI 在运行时需要 JS 对象。
 * 修改任一处颜色时，请同步另一处。
 */
import type { GlobalThemeOverrides } from "naive-ui";

export const lightThemeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#2f6fed",
    primaryColorHover: "#3a7af0",
    primaryColorPressed: "#2961d4",
    primaryColorSuppl: "#2f6fed",

    bodyColor: "#ffffff",
    cardColor: "#ffffff",
    modalColor: "#ffffff",
    popoverColor: "#ffffff",

    textColorBase: "#1f2329",
    textColor1: "#1f2329",
    textColor2: "#4b5563",
    textColor3: "#8b8b92",
    textColorDisabled: "#b8bcc4",

    borderColor: "rgba(128, 128, 128, 0.24)",
    dividerColor: "rgba(128, 128, 128, 0.16)",

    successColor: "#2dbd78",
    successColorHover: "#36cf87",
    successColorPressed: "#28a269",
    warningColor: "#e0a73b",
    warningColorHover: "#eab952",
    warningColorPressed: "#c8912f",
    errorColor: "#d9534f",
    errorColorHover: "#e2625d",
    errorColorPressed: "#bf4542",
    infoColor: "#4c8dff",

    borderRadius: "8px",
    borderRadiusSmall: "4px",
  },
};

export const darkThemeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#2f6fed",
    primaryColorHover: "#3a7af0",
    primaryColorPressed: "#2961d4",
    primaryColorSuppl: "#2f6fed",

    bodyColor: "#18181d",
    cardColor: "#1f1f25",
    modalColor: "#1f1f25",
    popoverColor: "#1f1f25",

    textColorBase: "#e8e8ea",
    textColor1: "#e8e8ea",
    textColor2: "#b8bcc4",
    textColor3: "#8a8a92",
    textColorDisabled: "#5a5a62",

    borderColor: "rgba(255, 255, 255, 0.10)",
    dividerColor: "rgba(255, 255, 255, 0.06)",

    successColor: "#2dbd78",
    successColorHover: "#36cf87",
    successColorPressed: "#28a269",
    warningColor: "#e0a73b",
    warningColorHover: "#eab952",
    warningColorPressed: "#c8912f",
    errorColor: "#d9534f",
    errorColorHover: "#e2625d",
    errorColorPressed: "#bf4542",
    infoColor: "#4c8dff",

    borderRadius: "8px",
    borderRadiusSmall: "4px",
  },
};