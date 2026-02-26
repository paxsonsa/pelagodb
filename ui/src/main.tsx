import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import App from "./App";
import ExplorerPage from "./pages/ExplorerPage";
import QueryPage from "./pages/QueryPage";
import OpsPage from "./pages/OpsPage";
import AdminPage from "./pages/AdminPage";
import WatchPage from "./pages/WatchPage";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter basename="/ui">
      <Routes>
        <Route path="/" element={<App />}>
          <Route index element={<Navigate to="/explorer" replace />} />
          <Route path="explorer" element={<ExplorerPage />} />
          <Route path="query" element={<QueryPage />} />
          <Route path="ops" element={<OpsPage />} />
          <Route path="admin" element={<AdminPage />} />
          <Route path="watch" element={<WatchPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
