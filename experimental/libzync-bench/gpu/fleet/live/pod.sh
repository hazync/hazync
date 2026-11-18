#!/bin/bash
# historical run script; uses RunPod GraphQL, retired early 2027 — port before reuse
# pod.sh status|term <id> — read-only status of a pod, or terminate it.
RP=$(cat ~/.hazync/runpod.key)
gql(){ curl -s -X POST -H 'Content-Type: application/json' -H 'User-Agent: hazync-libzync/1' -H "Authorization: Bearer $RP" -d "$1" https://api.runpod.io/graphql; }
case $1 in
 status) gql '{"query":"query { myself { clientBalance currentSpendPerHr pods { id name costPerHr runtime { uptimeInSeconds ports { ip isIpPublic privatePort publicPort } } } } }"}' ;;
 term) gql "{\"query\":\"mutation { podTerminate(input: {podId: \\\"$2\\\"}) }\"}" ;;
esac
