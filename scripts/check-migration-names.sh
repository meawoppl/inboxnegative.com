#!/usr/bin/env bash
#
# Validates that all Diesel migration directories follow the naming convention:
#   - 00000000000000_<description>    (special initial migration)
#   - YYYY-MM-DD-HHMMSS_<description>  (standard timestamp format)
#
# Where <description> is lowercase snake_case (e.g. add_users_table).
#
# Usage: ./scripts/check-migration-names.sh
# Exit code: 0 if all valid, 1 if any invalid.

set -euo pipefail

MIGRATIONS_DIR="backend/migrations"
ERRORS=0

INITIAL_PATTERN='^00000000000000_[a-z][a-z0-9_]*$'
TIMESTAMP_PATTERN='^[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]{6}_[a-z][a-z0-9_]*$'

echo "Checking migration naming convention in $MIGRATIONS_DIR ..."

for dir in "$MIGRATIONS_DIR"/*/; do
    [[ -d "$dir" ]] || continue
    name=$(basename "$dir")
    [[ "$name" == .* ]] && continue

    if [[ "$name" =~ $INITIAL_PATTERN ]] || [[ "$name" =~ $TIMESTAMP_PATTERN ]]; then
        echo "  OK: $name"
    else
        echo "  ERROR: $name"
        echo "         Expected: YYYY-MM-DD-HHMMSS_snake_case_description"
        ERRORS=$((ERRORS + 1))
    fi
done

if [[ $ERRORS -gt 0 ]]; then
    echo "FAILED: $ERRORS migration(s) have invalid names"
    exit 1
fi

echo "PASSED: all migrations follow the naming convention"
