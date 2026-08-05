#!/bin/bash
set -e

# Resolve paths relative to this script. The bodies were previously read from a
# hardcoded /home/meawoppl/repos/inboxnull/ path, which no longer exists.
cd "$(dirname "$0")"

# Send welcome email
swaks --to noone@inboxnegative.com \
    --from welcome@inboxnull.example.com \
    --header "Subject: Welcome to InboxNull!" \
    --header "Content-Type: text/html" \
    --body "$(cat welcome_email.html)" \
    --server localhost:2525

# Send newsletter email
swaks --to noone@inboxnegative.com \
    --from newsletter@inboxnull.example.com \
    --header "Subject: InboxNull Newsletter - March 2025" \
    --header "Content-Type: text/html" \
    --body "$(cat newsletter.html)" \
    --server localhost:2525
