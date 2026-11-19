import { stringsFor } from "../src/i18n.ts";
import { startupPageHtml } from "../src/startupPage.ts";

const locales = ["en","zh-Hans","zh-Hant","de","es","fr","ja","ko","ru"];
const resources = Object.fromEntries(locales.map(locale => {
    const strings = stringsFor(locale);
    return [locale, { ...strings, startingPage: startupPageHtml(strings, "starting"),
        failedPage: startupPageHtml(strings, "failed", "DETAIL") }];
}));
process.stdout.write(JSON.stringify(resources));
