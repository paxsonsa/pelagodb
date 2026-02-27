export type ScopeContext = {
  database: string;
  namespace: string;
};

export type PrincipalSummary = {
  id: string;
  type: string;
  roles: string[];
};

export type AuthSession = {
  apiKey: string;
  accessToken: string;
  refreshToken: string;
  expiresAt?: number;
  principal?: PrincipalSummary;
};

export type ApiErrorShape = {
  error?: {
    code?: string;
    message?: string;
  };
};

export type PropertySchema = {
  type: string;
  required: boolean;
  index: string;
  default?: unknown;
};

export type EdgeSchema = {
  direction: string;
  sort_key: string;
  ownership: string;
};

export type SchemaDefinition = {
  name: string;
  version: number;
  properties: Record<string, PropertySchema>;
  edges: Record<string, EdgeSchema>;
  created_at: number;
  created_by: string;
};

export type SchemaListResponse = {
  schemas: SchemaDefinition[];
};

export type NodeRef = {
  entity_type: string;
  node_id: string;
  database: string;
  namespace: string;
};

export type NodeModel = {
  id: string;
  entity_type: string;
  properties: Record<string, unknown>;
  locality: string;
  created_at?: number;
  updated_at?: number;
};

export type EdgeModel = {
  edge_id: string;
  source?: NodeRef;
  target?: NodeRef;
  label: string;
  properties: Record<string, unknown>;
  created_at?: number;
};

export type FindNodesResponse = {
  items: NodeModel[];
  next_cursor: string;
  degraded: boolean;
  degraded_reason: string;
  consistency_applied: number;
  snapshot_read_version: number;
};

export type PqlRow = {
  block_name: string;
  node?: NodeModel;
  edge?: EdgeModel;
  explain?: string;
};

export type PqlResponse = {
  items: PqlRow[];
  next_cursor: string;
  degraded: boolean;
  degraded_reason: string;
  consistency_applied: number;
  snapshot_read_version: number;
};

export type ExplainResponse = {
  plan?: {
    strategy: string;
    indexes: Array<{
      field: string;
      operator: string;
      value: string;
      estimated_rows: number;
    }>;
    residual_filter: string;
  };
  explanation: string;
  estimated_cost: number;
  estimated_rows: number;
};

export type GraphNodeResponse = {
  node?: NodeModel;
};

export type GraphEdgesResponse = {
  items: EdgeModel[];
  next_cursor: string;
};

export type HealthResponse = {
  status: string;
};

export type SiteStatus = {
  site_id: string;
  site_name: string;
  claimed_at: number;
  status: string;
};

export type SitesResponse = {
  sites: SiteStatus[];
};

export type ReplicationPeer = {
  remote_site_id: string;
  last_applied_versionstamp: string;
  lag_events: number;
  updated_at: number;
};

export type ReplicationResponse = {
  peers: ReplicationPeer[];
};

export type JobStatus = {
  job_id: string;
  job_type: string;
  status: string;
  created_at: number;
  updated_at: number;
  progress: number;
  error: string;
};

export type JobsResponse = {
  jobs: JobStatus[];
};

export type AuditEvent = {
  event_id: string;
  timestamp: number;
  principal_id: string;
  action: string;
  resource: string;
  allowed: boolean;
  reason: string;
  metadata: Record<string, string>;
};

export type AuditResponse = {
  events: AuditEvent[];
};

export type WatchSubscription = {
  subscription_id: string;
  subscription_type: string;
  database: string;
  namespace: string;
  created_at: number;
  expires_at: number;
};

export type WatchSubscriptionsResponse = {
  subscriptions: WatchSubscription[];
};

export type OpsSnapshotResponse = {
  health: HealthResponse | null;
  sites: SitesResponse | null;
  replication: ReplicationResponse | null;
  jobs: JobsResponse | null;
  audit: AuditResponse | null;
  subscriptions: WatchSubscriptionsResponse | null;
  metrics: string;
};

export type AdminMutationResult = {
  title: string;
  payload: unknown;
};

export type AuthenticateResponse = {
  access_token: string;
  refresh_token: string;
  expires_at: number;
  principal?: {
    principal_id: string;
    principal_type: string;
    roles: string[];
  };
};

export type ValidateTokenResponse = {
  valid: boolean;
  expires_at: number;
  principal?: {
    principal_id: string;
    principal_type: string;
    roles: string[];
  };
};

export type RefreshTokenResponse = {
  access_token: string;
  refresh_token: string;
  expires_at: number;
};

export type StreamEventPayload = {
  subscription_id?: string;
  type?: string;
  versionstamp?: string;
  node?: NodeModel;
  edge?: EdgeModel;
  reason?: string;
  code?: string;
  message?: string;
  [key: string]: unknown;
};

export type WatchEventRow = {
  timestamp: string;
  event: string;
  payload: StreamEventPayload | string;
};
