#!/bin/bash

# Test user email address (hashed version of testing@gmail.com)
TEST_EMAIL="21c179d70d4a07a7@inboxnegative.com"

# Choose which email to send based on first argument
if [ "$1" == "welcome" ]; then
    echo "Sending welcome email to ${TEST_EMAIL}..."
    swaks --to ${TEST_EMAIL} \
        --from welcome@inboxnull.example.com \
        --header "Subject: Welcome to InboxNull!" \
        --header "Content-Type: text/html" \
        --body "$(cat $(dirname "$0")/emails/welcome_email.html)" \
        --server localhost:2525
elif [ "$1" == "newsletter" ]; then
    echo "Sending newsletter email to ${TEST_EMAIL}..."
    swaks --to ${TEST_EMAIL} \
        --from newsletter@inboxnull.example.com \
        --header "Subject: InboxNull Newsletter - March 2025" \
        --header "Content-Type: text/html" \
        --body "$(cat $(dirname "$0")/emails/newsletter.html)" \
        --server localhost:2525
elif [ "$1" == "attachment" ]; then
    echo "Sending email with attachment to ${TEST_EMAIL}..."

    # Create a temporary test file if it doesn't exist
    TEMP_FILE="/tmp/test_attachment.txt"
    echo "This is a test attachment created at $(date)" > ${TEMP_FILE}

    swaks --to ${TEST_EMAIL} \
        --from files@inboxnull.example.com \
        --header "Subject: Test Email with Attachment" \
        --body "This email contains a test attachment." \
        --attach ${TEMP_FILE} \
        --server localhost:2525

    echo "Email with attachment sent!"
else
    # Default behavior - send text email in a loop
    echo "Sending plain text test emails to ${TEST_EMAIL} in a loop..."
    while true; do
        swaks --to ${TEST_EMAIL} \
            --from sender@example.com \
            --header "Subject: Test Email" \
            --body "This is a test email sent at $(date)" \
            --server localhost:2525
        sleep 5
    done
fi
