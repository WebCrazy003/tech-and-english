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

// After a news fetch: check new articles one by one; the backend hides the ones that can't be read.
const checkQueue = new Set<number>(); // queued and in-flight ids
let checking = false;
void onEvent<{ articleIds: number[] }>(EVENTS.checkBodies, ({ articleIds }) => {
  articleIds.forEach((id) => checkQueue.add(id));
  if (checking) return;
  checking = true;
  void (async () => {
    for (const id of checkQueue) {
      await ensureBody(id).catch(() => {});
      checkQueue.delete(id);
    }
    checking = false;
  })();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Widget />
    <Toasts compact />
  </React.StrictMode>,
);
