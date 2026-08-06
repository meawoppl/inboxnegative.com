# InboxNull

A temporary email service with high security through data transience - emails exist only while you're viewing them.

## Local Development and Testing

### Database Setup

InboxNull now supports using a PostgreSQL database for statistics:

1. Install PostgreSQL and development libraries if you haven't already:
   ```bash
   # On Ubuntu/Debian:
   sudo apt-get install postgresql postgresql-contrib libpq-dev

   # On Fedora/RHEL:
   sudo dnf install postgresql postgresql-devel

   # On macOS with Homebrew:
   brew install postgresql
   ```

2. Create a database for InboxNull (or use the existing Neon DB credentials)
3. Copy the `.env.example` file to `.env` in the backend directory:
   ```bash
   cp backend/.env.example backend/.env
   ```
4. Update the `DATABASE_URL` in the `.env` file with your PostgreSQL credentials
5. The application will automatically:
   - Connect to the database
   - Create necessary tables if they don't exist
   - Migrate any existing stats from the JSON file to the database

The database is required. If the connection pool cannot be created, the application
logs the reason and exits rather than starting. There is deliberately no fallback:
without a database, deletion counts read back as zero for every user while the
service otherwise looks healthy, which is far harder to notice than a failed boot.

### ZMQ socket location

The backend publishes internal messages over a ZMQ `ipc://` socket. Its path is
absolute and configurable:

| Variable | Default | Notes |
|---|---|---|
| `ZMQ_SOCKET_DIR` | `$XDG_RUNTIME_DIR`, else `/tmp` | Must be absolute and writable. A relative value is refused and the default used instead, so the socket never depends on the working directory. |

The socket file is named `local_publisher_<pid>` and is removed on clean shutdown.

### Testing Mode

When developing or testing locally, you can use the testing mode to bypass Google OAuth authentication:

1. Set the `TESTING_MODE=true` environment variable when running the backend
2. Use the test login button on the login screen or navigate to `/api/test-login`
3. You'll be automatically logged in as `testing@gmail.com`

Run the included testing script to start the application in testing mode:

```bash
./testing.sh
```

After starting the application in testing mode, you can send test emails to the test account using:

```bash
# Send a simple text email in a loop (every 5 seconds)
./test_send.sh

# Send a welcome HTML email
./test_send.sh welcome

# Send a newsletter HTML email
./test_send.sh newsletter
```

The test emails will be automatically sent to `21c179d70d4a07a7@inboxnegative.com`, which is the properly hashed version of the test account email address.

### Complete Testing Workflow

1. Start the application in testing mode:
   ```
   ./testing.sh
   ```

2. Open your browser to http://localhost:8080

3. Click on the "Use Test Account" button to log in as testing@gmail.com

4. In another terminal, send some test emails:
   ```
   ./test_send.sh welcome
   ```

5. You should see the emails appear in real-time in your browser window

This allows you to test the HTML email rendering functionality without setting up OAuth credentials.