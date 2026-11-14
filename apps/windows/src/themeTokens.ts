// SPDX-License-Identifier: Apache-2.0
// Canonical Warm Ink & Warm Paper surface tokens synchronized with shared/design-tokens/tokens.json.

export interface DesktopThemeTokens {
    readonly canvas: string;
    readonly surface: string;
    readonly surfaceSubtle: string;
    readonly ink: string;
    readonly inkSecondary: string;
    readonly border: string;
    readonly outline: string;
    readonly accent: string;
    readonly onAccent: string;
}

export const LIGHT_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#f7f6f3",
    surface: "#ffffff",
    surfaceSubtle: "#f1f0ec",
    ink: "#181817",
    inkSecondary: "#5c5b57",
    border: "#e5e3dd",
    outline: "#7a7872",
    accent: "#181817",
    onAccent: "#ffffff",
};

export const DARK_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#131312",
    surface: "#1b1b19",
    surfaceSubtle: "#232320",
    ink: "#f2f0eb",
    inkSecondary: "#a3a09a",
    border: "#2e2d2a",
    outline: "#827f79",
    accent: "#f2f0eb",
    onAccent: "#131312",
};
