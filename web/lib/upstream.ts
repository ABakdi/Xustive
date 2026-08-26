import 'server-only'

import { Agent, type Dispatcher } from 'undici'

/**
 * One keep-alive agent for every upstream the knowledge routes talk to (Wikidata, Wikipedia,
 * Open Library).
 *
 * Without it every call opened a fresh TLS connection, and Wikimedia — which asks clients for a
 * handful of connections at most — began refusing the surplus: `UND_ERR_CONNECT_TIMEOUT` in the
 * server log while curl on the same host connected in under a second. A shared pool with a
 * small cap reuses connections instead of churning them, and a short connect timeout turns a
 * refused connection into a fast failure rather than a ten-second stall on the reader's page.
 *
 * On `globalThis` for the reason `thumb.ts` gives: Next compiles each route into its own bundle
 * with its own module instances, and the point of a pool is that there is one of it.
 */
declare global {
  // eslint-disable-next-line no-var
  var __xustiveUpstreamAgent: Dispatcher | undefined
}

export function upstreamAgent(): Dispatcher {
  return (globalThis.__xustiveUpstreamAgent ??= new Agent({
    connections: 4,
    pipelining: 1,
    keepAliveTimeout: 30_000,
    // IPv4 only: the resolver hands back an IPv6 address for Wikimedia that this host cannot
    // route (`curl -6` fails instantly), and an attempt at it is a connect timeout, not a fast
    // failure. Revisit when the host has a working IPv6 route.
    connect: { timeout: 5_000, family: 4 },
  }))
}

/** `fetch` init that routes through the shared agent. Spread it into any upstream call. */
export function viaUpstream(): { dispatcher: Dispatcher } {
  return { dispatcher: upstreamAgent() }
}
