import { lazy, StrictMode, Suspense, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import App from "@/App";
import { LoadingBlock } from "@/components/shared/LoadingBlock";
import "@/styles/globals.css";

const ExplorerPage = lazy(() => import("@/pages/ExplorerPage"));
const QueryPage = lazy(() => import("@/pages/QueryPage"));
const OpsPage = lazy(() => import("@/pages/OpsPage"));
const AdminPage = lazy(() => import("@/pages/AdminPage"));
const WatchPage = lazy(() => import("@/pages/WatchPage"));

function RouteFallback() {
  return (
    <div className="section-card">
      <LoadingBlock rows={6} />
    </div>
  );
}

function lazyRoute(element: ReactNode) {
  return <Suspense fallback={<RouteFallback />}>{element}</Suspense>;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter basename="/ui">
      <Routes>
        <Route path="/" element={<App />}>
          <Route index element={<Navigate to="/explorer" replace />} />
          <Route path="explorer" element={lazyRoute(<ExplorerPage />)} />
          <Route path="query" element={lazyRoute(<QueryPage />)} />
          <Route path="ops" element={lazyRoute(<OpsPage />)} />
          <Route path="admin" element={lazyRoute(<AdminPage />)} />
          <Route path="watch" element={lazyRoute(<WatchPage />)} />
        </Route>
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
