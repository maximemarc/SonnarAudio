import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import LinuxApp from "./linux/LinuxApp";
// Inter : la police du design system Nocturne (maquette Claude Design).
// EMBARQUÉE via @fontsource et non chargée depuis un CDN — l'app est une
// app desktop qui doit rendre à l'identique hors connexion, et la webview
// tourne sans CSP (voir tauri.conf.json). Les quatre graisses sont celles
// que la maquette déclare : corps 400, titres 500, valeurs 600/700.
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
import "./styles.css";

// Deux backends : Windows (moteur cpal/WASAPI, `App`) et Linux (PipeWire,
// `LinuxApp`). La webview le dit elle-même : WebView2 sous Windows,
// WebKitGTK ("X11; Linux" / "Wayland") sous Linux.
const isLinux = /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isLinux ? <LinuxApp /> : <App />}</React.StrictMode>,
);
