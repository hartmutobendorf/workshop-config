#!/usr/bin/env bash
set -euo pipefail

# Extract workshop name from workshop.yaml
WORKSHOP=$(grep "^name:" workshop.yaml | cut -d: -f2 | xargs)
PROJECT_PATH="/project"

PLATFORM="$(uname -s)"

if [[ "$PLATFORM" == "Darwin" ]]; then
  # macOS: use multipass to reach the workshop-host VM, then lxc inside it

  # Ensure workshop-host instance exists and is running
  STATE=$(multipass list --format csv | awk -F, -v name="workshop-host" '$1 == name {print $2}')
  if [[ -z "$STATE" ]]; then
    echo "Error: no multipass instance named 'workshop-host' found."
    echo "Create one first with: multipass launch --name workshop-host"
    exit 1
  fi
  if [[ "$STATE" != "Running" ]]; then
    echo "Starting multipass instance 'workshop-host'..."
    multipass start workshop-host
  fi

  # Get the workshop-host VM IP for use as SSH jump host
  JUMP_IP=$(multipass list --format csv | awk -F, -v name="workshop-host" '$1 == name {print $3}')
  if [[ -z "$JUMP_IP" ]]; then
    echo "Error: could not determine IP of 'workshop-host'."
    exit 1
  fi

  # Get the workshop container IP via workshop shell inside the multipass VM
  IP=$(multipass exec workshop-host -- lxc list --all-projects --format csv -c 4 "$WORKSHOP" 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+' | head -1)

  if [[ -z "$IP" ]]; then
    echo "Error: could not find a running workshop named '$WORKSHOP' inside workshop-host."
    echo "Make sure it's running with: workshop launch"
    exit 1
  fi

  echo "Opening VSCode remote session at $IP (via jump host $JUMP_IP)..."
  code --folder-uri "vscode-remote://ssh-remote+workshop@$IP$PROJECT_PATH" \
    --ssh-config-args "-J ubuntu@$JUMP_IP"

else
  # Linux: use lxc directly

  # Get the IPv4 address of the workshop (strips CIDR suffix e.g. 10.x.x.x/24 -> 10.x.x.x)
  IP=$(lxc list --all-projects --format csv -c 4 "$WORKSHOP" 2>/dev/null | grep -oP '\d+\.\d+\.\d+\.\d+' | head -1)

  if [[ -z "$IP" ]]; then
    echo "Error: could not find a running workshop named '$WORKSHOP'."
    echo "Make sure it's running with: workshop launch"
    exit 1
  fi

  echo "Opening VSCode remote session at $IP..."
  code --folder-uri "vscode-remote://ssh-remote+workshop@$IP$PROJECT_PATH"
fi
