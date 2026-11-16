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
    textColor3: "#737378",
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
    baseColor: "#18283f",
    primaryColor: "#8bb8ff",
    primaryColorHover: "#a7c9ff",
    primaryColorPressed: "#73a4ef",
    primaryColorSuppl: "#8bb8ff",

    bodyColor: "#252c36",
    cardColor: "#2d3642",
    modalColor: "#354151",
    popoverColor: "#354151",
    inputColor: "#26303d",
    inputColorDisabled: "#2b3440",
    tableColor: "#2d3642",

    textColorBase: "#e7edf6",
    textColor1: "#e7edf6",
    textColor2: "#c2cddb",
    textColor3: "#a2b0c3",
    textColorDisabled: "#7d899a",

    borderColor: "rgba(180, 199, 224, 0.26)",
    dividerColor: "rgba(180, 199, 224, 0.16)",

    successColor: "#75d2ad",
    successColorHover: "#91dfc0",
    successColorPressed: "#5bbb96",
    warningColor: "#f0c879",
    warningColorHover: "#f6d99e",
    warningColorPressed: "#dbb266",
    errorColor: "#f29aa0",
    errorColorHover: "#f7b3b8",
    errorColorPressed: "#dc8088",
    infoColor: "#93beff",

    borderRadius: "8px",
    borderRadiusSmall: "4px",
  },
};
