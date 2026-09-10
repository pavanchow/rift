<img src="docs/logo.svg" alt="Rift logo" width="96">

# Rift: a programmable reverse proxy in Rust

Rift is a programmable single-binary reverse proxy written in Rust where routing
rules are a compiled DSL, not YAML. It is one static binary that matches traffic
on host, path prefix or regex, and headers, then answers inline or forwards over
pooled, streaming upstream connections, so rules are code, not config. Use it as
a lightweight nginx alternative built on hyper when you want compiled routing
rules instead of a config file. By Pavan Nallamothu.

**[Live demo](https://pavanchow.github.io/rift/)** · MIT licensed · written in Rust

Built from scratch by [Pavan Nallamothu](https://pavanchow.github.io/) ([LinkedIn](https://www.linkedin.com/in/pavanchow/), [GitHub](https://github.com/pavanchow)).

## The rules are the config

A Rift config is a list of routes. Each route matches on host, path, and headers, and either answers
inline or forwards to an upstream. First match wins.

```
# a static answer, no backend needed
route path "/health" -> respond 200 "ok"

# regex path match, forward to a backend
route host "api.example.com" path ~ "^/v[0-9]+/" -> upstream "http://127.0.0.1:9001"

# header-gated route
route path "/" header "X-Env" "prod" -> upstream "http://10.0.0.5:80"

# catch-all
route path "/" -> upstream "http://127.0.0.1:8000"
```

Matchers (all AND together): `host` (exact, case-insensitive), `path` (prefix, or `~` for a regex),
`header` (exact). Actions: `upstream "<url>"` or `respond <code> "<body>"`.

## Run it

```
rift check --config routes.rift          # parse and validate, print the rule count
rift serve --config routes.rift --addr 127.0.0.1:8080
```

The server speaks HTTP/1.1 and HTTP/2, forwards matched requests to the upstream over a pooled
client connection, and streams the response body straight back to the caller.

## Why Rift

- **One binary.** No daemon to install, no plugin ecosystem, no reload dance.
- **Rules are compiled.** The DSL is parsed once into matchers at startup, not re-interpreted from
  YAML on every request. A bad rule fails at `check` time with the offending line, not in production.
- **Pooled and streaming.** Upstream connections are reused, and response bodies stream rather than
  buffer, so large responses do not sit in memory.

## Stack

Rust, on `hyper` and `hyper-util` for the HTTP/1.1 and HTTP/2 server and the pooled client, `regex`
for path matching, and `clap` for the CLI. See [DESIGN.md](DESIGN.md).

## Status

v0.1: HTTP/1.1 and HTTP/2 server, the routing DSL (host, path prefix and regex, header matchers,
upstream and respond actions), pooled upstream forwarding with streaming, and a `check` validator.
Next: rate limiting, header mutation, and edge caching as native rule actions.
