mod rejection_target {
    goodfetch_test_support::fuzz_replay!(crate::rejections::fuzz_rejection, "fuzz/rejection");
}
