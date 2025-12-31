#!/bin/bash
set -e
oras login --ca-file /home/cc/Desktop/harbor/certs/harbor.crt  -u admin -p Harbor12345 192.168.31.138
oras pull --ca-file=/home/cc/Desktop/harbor/certs/harbor.crt 192.168.31.138/ntx/executor:v0.0.1 \
  -o ./tmp