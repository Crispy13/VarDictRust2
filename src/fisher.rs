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

// ── Apache Commons Math3 3.6.1 SaddlePointExpansion (faithful port) ──
// Java FisherExact uses org.apache.commons.math3.distribution.HypergeometricDistribution
// .logProbability, which is computed by SaddlePointExpansion.logBinomialProbability — NOT a
// direct lgamma-based log binomial coefficient. The two methods differ at ~1e-12, which the
// 7-decimal rounding in `sanitize_log_probability` occasionally amplifies into a 5th-decimal
// p-value divergence. Porting the saddle-point method makes logProbability bit-identical to Java.

/// MathUtils.TWO_PI (= 2π as the nearest f64), matching commons-math3.
const TWO_PI: f64 = std::f64::consts::TAU;
/// 0.5 * log(2π); only used by get_stirling_error's non-half-integer branch (unreachable for
/// integer counts, so libm log vs FastMath.log here cannot affect parity).
const HALF_LOG_2_PI: f64 = 0.918_938_533_204_672_7;

/// SaddlePointExpansion.EXACT_STIRLING_ERRORS — exact Stirling error for z = 0, 0.5, … , 15.0.
const EXACT_STIRLING_ERRORS: [f64; 31] = [
    0.0,                           // 0.0
    0.153_426_409_720_027_345_291, // 0.5
    0.081_061_466_795_327_258_220, // 1.0
    0.054_814_121_051_917_653_896, // 1.5
    0.041_340_695_955_409_294_094, // 2.0
    0.033_162_873_519_936_287_485, // 2.5
    0.027_677_925_684_998_339_149, // 3.0
    0.023_746_163_656_297_495_971, // 3.5
    0.020_790_672_103_765_093_112, // 4.0
    0.018_488_450_532_673_185_231, // 4.5
    0.016_644_691_189_821_192_163, // 5.0
    0.015_134_973_221_917_378_874, // 5.5
    0.013_876_128_823_070_747_999, // 6.0
    0.012_810_465_242_920_226_924, // 6.5
    0.011_896_709_945_891_770_095, // 7.0
    0.011_104_559_758_206_917_327, // 7.5
    0.010_411_265_261_972_096_497, // 8.0
    0.009_799_416_126_158_803_298, // 8.5
    0.009_255_462_182_712_732_918, // 9.0
    0.008_768_700_134_139_385_463, // 9.5
    0.008_330_563_433_362_871_256, // 10.0
    0.007_934_114_564_314_020_547, // 10.5
    0.007_573_675_487_951_840_795, // 11.0
    0.007_244_554_301_320_383_180, // 11.5
    0.006_942_840_107_209_529_866, // 12.0
    0.006_665_247_032_707_682_442, // 12.5
    0.006_408_994_188_004_207_068, // 13.0
    0.006_171_712_263_039_457_648, // 13.5
    0.005_951_370_112_758_847_736, // 14.0
    0.005_746_216_513_010_115_682, // 14.5
    0.005_554_733_551_962_801_371, // 15.0
];

/// SaddlePointExpansion.getStirlingError(z).
fn get_stirling_error(z: f64) -> f64 {
    if z < 15.0 {
        let z2 = 2.0 * z;
        if z2.floor() == z2 {
            EXACT_STIRLING_ERRORS[z2 as usize]
        } else {
            ln_gamma(z + 1.0) - (z + 0.5) * z.ln() + z - HALF_LOG_2_PI
        }
    } else {
        let z2 = z * z;
        (0.083_333_333_333_333_333_333
            - (0.002_777_777_777_777_777_778
                - (0.000_793_650_793_650_793_651
                    - (0.000_595_238_095_238_095_238 - 0.000_841_750_841_750_841_751 / z2) / z2)
                    / z2)
                / z2)
            / z
    }
}

