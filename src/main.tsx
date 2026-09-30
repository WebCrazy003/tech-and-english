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
        ],
      },
    ],
  },
]);

startAppStore();
startAiStore();

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
