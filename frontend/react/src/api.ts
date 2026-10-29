import { createClient } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import { HealthService } from "./gen/health/v1/health_pb";

// Every API is served under /api/: the HTTPRoute sends it to the app, which
// mounts its Connect services there. One client per proto service; the
// generated descriptors come from proto/ at build (src/gen/, never committed).
export const transport = createConnectTransport({ baseUrl: "/api" });

export const health = createClient(HealthService, transport);
