import "./dev/installMock"; // dev-only fake backend for browser previews (no-op in the app)
import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/theme.css";
import Widget from "./windows/widget/Widget";
import { startAppStore } from "./stores/app";
import { startAiStore } from "./stores/ai";
import { startWordsStore } from "./stores/words";
import { EVENTS, onEvent } from "./lib/api";
import { ensureBody } from "./lib/extract";
import Toasts from "./components/Toasts";

startAppStore();
startAiStore();
startWordsStore();

// The daily pick waits for the article text; the widget is always loaded, so it does the extraction.
void onEvent<{ articleId: number }>(EVENTS.needsBody, ({ articleId }) => {
  void ensureBody(articleId).catch(() => {});
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Widget />
    <Toasts compact />
  </React.StrictMode>,
);
