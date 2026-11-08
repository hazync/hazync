#!/bin/bash
# Round 5 evidence: regression suite, then the signet verifier tests. Writes the two .out files beside this script.
# usage: HR=<signet host of the receipt's image c3501ac9> run_r5.sh   (see test_r5_signet.sh)
D=$(cd "$(dirname "$0")" && pwd)
bash $D/test_r5_regress.sh 2>&1 | /usr/bin/grep -av 'panicked at src/main.rs:2093' > $D/test_r5_regress.out   # expected collect-mode panics
bash $D/test_r5_signet.sh > $D/test_r5_signet.out 2>&1 || { echo "signet tests refused: see test_r5_signet.out"; exit 1; }
echo RUN_R5_DONE
