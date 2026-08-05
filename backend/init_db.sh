#!/bin/bash
set -e

# Load environment variables from .env file
if [ -f .env ]; then
    source .env
else
    echo "Error: .env file not found"
    echo "Please copy .env.example to .env and update the database credentials"
    exit 1
fi

if [ -z "$DATABASE_URL" ]; then
    echo "Error: DATABASE_URL environment variable not set"
    exit 1
fi

echo "Initializing database schema..."

# Extract connection parameters for Neon DB
# URL format is postgresql://username:password@hostname/dbname?parameters

# The application runs embedded migrations automatically on startup; this
# script is only for manually initializing a fresh database out of band.
psql "$DATABASE_URL" -f migrations/00000000000000_initial/up.sql

echo "Database schema initialized successfully!"