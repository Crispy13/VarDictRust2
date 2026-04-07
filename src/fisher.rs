use crate::utils::round_half_even;
use statrs::function::gamma::ln_gamma;

const RESULT_ROUND_R: f64 = 1e5;
const TWO_SIDED_REL_ERR: f64 = 1.0 + 1e-7;

#[derive(Clone, Debug)]
pub struct FisherExact {
    logdc: Vec<f64>,
    m: i32,
    n: i32,
    k: i32,
    x: i32,
    lo: i32,
    hi: i32,
    p_value_less: f64,
    p_value_greater: f64,
    p_value_two_sided: f64,
    support: Vec<i32>,
}

fn lower_support_bound(population: i32, successes: i32, draws: i32) -> i32 {
    (draws - (population - successes)).max(0)
}

fn upper_support_bound(successes: i32, draws: i32) -> i32 {
    successes.min(draws)
}

fn sanitize_log_probability(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        round_half_even("0.0000000", value)
    }
}

fn log_binomial_coefficient(n: i32, k: i32) -> f64 {
    if k < 0 || k > n {
        return f64::NEG_INFINITY;
    }

    ln_gamma(f64::from(n) + 1.0) - ln_gamma(f64::from(k) + 1.0) - ln_gamma(f64::from(n - k) + 1.0)
}

fn hypergeometric_log_probability(
    population: i32,
    successes: i32,
    draws: i32,
    observed: i32,
) -> f64 {
    let lo = lower_support_bound(population, successes, draws);
    let hi = upper_support_bound(successes, draws);
    if observed < lo || observed > hi {
        return f64::NEG_INFINITY;
    }

    let failures = population - successes;
    log_binomial_coefficient(successes, observed)
        + log_binomial_coefficient(failures, draws - observed)
        - log_binomial_coefficient(population, draws)
}

fn hypergeometric_probability(population: i32, successes: i32, draws: i32, observed: i32) -> f64 {
    hypergeometric_log_probability(population, successes, draws, observed).exp()
}

fn hypergeometric_cumulative_probability(
    population: i32,
    successes: i32,
    draws: i32,
    q: i32,
) -> f64 {
    let lo = lower_support_bound(population, successes, draws);
    let hi = upper_support_bound(successes, draws);
    if q < lo {
        return 0.0;
    }
    if q >= hi {
        return 1.0;
    }

    (lo..=q)
        .map(|observed| hypergeometric_probability(population, successes, draws, observed))
        .sum()
}

fn hypergeometric_upper_cumulative_probability(
    population: i32,
    successes: i32,
    draws: i32,
    q: i32,
) -> f64 {
    let lo = lower_support_bound(population, successes, draws);
    let hi = upper_support_bound(successes, draws);
    if q <= lo {
        return 1.0;
    }
    if q > hi {
        return 0.0;
    }

    (q..=hi)
        .map(|observed| hypergeometric_probability(population, successes, draws, observed))
        .sum()
}

// Java: FisherExact.round_as_r() L164-L169
fn round_as_r(value: f64) -> f64 {
    let rounded_value = round_half_even("0", value * RESULT_ROUND_R) / RESULT_ROUND_R;
    if rounded_value == 0.0 {
        0.0
    } else if rounded_value == 1.0 {
        1.0
    } else {
        rounded_value
    }
}

// Java: UnirootZeroIn.zeroinC() L22-L101
fn zeroin_c<F>(ax: f64, bx: f64, function: F, tolerance: f64) -> f64
where
    F: Fn(f64) -> f64,
{
    let mut a = ax;
    let mut b = bx;
    let mut fa = function(a);
    let mut fb = function(b);
    let mut c = a;
    let mut fc = fa;
    let epsilon = f64::EPSILON;

    loop {
        let prev_step = b - a;
        let mut new_step = (c - b) / 2.0;

        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }

        let tolerance_active = 2.0 * epsilon * b.abs() + tolerance / 2.0;

        if new_step.abs() <= tolerance_active || fb == 0.0 {
            return b;
        }

        if prev_step.abs() >= tolerance_active && fa.abs() > fb.abs() {
            let cb = c - b;
            let (mut p, mut q) = if a == c {
                let t1 = fb / fa;
                (cb * t1, 1.0 - t1)
            } else {
                let q = fa / fc;
                let t1 = fb / fc;
                let t2 = fb / fa;
                (
                    t2 * (cb * q * (q - t1) - (b - a) * (t1 - 1.0)),
                    (q - 1.0) * (t1 - 1.0) * (t2 - 1.0),
                )
            };

            if p > 0.0 {
                q = -q;
            } else {
                p = -p;
            }

            if p < (0.75 * cb * q - (tolerance_active * q).abs() / 2.0)
                && p < (prev_step * q / 2.0).abs()
            {
                new_step = p / q;
            }
        }

        if new_step.abs() < tolerance_active {
            new_step = if new_step > 0.0 {
                tolerance_active
            } else {
                -tolerance_active
            };
        }

        a = b;
        fa = fb;
        b += new_step;
        fb = function(b);

        if (fb > 0.0 && fc > 0.0) || (fb < 0.0 && fc < 0.0) {
            c = a;
            fc = fa;
        }
    }
}

