import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

const mockContext = {
  session: {
    apiKey: "",
    accessToken: "",
    refreshToken: "",
  },
  setSession: vi.fn(),
  scope: {
    database: "default",
    namespace: "default",
  },
  setScope: vi.fn(),
  notify: vi.fn(),
  schemaCatalog: [
    {
      name: "Person",
      version: 1,
      properties: {},
      edges: {},
      created_at: 0,
      created_by: "seed",
    },
  ],
  schemaLoading: false,
  schemaError: null,
  refreshSchemaCatalog: vi.fn(),
};

vi.mock("@/App", () => ({
  useConsoleContext: () => mockContext,
}));

import AdminView from "@/features/admin/AdminView";
import ExplorerView from "@/features/explorer/ExplorerView";
import OpsView from "@/features/ops/OpsView";
import QueryView from "@/features/query/QueryView";
import WatchView from "@/features/watch/WatchView";

function makeResponse(data: unknown, contentType = "application/json") {
  return new Response(typeof data === "string" ? data : JSON.stringify(data), {
    status: 200,
    headers: {
      "content-type": contentType,
    },
  });
}

function Wrapper({ children }: { children: React.ReactNode }) {
  return <MemoryRouter>{children}</MemoryRouter>;
}

describe("route smoke", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = String(input);

        if (url.includes("/state/health")) {
          return Promise.resolve(makeResponse({ status: "SERVING" }));
        }
        if (url.includes("/state/sites")) {
          return Promise.resolve(makeResponse({ sites: [] }));
        }
        if (url.includes("/state/replication")) {
          return Promise.resolve(makeResponse({ peers: [] }));
        }
        if (url.includes("/state/jobs")) {
          return Promise.resolve(makeResponse({ jobs: [] }));
        }
        if (url.includes("/state/audit")) {
          return Promise.resolve(makeResponse({ events: [] }));
        }
        if (url.includes("/state/watch/subscriptions")) {
          return Promise.resolve(makeResponse({ subscriptions: [] }));
        }
        if (url.includes("/metrics/raw")) {
          return Promise.resolve(makeResponse("# metrics", "text/plain"));
        }

        return Promise.resolve(makeResponse({}));
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders explorer route", () => {
    render(<ExplorerView />, { wrapper: Wrapper });
    expect(screen.getByText("Node Exploration Flow")).toBeInTheDocument();
  });

  it("renders query route", () => {
    render(<QueryView />, { wrapper: Wrapper });
    expect(screen.getByText("Query Studio")).toBeInTheDocument();
  });

  it("renders ops route", () => {
    render(<OpsView />, { wrapper: Wrapper });
    expect(screen.getByText("Cluster Operations")).toBeInTheDocument();
  });

  it("renders admin route", () => {
    render(<AdminView />, { wrapper: Wrapper });
    expect(screen.getByText("Safe Admin Mutations")).toBeInTheDocument();
  });

  it("renders watch route", () => {
    render(<WatchView />, { wrapper: Wrapper });
    expect(screen.getByText("Stream Setup")).toBeInTheDocument();
  });
});
