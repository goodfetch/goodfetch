use goodfetch_protocol::plugin::Limits;
use rstest::rstest;

use super::{Resource, Rlimit64};

/// Each limit's soft and hard rlimits: `cpu_time`'s hard limit is one
/// second past its soft one, so that `SIGXCPU` comes before `SIGKILL`.
#[rstest]
#[case::memory(Resource::Memory, 64 * 1024 * 1024, 64 * 1024 * 1024)]
#[case::cpu_time(Resource::CpuTime, 60, 61)]
#[case::open_files(Resource::OpenFiles, 32, 32)]
fn each_limit_is_its_rlimits_soft_and_hard_value(
    #[case] resource: Resource,
    #[case] soft: u64,
    #[case] hard: u64,
) {
    let limits = Limits {
        memory: 64 * 1024 * 1024,
        cpu_time: 60,
        open_files: 32,
        ..Limits::default()
    };

    assert_eq!(resource.limit(&limits), Rlimit64 { soft, hard });
}
