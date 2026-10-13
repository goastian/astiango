import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import "./control.css";
import "../../../packages/i18n/fonts.css";

mount(App, { target: document.getElementById("app")! });
