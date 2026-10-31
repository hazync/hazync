#!/bin/bash
# Round 5 evidence: regression suite, then the signet verifier tests. Writes the two .out files beside this script.
D=$(cd "$(dirname "$0")" && pwd)
bash $D/test_r5_regress.sh 2>&1 | /usr/bin/grep -av 'panicked at src/main.rs:2093' > $D/test_r5_regress.out   # expected collect-mode panics
bash $D/test_r5_signet.sh > $D/test_r5_signet.out 2>&1
echo RUN_R5_DONE
