#!/bin/bash
# Round-5 verifier fixes on the round-4 signet receipt (signet/gpu_r4/block_2600.rcpt). Run by run_r5.sh.
# The receipt is from image c3501ac9, so checks A-G need a verifier built for THAT image: HR=<host of image
# c3501ac9> (its verifier code must match this commit's; it has been unchanged since round 6). Refuses otherwise,
# rather than recording METHOD_ID failures as evidence.
R=~/libzync-gpu/r18; E=~/dev/hazync-libzync/experimental/libzync-bench/signet; G=$E/gpu_r4; H=$R/host_signet; O=$R/r5; mkdir -p $O
RECEIPT_IMAGE=c3501ac900743bbc911c8f6dedcd71f1ee53fe4bea4f1cbc9726324478e63dae
HR=${HR:?set HR to a signet host built for image $RECEIPT_IMAGE}
$HR method-id 2>/dev/null | grep -qw $RECEIPT_IMAGE || { echo "HR=$HR is not a host of image $RECEIPT_IMAGE"; exit 1; }
echo "== A. receipt, real clock, effective anchor (expect DEMO exit 2)"; $HR libzync-verify $G/block_2600.rcpt $E/signet2600.hints $G/anchor_effective.json | tail -1; echo "exit=${PIPESTATUS[0]}"
echo "== B. HAZYNC_VERIFY_NOW set to a passing time (expect FAIL exit 1, override named)"; HAZYNC_VERIFY_NOW=1793400000 $HR libzync-verify $G/block_2600.rcpt $E/signet2600.hints $G/anchor_effective.json | grep -a 'HAZYNC_VERIFY_NOW\|time-too-new\|VERIFY_'; echo "exit=${PIPESTATUS[0]}"
echo "== C. HAZYNC_VERIFY_NOW before the last block (expect time-too-new FAIL)"; HAZYNC_VERIFY_NOW=1599700000 $HR libzync-verify $G/block_2600.rcpt $E/signet2600.hints $G/anchor_effective.json | grep -a 'time-too-new\|VERIFY_'; echo "exit=${PIPESTATUS[0]}"
echo "== D. unparsable override (expect FAIL exit 1)"; HAZYNC_VERIFY_NOW=1599700000s $HR libzync-verify $G/block_2600.rcpt $E/signet2600.hints $G/anchor_effective.json | grep -a 'HAZYNC_VERIFY_NOW\|VERIFY_'; echo "exit=${PIPESTATUS[0]}"
echo "== D2. empty override = unset (expect DEMO exit 2)"; HAZYNC_VERIFY_NOW= $HR libzync-verify $G/block_2600.rcpt $E/signet2600.hints $G/anchor_effective.json | tail -1; echo "exit=${PIPESTATUS[0]}"
echo "== E. missing receipt / garbage receipt / missing hints / missing argument (expect FAIL exit 1, not 101)"
$HR libzync-verify /nonexistent.rcpt $E/signet2600.hints | head -1; echo "exit=${PIPESTATUS[0]}"
head -c 1000 /dev/urandom > $O/junk.rcpt; $HR libzync-verify $O/junk.rcpt $E/signet2600.hints | head -1; echo "exit=${PIPESTATUS[0]}"
$HR libzync-verify $G/block_2600.rcpt /nonexistent.hints $G/anchor_effective.json | head -1; echo "exit=${PIPESTATUS[0]}"
$HR libzync-verify $G/block_2600.rcpt | head -1; echo "exit=${PIPESTATUS[0]}"
echo "== E2. stdout closed early (expect no abort: exit 1 or 2, not 134)"; $HR libzync-verify $G/block_2600.rcpt $E/signet2600.hints $G/anchor_effective.json | head -c 1 > /dev/null; echo "exit=${PIPESTATUS[0]}"
echo "== F. verify-chain on a libzync build (expect refusal exit 1)"; $HR verify-chain $G/block_2600.rcpt; echo "exit=$?"
echo "== G. swiftsync-file-hash: canonical (exit 0) / trailing byte (exit 1)"
$HR swiftsync-file-hash $E/signet2600.hints; echo "exit=$?"
cp $E/signet2600.hints $O/trail.hints; printf '\x00' >> $O/trail.hints; $HR swiftsync-file-hash $O/trail.hints; echo "exit=$?"
echo "== H. prove-range replay path, dev mode: anchor without max_time, blocks 1..2500 present -> filled exactly, effective anchor == sweep state"
python3 -c "import json;a=json.load(open('$R/r5/reg/sig/state_2500.json'));a.pop('max_time');json.dump(a,open('$O/anchor_nomax.json','w'))"
rm -rf $O/pr; RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_SS_HINTS=$E/signet2600.hints $H libzync-prove-range 2501 2501 ~/libzync-gpu/signet/json $O/anchor_nomax.json $O/pr 2>&1 | grep -a 'LZ_PROVE_RANGE_DONE\|WARNING: anchor'
python3 -c "import json;e=json.load(open('$O/pr/anchor_effective.json'));a=json.load(open('$R/r5/reg/sig/state_2500.json'));print('filled max_time',e['max_time'],'| effective anchor == sweep state:',e==a)"
echo R5_SIGNET_DONE
