// Entry point of the Kadr front end: fonts and design tokens, the user's
// preferences, then the app.

import "@fontsource-variable/inter";
import "./lib/styles/tokens.css";
import "./lib/styles/base.css";

import { mount } from "svelte";

import App from "./App.svelte";
import { preferences } from "./lib/state/preferences.svelte";

preferences.load();

const target = document.getElementById("app");
if (!target) {
  throw new Error("the page has no #app element");
}

export default mount(App, { target });
