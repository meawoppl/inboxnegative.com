#!/bin/bash

# Send welcome email
swaks --to noone@inboxnegative.com \
    --from welcome@inboxnull.example.com \
    --header "Subject: Welcome to InboxNull!" \
    --header "Content-Type: text/html" \
    --body "$(cat /home/meawoppl/repos/inboxnull/emails/welcome_email.html)" \
    --server localhost:2525

# Send newsletter email
swaks --to noone@inboxnegative.com \
    --from newsletter@inboxnull.example.com \
    --header "Subject: InboxNull Newsletter - March 2025" \
    --header "Content-Type: text/html" \
    --body "$(cat /home/meawoppl/repos/inboxnull/emails/newsletter.html)" \
    --server localhost:2525