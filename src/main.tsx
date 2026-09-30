import "./dev/installMock"; // dev-only fake backend for browser previews (no-op in the app)
import React from "react";
import ReactDOM from "react-dom/client";
import { createHashRouter, Navigate, RouterProvider } from "react-router";
import "./styles/theme.css";
import Layout from "./app/Layout";
import Today from "./pages/Today";
import Explore from "./pages/Explore";
import Onboarding from "./pages/Onboarding";
import SettingsLayout from "./pages/settings/SettingsLayout";
import General from "./pages/settings/General";
import Topics from "./pages/settings/Topics";
import Feeds from "./pages/settings/Feeds";
import Notifications from "./pages/settings/Notifications";
import Ai from "./pages/settings/Ai";
import Reader from "./features/reader/Reader";
import { LearningSettings, VoiceSettings } from "./pages/settings/Voice";
import WordBook from "./pages/words/WordBook";
import WordDetail from "./pages/words/WordDetail";
import { PracticeResult, PracticeSession, PracticeStart } from "./pages/words/Practice";
import { startWordsStore } from "./stores/words";
import TalkSetup from "./pages/talk/TalkSetup";
import TalkSession from "./pages/talk/TalkSession";
import TalkReview from "./pages/talk/TalkReview";
import TalkHistory, { TalkTranscript } from "./pages/talk/TalkHistory";
import { startAiStore } from "./stores/ai";
import { api, EVENTS, onEvent } from "./lib/api";
import { startAppStore } from "./stores/app";

const router = createHashRouter([
  { path: "/onboarding", element: <Onboarding /> },
  {
    path: "/",
    element: <Layout />,
    children: [
      { index: true, element: <Navigate to="/today" replace /> },
      { path: "today", element: <Today /> },
      { path: "explore", element: <Explore /> },
      { path: "reader/:id", element: <Reader /> },
      { path: "wordbook", element: <WordBook /> },
      { path: "wordbook/:id", element: <WordDetail /> },
      { path: "practice", element: <PracticeStart /> },
      { path: "practice/session", element: <PracticeSession /> },
      { path: "practice/result", element: <PracticeResult /> },
      { path: "talk", element: <TalkSetup /> },
      { path: "talk/session", element: <TalkSession /> },
      { path: "talk/review/:id", element: <TalkReview /> },
      { path: "talk/history", element: <TalkHistory /> },
      { path: "talk/history/:id", element: <TalkTranscript /> },
      {
        path: "settings",
        element: <SettingsLayout />,
        children: [
          { index: true, element: <Navigate to="/settings/general" replace /> },
          { path: "general", element: <General /> },
          { path: "topics", element: <Topics /> },
          { path: "feeds", element: <Feeds /> },
          { path: "notifications", element: <Notifications /> },
          { path: "ai", element: <Ai /> },
          { path: "voice", element: <VoiceSettings /> },
          { path: "learning", element: <LearningSettings /> },
        ],
      },
    ],
  },
]);

startAppStore();
startAiStore();
startWordsStore();

// The backend asks the main window to show a route (tray menu, widget buttons).
void onEvent<{ route: string }>(EVENTS.navigate, ({ route }) => {
  void api.takePendingRoute();
  void router.navigate(route);
});
void api.takePendingRoute().then((route) => {
  if (route) void router.navigate(route);
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>,
);
