import { mount } from 'svelte';
import App from './App.svelte';
import './theme.css';
import { initializeLocale, observeLocaleChanges } from './lib/i18n';

import { installSelectLabelPickers } from './lib/selectLabelPicker';
const stopSelectLabelPickers = installSelectLabelPickers(document);
import.meta.hot?.dispose(stopSelectLabelPickers);

initializeLocale();
const stopObservingLocale = observeLocaleChanges();
import.meta.hot?.dispose(stopObservingLocale);
mount(App, { target: document.getElementById('app')! });
