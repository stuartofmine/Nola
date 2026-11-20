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
    canvas: "#faf8f4",
    surface: "#ffffff",
    surfaceSubtle: "#f1efe8",
    surfaceElevated: "#ffffff",
    ink: "#242923",
    inkSecondary: "#5e665e",
    inkMuted: "#626b62",
    border: "#dedcd3",
    outline: "#777e74",
    accent: "#345d50",
    onAccent: "#ffffff",
    accentSubtle: "#e3ebe3",
    success: "#237047",
    warning: "#865400",
    danger: "#ad3d34",
    info: "#1f5f8b",
};

export const DARK_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#1b1c1a",
    surface: "#222420",
    surfaceSubtle: "#292c27",
    surfaceElevated: "#30332e",
    ink: "#f1f1e9",
    inkSecondary: "#b5baaf",
    inkMuted: "#a1aa9d",
    border: "#383d35",
    outline: "#899084",
    accent: "#a8c9b7",
    onAccent: "#192b22",
    accentSubtle: "#303f36",
    success: "#82c99b",
    warning: "#e9bd66",
    danger: "#efaaa2",
    info: "#8ab8e6",
};
