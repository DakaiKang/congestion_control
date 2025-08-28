#!/bin/bash

# List of IP addresses
IPS=(
3.93.177.17
54.91.151.6
184.72.192.186
54.235.228.115
)

# Path to your SSH private key
KEY=~/.ssh/dakai_dev.pem

# Remote user (adjust if needed, e.g., ubuntu/ec2-user)
USER=ubuntu

# Destination folder for logs
DEST=./logs
mkdir -p "$DEST"

# Loop through IPs and fetch logs
for ip in "${IPS[@]}"; do
    echo "Fetching node.log from $ip..."
    scp -o StrictHostKeyChecking=no -i "$KEY" "$USER@$ip:~/node.log" "$DEST/node-$ip.log" &
done

wait

echo "✅ All logs fetched into $DEST/"
