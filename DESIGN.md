# Rift design

## The wedge

A reverse proxy is a router plus a forwarder. Most proxies bury that behind large YAML or a plugin
system. Rift keeps it to one binary and one small language: routes compile to matchers at startup, so
the hot path is a cheap match-and-forward with no config interpretation per request.

## Pipeline

1. **Parse.** The DSL is lexed (quoted strings, `~`, `->`) and parsed into `Rule` values, each a set
   of matchers plus an action. Parse errors name the offending line, so `rift check` catches a bad
   config before it ever serves traffic.
2. **Serve.** A `hyper` server accepts HTTP/1.1 and HTTP/2 connections. For each request Rift extracts
   the host (from the Host header, or the URI authority on HTTP/2), the path, and the headers.
3. **Route.** The compiled table is scanned in order and the first matching rule wins. Matchers AND
   together: host is an exact case-insensitive compare, path is a prefix or a compiled regex, headers
   are exact key and value matches.
4. **Act.** A `respond` rule answers inline. An `upstream` rule rewrites the request URI to the
   backend authority, points the Host header at the upstream, and forwards it.

## Forwarding

Upstream requests go through a pooled `hyper-util` client, so connections to a backend are reused
across requests rather than dialed fresh each time. The request body streams from the client to the
upstream, and the upstream response body streams straight back to the caller, so a large payload is
never fully buffered in the proxy. A bad upstream URL or an unreachable backend returns a clear 502
rather than dropping the connection.

## Deliberate non-goals for v0.1

No rate limiting, header mutation, edge caching, TLS termination, or load balancing across multiple
upstreams yet. v0.1 is the correct core: parse, route, forward, stream. Those features are native
rule actions to add next, and each should stay expressible as one line of the DSL rather than a new
config file.
