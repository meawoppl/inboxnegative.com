#!/bin/bash
set -e

# Install Rust and other necessary build tools
apt-get update && apt-get install -y \
    curl \
    build-essential \
    git \
    cmake \
    libssl-dev \
    pkg-config && apt-get clean

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
export PATH="/root/.cargo/bin:${PATH}"
rustup update stable

# Setup for WASM
cargo install --locked trunk
cargo install --locked wasm-bindgen-cli
rustup target add wasm32-unknown-unknown