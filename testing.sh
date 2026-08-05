#!/bin/bash
set -e

# Set the working directory
cd "$(dirname "$0")"

# Set environment variables for testing
export RUST_BACKTRACE=1
export HTTP_ROOT="$(pwd)/frontend/dist"
export INBOXNULL_HOSTNAME=http://localhost:8080
# OAuth credentials come from the environment -- never hardcode them here, this
# file is tracked. Export them first, or source a gitignored env file:
#   export GOOGLE_CLIENT_ID=... GOOGLE_CLIENT_SECRET=...
: "${GOOGLE_CLIENT_ID:?set GOOGLE_CLIENT_ID before running (see backend/.env.example)}"
: "${GOOGLE_CLIENT_SECRET:?set GOOGLE_CLIENT_SECRET before running (see backend/.env.example)}"
export GOOGLE_CLIENT_ID GOOGLE_CLIENT_SECRET
export TESTING_MODE=true

# Check if frontend files exist and build if needed
  
# Build the frontend
cd frontend
trunk build
cd ..
  
# Build the backend if needed
echo "Building backend..."
cd backend
cargo build --release
cd ..

# Run the application
echo "Starting inboxnull in testing mode..."
echo "SMTP server running on port 2525"
echo "HTTP server running on port 8080"
echo ""
echo "============================================"
echo "SECRET TESTING LOGIN LINKS:"
echo "- Direct test login: http://localhost:8080/api/test-login"
echo "- App with testing mode: http://localhost:8080?testing=true"
echo "  (Use the 'Use Test Account' button on the login screen)"
echo "============================================"
echo ""
"$(pwd)/target/release/inboxnull"