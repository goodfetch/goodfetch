use super::*;

/// A refresh's work folder after its guided build: the profile and hot
/// order in `guided`, and Foxhound's executable.
struct Built {
    work: tempfile::TempDir,
}

impl Built {
    fn new() -> Self {
        let work = tempfile::tempdir().expect("a work folder");
        std::fs::create_dir(work.path().join("guided")).expect("the guided folder");
        let built = Self { work };
        built.write("guided/merged.profdata", "profile");
        built.write(
            "guided/order.txt",
            "_RNvCs1_4main3hot\n_RNvCs1_4main4warm\n",
        );
        built.write("foxhound", "executable");
        built
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.work.path().join(name), contents).expect("a file of the build");
    }

    fn record(&self) {
        record(self.work.path(), &self.foxhound()).expect("the record");
    }

    fn hot(&self) -> Result<Vec<String>> {
        recorded_hot(self.work.path(), &self.foxhound())
    }

    fn foxhound(&self) -> PathBuf {
        self.work.path().join("foxhound")
    }
}

#[test]
fn the_guided_build_of_the_profile_on_disk_gives_its_hot_order() {
    let built = Built::new();
    built.record();

    assert_eq!(
        built.hot().expect("the build is the profile's"),
        ["_RNvCs1_4main3hot", "_RNvCs1_4main4warm"]
    );
}

#[test]
fn a_build_no_refresh_recorded_is_refused() {
    let built = Built::new();

    let refused = built.hot().expect_err("no refresh recorded the build");

    assert!(
        format!("{refused:#}").contains("no guided build is recorded"),
        "{refused:#}"
    );
}

#[rstest::rstest]
#[case::the_profile("guided/merged.profdata", "the merged profile")]
#[case::the_order("guided/order.txt", "the hot order")]
#[case::the_executable("foxhound", "Foxhound's executable")]
fn a_build_whose_inputs_changed_since_is_refused(#[case] changed: &str, #[case] named: &str) {
    let built = Built::new();
    built.record();
    built.write(changed, "another");

    let refused = built
        .hot()
        .expect_err("the build is no longer the profile's");

    assert!(
        format!("{refused:#}").contains(&format!("{named} changed")),
        "{refused:#}"
    );
}
