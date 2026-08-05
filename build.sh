#!/bin/bash
set -e

# Set the working directory
cd "$(dirname "$0")"

# Build the frontend
echo "Building frontend..."
cd frontend
trunk build --release
cd ..

# Build the backend
echo "Building backend..."
cd backend
cargo build --release
cd ..

echo "Build completed successfully!"