impl FisherExact {
    // Java: FisherExact(int,int,int,int) L41-L55
    pub fn new(ref_fwd: i32, ref_rev: i32, alt_fwd: i32, alt_rev: i32) -> Self {
        let m = ref_fwd + ref_rev;
        let n = alt_fwd + alt_rev;
        let k = ref_fwd + alt_fwd;
        let x = ref_fwd;
        let lo = (k - n).max(0);
        let hi = k.min(m);
        let support = (lo..=hi).collect::<Vec<_>>();

        let mut fisher = Self {
            logdc: Vec::new(),
            m,
            n,
            k,
            x,
            lo,
            hi,
            p_value_less: 0.0,
            p_value_greater: 0.0,
            p_value_two_sided: 0.0,
            support,
        };
        fisher.logdc = fisher.logdc_dhyper();
        fisher.calculate_p_value();
        fisher
    }

    // Java: FisherExact.logdcDhyper() L58-L75
    fn logdc_dhyper(&self) -> Vec<f64> {
        self.support
            .iter()
            .map(|&element| {
                if self.m + self.n == 0 {
                    0.0
                } else {
                    sanitize_log_probability(hypergeometric_log_probability(
                        self.m + self.n,
                        self.m,
                        self.k,
                        element,
                    ))
                }
            })
            .collect()
    }

    // Java: FisherExact.mle() L85-L101
    fn mle(&self, observed: f64) -> f64 {
        let epsilon = f64::EPSILON;
        if observed == f64::from(self.lo) {
            return 0.0;
        }
        if observed == f64::from(self.hi) {
            return f64::INFINITY;
        }

        let mu = self.mnhyper(1.0);
        if mu > observed {
            zeroin_c(
                0.0,
                1.0,
                |value| self.mnhyper(value) - observed,
                epsilon.powf(0.25),
            )
        } else if mu < observed {
            1.0 / zeroin_c(
                epsilon,
                1.0,
                |value| self.mnhyper(1.0 / value) - observed,
                epsilon.powf(0.25),
            )
        } else {
            1.0
        }
    }

    // Java: FisherExact.mnhyper() L103-L115
    fn mnhyper(&self, non_centrality_parameter: f64) -> f64 {
        if non_centrality_parameter == 0.0 {
            return f64::from(self.lo);
        }
        if non_centrality_parameter.is_infinite() {
            return f64::from(self.hi);
        }

        self.support
            .iter()
            .zip(self.dnhyper(non_centrality_parameter))
            .map(|(&support_value, probability)| f64::from(support_value) * probability)
            .sum()
    }

    // Java: FisherExact.dnhyper() L117-L134
    fn dnhyper(&self, non_centrality_parameter: f64) -> Vec<f64> {
        debug_assert!(non_centrality_parameter > 0.0);
        debug_assert!(non_centrality_parameter.is_finite());

        let weighted_logs = self
            .logdc
            .iter()
            .zip(self.support.iter())
            .map(|(&log_probability, &support_value)| {
                log_probability + non_centrality_parameter.ln() * f64::from(support_value)
            })
            .collect::<Vec<_>>();
        let max_weighted_log = weighted_logs
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let exponentiated = weighted_logs
            .iter()
            .map(|value| (value - max_weighted_log).exp())
            .collect::<Vec<_>>();
        let sum: f64 = exponentiated.iter().sum();

        exponentiated.into_iter().map(|value| value / sum).collect()
    }

    // Java: FisherExact.getOddRatio() L136-L145
    pub fn get_odd_ratio(&self) -> String {
        let odd_ratio = self.mle(f64::from(self.x));
        if odd_ratio.is_infinite() {
            "Inf".to_string()
        } else if odd_ratio == odd_ratio.round() {
            format!("{odd_ratio:.0}")
        } else {
            round_as_r(odd_ratio).to_string()
        }
    }

    // Java: FisherExact.getPValue() L147-L149
    pub fn get_p_value(&self) -> f64 {
        round_as_r(self.p_value_two_sided)
    }

    // Java: FisherExact.getLogdc() L151-L154
    pub fn get_logdc(&mut self) -> &[f64] {
        self.logdc = self.logdc_dhyper();
        &self.logdc
    }

    // Java: FisherExact.getPValueGreater() L156-L158
    pub fn get_p_value_greater(&self) -> f64 {
        round_as_r(self.p_value_greater)
    }

