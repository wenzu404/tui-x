# tui-x

TUI client for X/Twitter written in Rust with ratatui.

## Architecture

- `src/api/` — API layer: HTTP client, GraphQL operation extraction, models, rate limiting
- `src/auth/` — Authentication: cookie-based credentials, multi-account store
- `src/config/` — TOML configuration
- `src/tui/` — Terminal UI: views, widgets, theme
- `src/app.rs` — Main application state and event loop

## Key design decisions

- Uses X/Twitter's internal GraphQL API (same as web client), not the official API
- GraphQL operation IDs are dynamically extracted from X.com's JS bundles and cached
- Auth is cookie-based (auth_token + ct0 CSRF token)
- TLS fingerprinting via `rquest` to mimic Chrome
- Rate limiting with jitter + exponential backoff

## Build & run

```sh
cargo build --release
cargo run
```

## Auth setup

Set env vars `X_AUTH_TOKEN` and `X_CT0`, or create `~/.config/tui-x/auth.json`:
```json
{
  "default": "myaccount",
  "accounts": {
    "myaccount": {
      "auth_token": "...",
      "ct0": "..."
    }
  }
}
```
