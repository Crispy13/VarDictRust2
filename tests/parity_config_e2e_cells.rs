use libtest_mimic::{Arguments, Failed, Trial};

mod common;

fn main() {
    if std::env::var_os("PARITY_REGION_INDEX").is_some() {
        eprintln!(
            "error: PARITY_REGION_INDEX must NOT be set when running parity_config_e2e_cells; use VARDICT_CELL_SHARD=i/N instead"
        );
        std::process::exit(2);
    }

    let args = Arguments::from_args();
    let trials = build_cell_trials();
    libtest_mimic::run(&args, trials).exit();
}

struct Shard {
    i: usize,
    n: usize,
}

fn parse_shard_env() -> Option<Shard> {
    let raw = std::env::var("VARDICT_CELL_SHARD").ok()?;
    let fail = |why: &str| -> ! {
        eprintln!(
            "error: VARDICT_CELL_SHARD={raw:?} invalid ({why}); expected \"i/N\" with 0 <= i < N and N > 0"
        );
        std::process::exit(2);
    };

    let mut parts = raw.split('/');
    let i_str = parts.next().unwrap_or_else(|| fail("empty"));
    let n_str = parts.next().unwrap_or_else(|| fail("missing '/'"));
    if parts.next().is_some() {
        fail("extra '/' segments");
    }
    if i_str.is_empty() {
        fail("empty i");
    }
    if n_str.is_empty() {
        fail("empty N");
    }

    let i = i_str.parse::<usize>().unwrap_or_else(|_| fail("non-numeric i"));
    let n = n_str.parse::<usize>().unwrap_or_else(|_| fail("non-numeric N"));
    if n == 0 {
        fail("N == 0");
    }
    if i >= n {
        fail("i >= N");
    }

    Some(Shard { i, n })
}

fn build_cell_trials() -> Vec<Trial> {
    let config_name = "T1-01";
    let slug = common::config_name_to_slug(config_name);
    let region_count = common::load_region_config().len();
    let shard = parse_shard_env();

    let mut trials: Vec<Trial> = (0..region_count)
        .map(|region_idx| {
            let trial_name = format!("parity_config_e2e_cell_{}_r{:03}", slug, region_idx);
            let config_name = config_name.to_string();
            Trial::test(trial_name, move || {
                common::run_cell(&config_name, region_idx).map_err(Failed::from)
            })
            .with_ignored_flag(true)
        })
        .collect();

    trials.sort_by(|left, right| left.name().cmp(right.name()));
    if let Some(shard) = shard {
        let trial_count = trials.len();
        let lo = (shard.i * trial_count) / shard.n;
        let hi = ((shard.i + 1) * trial_count) / shard.n;
        trials.drain(hi..);
        trials.drain(..lo);
    }

    trials
}