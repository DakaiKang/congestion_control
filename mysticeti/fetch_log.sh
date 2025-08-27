#!/bin/bash

# List of IP addresses
IPS=(
54.163.62.237
3.90.207.134
18.207.151.98
54.145.14.44
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