/// SaddlePointExpansion.getDeviancePart(x, mu).
fn get_deviance_part(x: f64, mu: f64) -> f64 {
    if (x - mu).abs() < 0.1 * (x + mu) {
        let d = x - mu;
        let mut v = d / (x + mu);
        let mut s1 = v * d;
        let mut s = f64::NAN;
        let mut ej = 2.0 * x * v;
        v *= v;
        let mut j = 1i32;
        while s1 != s {
            s = s1;
            ej *= v;
            s1 = s + ej / f64::from(j * 2 + 1);
            j += 1;
        }
        s1
    } else {
        x * (x / mu).ln() + mu - x
    }
}

/// SaddlePointExpansion.logBinomialProbability(x, n, p, q).
fn log_binomial_probability(x: i32, n: i32, p: f64, q: f64) -> f64 {
    if x == 0 {
        if p < 0.1 {
            -get_deviance_part(f64::from(n), f64::from(n) * q) - f64::from(n) * p
        } else {
            f64::from(n) * q.ln()
        }
    } else if x == n {
        if q < 0.1 {
            -get_deviance_part(f64::from(n), f64::from(n) * p) - f64::from(n) * q
        } else {
            f64::from(n) * p.ln()
        }
    } else {
        let ret = get_stirling_error(f64::from(n))
            - get_stirling_error(f64::from(x))
            - get_stirling_error(f64::from(n - x))
            - get_deviance_part(f64::from(x), f64::from(n) * p)
            - get_deviance_part(f64::from(n - x), f64::from(n) * q);
        let f = (TWO_PI * f64::from(x) * f64::from(n - x)) / f64::from(n);
        -0.5 * f.ln() + ret
    }
}

#[cfg(test)]
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

    // commons-math3 HypergeometricDistribution.logProbability(x):
    // populationSize = population, numberOfSuccesses = successes, sampleSize = draws.
    let p = f64::from(draws) / f64::from(population);
    let q = f64::from(population - draws) / f64::from(population);
    let p1 = log_binomial_probability(observed, successes, p, q);
    let p2 = log_binomial_probability(draws - observed, population - successes, p, q);
    let p3 = log_binomial_probability(draws, population, p, q);
    p1 + p2 - p3
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

