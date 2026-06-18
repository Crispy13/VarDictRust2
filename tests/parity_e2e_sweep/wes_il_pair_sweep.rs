use libtest_mimic::Trial;

pub(crate) fn build_trials() -> Vec<Trial> {
    super::sweep_common::build_trials("wes_il_pair")
}
