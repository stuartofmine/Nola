// SPDX-License-Identifier: Apache-2.0
// Canonical Warm Ink & Warm Paper surface tokens synchronized with shared/design-tokens/tokens.json.

export interface DesktopThemeTokens {
    readonly canvas: string;
    readonly surface: string;
    readonly surfaceSubtle: string;
    readonly surfaceElevated: string;
    readonly ink: string;
    readonly inkSecondary: string;
    readonly inkMuted: string;
    readonly border: string;
    readonly outline: string;
    readonly accent: string;
    readonly onAccent: string;
    readonly accentSubtle: string;
    readonly success: string;
    readonly warning: string;
    readonly danger: string;
    readonly info: string;
}

export const LIGHT_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#f7f6f3",
    surface: "#ffffff",
    surfaceSubtle: "#f1f0ec",
    surfaceElevated: "#ffffff",
    ink: "#181817",
    inkSecondary: "#5c5b57",
    inkMuted: "#66655f",
    border: "#e5e3dd",
    outline: "#7a7872",
    accent: "#181817",
    onAccent: "#ffffff",
    accentSubtle: "#eae8e2",
    success: "#237047",
    warning: "#865400",
    danger: "#ad3d34",
    info: "#1f5f8b",
};

export const DARK_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#131312",
    surface: "#1b1b19",
    surfaceSubtle: "#232320",
    surfaceElevated: "#232320",
    ink: "#f2f0eb",
    inkSecondary: "#a3a09a",
    inkMuted: "#96938d",
    border: "#2e2d2a",
    outline: "#827f79",
    accent: "#f2f0eb",
    onAccent: "#131312",
    accentSubtle: "#292825",
    success: "#82c99b",
    warning: "#e9bd66",
    danger: "#efaaa2",
    info: "#8ab8e6",
};
