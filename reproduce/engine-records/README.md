# Published engine image records

One file per image a RELEASE published, named `<image id hex>.json`: `{"image": <64 hex>, "key_pin": <SHA-256 of the
key file, 64 hex>, "openvm": <version>, "test": false}`. A later image that hands over from one (a cross-version
handover, engine/HANDOVER.md) must pin exactly that `key_pin`: engine/guest/check_xver_provenance.py refuses the
assembly otherwise (review R1-5 #1, 27.11.2026). No engine image is published yet (Stage 4); the one record here is the
step-3 rehearsal's predecessor, marked `"test": true`, which only the test-lineage variant (lineage/test.rs) may use.