/// Mirror Java `String.valueOf(double)` / `Double.toString(double)`.
///
/// Java uses plain decimal notation for `1e-3 <= |m| < 1e7` and "computerized scientific
/// notation" (e.g. `8.8E-4`, `5.0E-5`, `1.0E7`) outside that range. Rust's `f64::to_string`
/// only ever emits decimal, so small odds ratios like `0.00088` printed as `0.00088` instead
/// of Java's `8.8E-4`. Both languages emit the shortest round-tripping digit sequence, so we
/// reuse Rust's formatting for the digits and only reshape decimal vs scientific to match Java.
fn java_double_string(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let abs = value.abs();
    if abs != 0.0 && (abs < 1e-3 || abs >= 1e7) {
        // Java computerized scientific notation.
        let sci = format!("{value:e}"); // shortest round-trip, e.g. "8.8e-4" or "5e-5"
        let (mantissa, exponent) = sci.split_once('e').expect("LowerExp always contains 'e'");
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        format!("{mantissa}E{exponent}")
    } else if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

// Java: DoubleStream.sum() uses Collectors.sumWithCompensation().
fn java_double_stream_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut high_order_sum = 0.0;
    let mut low_order_sum = 0.0;
    let mut simple_sum = 0.0;

    for value in values {
        simple_sum += value;
        let adjusted = value - low_order_sum;
        let next_sum = high_order_sum + adjusted;
        low_order_sum = (next_sum - high_order_sum) - adjusted;
        high_order_sum = next_sum;
    }

    let compensated = high_order_sum + low_order_sum;
    if compensated.is_nan() && simple_sum.is_infinite() {
        simple_sum
    } else {
        compensated
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

        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }

        let tolerance_active = 2.0 * epsilon * b.abs() + tolerance / 2.0;
        let mut new_step = (c - b) / 2.0;

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
        let distribution = self.dnhyper(non_centrality_parameter);
        java_double_stream_sum(
            self.support
                .iter()
                .zip(distribution)
                .map(|(support_value, probability)| f64::from(*support_value) * probability),
        )
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
        let sum = java_double_stream_sum(exponentiated.iter().copied());

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
            java_double_string(round_as_r(odd_ratio))
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

    #[test]
    fn fixture_hg002_cm_fisher_integerish_odds_ratio_formats_like_java() {
        let fisher = FisherExact::new(8, 1, 1, 1);
        assert_eq!(fisher.get_odd_ratio(), "6.0");
    }

    #[test]
    fn fixture_hg002_cm_fisher_chr14_odds_ratio_rounds_like_java() {
        let fisher = FisherExact::new(10, 1, 8, 3);
        assert_eq!(fisher.get_odd_ratio(), "3.53775");
    }

    #[test]
    fn fixture_hg002_cm_fisher_chr12_odds_ratio_rounds_like_java() {
        let fisher = FisherExact::new(6, 4, 1, 3);
        assert_eq!(fisher.get_odd_ratio(), "4.03195");
    }

    mod pbt {
        use super::*;
        use proptest::prelude::*;
        use proptest::test_runner::Config as ProptestConfig;

        fn arb_unit_interval() -> impl Strategy<Value = f64> {
            prop_oneof![
                0.0f64..=1.0f64,
                Just(0.0),
                Just(1.0),
                Just(0.000005),
                Just(0.999995),
                Just(0.0000049),
                Just(0.9999951),
                Just(0.5),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 256,
                ..ProptestConfig::default()
            })]

            #[test]
            fn pbt_fisher_p_values_in_unit_interval(
                ref_fwd in 0..100i32,
                ref_rev in 0..100i32,
                alt_fwd in 0..100i32,
                alt_rev in 0..100i32,
            ) {
                let fisher = FisherExact::new(ref_fwd, ref_rev, alt_fwd, alt_rev);

                prop_assert!((0.0..=1.0).contains(&fisher.get_p_value()));
                prop_assert!((0.0..=1.0).contains(&fisher.get_p_value_less()));
                prop_assert!((0.0..=1.0).contains(&fisher.get_p_value_greater()));
            }

            #[test]
            fn pbt_fisher_p_less_plus_p_greater_ge_p_two_sided(
                ref_fwd in 0..100i32,
                ref_rev in 0..100i32,
                alt_fwd in 0..100i32,
                alt_rev in 0..100i32,
            ) {
                let fisher = FisherExact::new(ref_fwd, ref_rev, alt_fwd, alt_rev);
                let p_less = fisher.get_p_value_less();
                let p_greater = fisher.get_p_value_greater();
                let p_two_sided = fisher.get_p_value();

                prop_assert!(
                    p_less + p_greater >= p_two_sided - 1e-4,
                    "p_less ({p_less}) + p_greater ({p_greater}) < p_two_sided ({p_two_sided})"
                );
            }

            #[test]
            fn pbt_fisher_row_swap_symmetry(
                ref_fwd in 0..100i32,
                ref_rev in 0..100i32,
                alt_fwd in 0..100i32,
                alt_rev in 0..100i32,
            ) {
                let fisher = FisherExact::new(ref_fwd, ref_rev, alt_fwd, alt_rev);
                let swapped = FisherExact::new(alt_fwd, alt_rev, ref_fwd, ref_rev);

                prop_assert_eq!(fisher.get_p_value(), swapped.get_p_value());
            }

            #[test]
            fn pbt_log_binom_neg_infinity_for_invalid_k(
                n in 0..200i32,
                k in -1..201i32,
            ) {
                prop_assume!(k < 0 || k > n);

                prop_assert_eq!(log_binomial_coefficient(n, k), f64::NEG_INFINITY);
            }

            #[test]
            fn pbt_log_binom_zero_for_boundary_k(n in 0..200i32) {
                prop_assert!((log_binomial_coefficient(n, 0) - 0.0).abs() <= 1e-12);
                prop_assert!((log_binomial_coefficient(n, n) - 0.0).abs() <= 1e-12);
            }

            #[test]
            fn pbt_round_as_r_preserves_unit_interval(value in arb_unit_interval()) {
                let rounded = round_as_r(value);

                prop_assert!((0.0..=1.0).contains(&rounded));
            }
        }
    }
}
