p="/root/hvneg28/src-xv/prover/host/src/lz.rs"; s=open(p).read()
old='''        let st = crate::engine_backend::decode_state(&journal);
        assert!(st.kind == KIND_CHAIN && st.self_id == last.image, "{p} is not the predecessor's chain state");
        assert_eq!(st.height + 1, last.first_height, "{p}: the pin hands over at {}", last.first_height);'''
new='''        // R1-5 #3 TEST-ONLY BYPASS (hazync-coord /root/hvneg28, 28.11.2026; never committed): a malicious prover runs its own
        // host, so the guest's refusal is what the test measures. HZ_TEST_HANDOVER_STATE=<journal>: the host builds its
        // witnesses from THAT (genuine) chain state and skips its own two checks; the guest still receives <p>'s journal + proof.
        let st = match std::env::var("HZ_TEST_HANDOVER_STATE") {
            Ok(t) => crate::engine_backend::decode_state(&std::fs::read(&t).expect("HZ_TEST_HANDOVER_STATE")),
            Err(_) => {
                let st = crate::engine_backend::decode_state(&journal);
                assert!(st.kind == KIND_CHAIN && st.self_id == last.image, "{p} is not the predecessor's chain state");
                assert_eq!(st.height + 1, last.first_height, "{p}: the pin hands over at {}", last.first_height);
                st
            }
        };'''
assert s.count(old)==1; open(p,"w").write(s.replace(old,new)); print("PATCHED")
