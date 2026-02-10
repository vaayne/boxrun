#!/usr/bin/env bash
# BoxRun CLI examples — run these after starting the server with `boxrun serve`
#
# Each section is self-contained. You can copy-paste individual commands.

set -euo pipefail

echo "=== 1. One-shot ephemeral run ==="
# Run a command in a temporary box (auto-created and auto-removed)
boxrun run ubuntu:24.04 echo "Hello from BoxRun!"

echo ""
echo "=== 2. Create a persistent box ==="
boxrun create ubuntu:24.04 --name mybox

echo ""
echo "=== 3. Execute commands ==="
# Use -- to separate boxrun flags from the command flags
boxrun exec mybox -- uname -a
boxrun exec mybox -- sh -c "echo 'Hello from inside the VM'"

echo ""
echo "=== 4. File transfer ==="
# Upload a file from host to box
echo "data from host" > /tmp/boxrun-example.txt
boxrun cp /tmp/boxrun-example.txt mybox:/root/input.txt

# Read it inside the box
boxrun exec mybox -- cat /root/input.txt

# Write output inside the box
boxrun exec mybox -- sh -c "wc -c /root/input.txt > /root/output.txt"

# Download result from box to host
boxrun cp mybox:/root/output.txt /tmp/boxrun-result.txt
cat /tmp/boxrun-result.txt

# Clean up local temp files
rm -f /tmp/boxrun-example.txt /tmp/boxrun-result.txt

echo ""
echo "=== 5. List boxes ==="
boxrun ls

echo ""
echo "=== 6. Stop and restart ==="
boxrun stop mybox
boxrun ls --status stopped
boxrun start mybox

echo ""
echo "=== 7. Different images ==="
# Run Python code
boxrun run python:3.12-slim -- python3 -c "print(sum(range(100)))"

# Run Node.js code
boxrun run node:22-slim -- node -e "console.log('Node ' + process.version)"

echo ""
echo "=== 8. Clean up ==="
boxrun rm mybox --force
boxrun ls

echo ""
echo "Done!"
