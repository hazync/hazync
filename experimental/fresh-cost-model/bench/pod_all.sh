#!/bin/bash
cd /workspace
until grep -q SETUP_DONE setup.log; do sleep 20; done
./pod_prove2.sh > prove2.log 2>&1
./pod_hazync.sh > hazync.log 2>&1
echo ALL_DONE >> prove2.log