    // Java: FisherExact.getPValueLess() L160-L162
    pub fn get_p_value_less(&self) -> f64 {
        round_as_r(self.p_value_less)
    }

    // Java: FisherExact.calculatePValue() L171-L184
    fn calculate_p_value(&mut self) {
        self.p_value_less = self.pnhyper(self.x, false);
        self.p_value_greater = self.pnhyper(self.x, true);

        let distribution = self.dnhyper(1.0);
        let observed_probability =
            distribution[usize::try_from(self.x - self.lo).expect("support index")];
        self.p_value_two_sided = distribution
            .iter()
            .copied()
            .filter(|value| *value <= observed_probability * TWO_SIDED_REL_ERR)
            .sum();
    }

    // Java: FisherExact.pnhyper() L186-L197
    fn pnhyper(&self, q: i32, upper_tail: bool) -> f64 {
        if self.m + self.n == 0 {
            return 1.0;
        }

        if upper_tail {
            hypergeometric_upper_cumulative_probability(self.m + self.n, self.m, self.k, q)
        } else {
            hypergeometric_cumulative_probability(self.m + self.n, self.m, self.k, q)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64, epsilon: f64) {
        let delta = (actual - expected).abs();
        assert!(
            delta <= epsilon,
            "expected {expected}, got {actual}, delta {delta} > {epsilon}"
        );
    }

    #[test]
    fn log_binomial_coefficient_matches_known_values() {
        assert_close(log_binomial_coefficient(10, 3), 4.787491742782046, 1e-12);
        assert_close(log_binomial_coefficient(0, 0), 0.0, 1e-12);
        assert_close(log_binomial_coefficient(5, 0), 0.0, 1e-12);
        assert_close(log_binomial_coefficient(5, 5), 0.0, 1e-12);
    }

    #[test]
    fn hypergeometric_log_probability_matches_exact_combinatorics() {
        assert_close(
            hypergeometric_log_probability(20, 10, 10, 5),
            -1.067933139579608,
            1e-12,
        );
    }

    #[test]
    fn hypergeometric_log_probability_matches_lower_extreme() {
        assert_close(
            hypergeometric_log_probability(20, 10, 10, 0),
            -12.126791314602455,
            1e-12,
        );
    }

    #[test]
    fn hypergeometric_cumulative_probability_matches_exact_combinatorics() {
        assert_close(
            hypergeometric_cumulative_probability(20, 10, 10, 5),
            0.6718591006516703,
            1e-12,
        );
    }

    #[test]
    fn hypergeometric_upper_cumulative_probability_is_inclusive_like_java() {
        assert_close(
            hypergeometric_upper_cumulative_probability(20, 10, 10, 5),
            0.6718591006516703,
            1e-12,
        );
        assert_close(
            hypergeometric_upper_cumulative_probability(20, 10, 10, 10),
            5.412544112234515e-6,
            1e-18,
        );
    }

    #[test]
    fn sanitize_log_probability_replaces_nan_with_zero() {
        assert_eq!(sanitize_log_probability(f64::NAN), 0.0);
    }

    #[test]
    fn zeroin_c_solves_square_root_of_two() {
        let root = zeroin_c(0.0, 2.0, |value| value * value - 2.0, 1e-10);
        assert_close(root, 2.0_f64.sqrt(), 1e-10);
    }

    #[test]
    fn zeroin_c_solves_linear_root_exactly() {
        let root = zeroin_c(0.0, 1.0, |value| value - 0.5, 1e-10);
        assert_close(root, 0.5, 1e-12);
    }

    #[test]
    fn zeroin_c_solves_cubic_root() {
        let root = zeroin_c(1.0, 2.0, |value| value.powi(3) - value - 2.0, 1e-10);
        assert_close(root, 1.5213797068045676, 1e-10);
    }

    #[test]
    fn round_as_r_rounds_to_five_places() {
        assert_eq!(round_as_r(0.123456789), 0.12346);
        assert_eq!(round_as_r(0.999999), 1.0);
        assert_eq!(round_as_r(0.000001), 0.0);
    }

    #[test]
    fn round_as_r_canonicalizes_zero_and_one() {
        assert_eq!(round_as_r(0.0), 0.0);
        assert_eq!(round_as_r(1.0), 1.0);
    }

    #[test]
    fn logdc_dhyper_matches_java_fixture_vector() {
        let fisher = FisherExact::new(11, 12, 1, 2);
        assert_eq!(
            fisher.logdc,
            vec![-2.4696392, -1.0345547, -0.8675006, -1.9661129]
        );
    }

    #[test]
    fn logdc_dhyper_all_zero_table_is_degenerate_zero() {
        let fisher = FisherExact::new(0, 0, 0, 0);
        assert_eq!(fisher.logdc, vec![0.0]);
    }

    #[test]
    fn dnhyper_normalizes_central_distribution() {
        let fisher = FisherExact::new(11, 12, 1, 2);
        let distribution = fisher.dnhyper(1.0);
        assert_close(distribution.iter().sum(), 1.0, 1e-12);
    }

    #[test]
    fn mnhyper_zero_non_centrality_returns_lower_support() {
        let fisher = FisherExact::new(10, 10, 10, 1);
        assert_eq!(fisher.mnhyper(0.0), f64::from(fisher.lo));
    }

    #[test]
    fn mnhyper_infinite_non_centrality_returns_upper_support() {
        let fisher = FisherExact::new(10, 10, 10, 1);
        assert_eq!(fisher.mnhyper(f64::INFINITY), f64::from(fisher.hi));
    }

    #[test]
    fn pnhyper_upper_tail_includes_observed_support_point() {
        let fisher = FisherExact::new(10, 0, 0, 5);
        assert_close(fisher.pnhyper(10, true), 1.0 / 3003.0, 1e-15);
    }

    #[test]
    fn getters_round_stored_values_to_five_decimals() {
        let mut fisher = FisherExact::new(1, 1, 1, 1);
        fisher.p_value_two_sided = 0.123456;
        fisher.p_value_less = 0.987654;
        fisher.p_value_greater = 0.111119;

        assert_eq!(fisher.get_p_value(), 0.12346);
        assert_eq!(fisher.get_p_value_less(), 0.98765);
        assert_eq!(fisher.get_p_value_greater(), 0.11112);
    }

    #[test]
    fn get_logdc_recomputes_after_mutation() {
        let mut fisher = FisherExact::new(11, 12, 1, 2);
        fisher.logdc = vec![42.0];
        assert_eq!(
            fisher.get_logdc(),
            &[-2.4696392, -1.0345547, -0.8675006, -1.9661129]
        );
    }

    macro_rules! fisher_fixture_test {
        ($name:ident, $ref_fwd:expr, $ref_rev:expr, $alt_fwd:expr, $alt_rev:expr, $p2:expr, $pless:expr, $pgreater:expr, $odd_ratio:expr) => {
            #[test]
            fn $name() {
                let fisher = FisherExact::new($ref_fwd, $ref_rev, $alt_fwd, $alt_rev);
                assert_close(fisher.get_p_value(), $p2, 1e-12);
                assert_close(fisher.get_p_value_less(), $pless, 1e-12);
                assert_close(fisher.get_p_value_greater(), $pgreater, 1e-12);
                assert_eq!(fisher.get_odd_ratio(), $odd_ratio);
            }
        };
    }

    fisher_fixture_test!(fixture_all_zero, 0, 0, 0, 0, 1.0, 1.0, 1.0, "0");
    fisher_fixture_test!(
        fixture_boundary_hi,
        10,
        0,
        0,
        5,
        0.00033,
        1.0,
        0.00033,
        "Inf"
    );
    fisher_fixture_test!(fixture_boundary_lo, 0, 10, 5, 0, 0.00033, 0.00033, 1.0, "0");
    fisher_fixture_test!(fixture_balanced, 10, 10, 10, 10, 1.0, 0.62381, 0.62381, "1");
    fisher_fixture_test!(
        fixture_strong_bias,
        100,
        5,
        3,
        50,
        0.0,
        1.0,
        0.0,
        "298.2426"
    );
    fisher_fixture_test!(fixture_single_count, 1, 0, 0, 0, 1.0, 1.0, 1.0, "0");
    fisher_fixture_test!(
        fixture_large_balanced,
        50,
        50,
        50,
        50,
        1.0,
        0.55621,
        0.55621,
        "1"
    );
    fisher_fixture_test!(
        fixture_minimal_two_by_two,
        1,
        1,
        1,
        1,
        1.0,
        0.83333,
        0.83333,
        "1"
    );
    fisher_fixture_test!(
        fixture_extreme_imbalance,
        200,
        3,
        1,
        150,
        0.0,
        1.0,
        0.0,
        "7049.2327"
    );
    fisher_fixture_test!(
        fixture_java_data_provider_balanced_bias,
        121,
        55,
        18,
        23,
        0.00378,
        0.99908,
        0.00287,
        "2.79657"
    );
    fisher_fixture_test!(
        fixture_java_data_provider_upper_tail,
        10,
        10,
        10,
        1,
        0.04722,
        0.02599,
        0.99802,
        "0.10703"
    );
    fisher_fixture_test!(
        fixture_java_data_provider_less_than_one_odds_ratio,
        37,
        76,
        1,
        1,
        1.0,
        0.55362,
        0.89275,
        "0.49015"
    );
    fisher_fixture_test!(
        fixture_java_data_provider_large_odds_ratio,
        69,
        1,
        74,
        95,
        0.0,
        1.0,
        0.0,
        "87.68597"
    );
}
