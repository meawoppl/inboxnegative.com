# TODO

## Build & Deployment

### Docker Build Optimization
- **Cache WASM Rust packages during Docker builds**
  - Currently, WASM packages are not being cached and are rebuilt on every Docker build
  - This significantly increases build times (see frontend compilation in Dockerfile)
  - Need to implement proper cargo cache mounting or multi-stage caching for wasm32-unknown-unknown target
  - Related files:
    - `Dockerfile` - frontend build stage
    - `frontend/Cargo.toml` - WASM dependencies

## Infrastructure

## Features

### Custom Service Unavailable Page
- **Replace Bad Gateway errors with user-friendly service page**
  - Currently users see generic 502/503 Bad Gateway errors during deployments
  - Create custom HTML page explaining service is temporarily unavailable
  - Show estimated time for service restoration
  - Add link to status page or fallback contact information
  - Nginx/reverse proxy should serve this page when backend is down

## Bugs

## Documentation

### How It Works Page
- **Create "How It Works" documentation page**
  - Explain the email flow: SMTP receive → storage → SSE delivery → browser
  - Document the ZMQ pub/sub architecture for email distribution
  - Describe the transient storage model (RAM only, no persistence)
  - Include architecture diagrams showing component interactions
  - Explain the 5-minute attachment expiration model
  - Document OAuth authentication flow
  - Add this to the website or as a dedicated doc file
